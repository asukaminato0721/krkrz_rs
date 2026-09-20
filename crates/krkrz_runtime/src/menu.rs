//! MenuItem model, following krkrz/menu (W.Dee and contributors, Kirikiri license).
//! Native presentation and popup tracking are separate from the shared model.
use crate::Services;
use anyhow::{Context, Result, bail, ensure};
use krkrz_assets::media::Image;
use krkrz_tjs::{ObjectRef, Value, Vm, unsupported};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub enum MenuBitmap {
    #[default]
    None,
    System(i32),
    Image(Arc<Image>),
}
impl MenuBitmap {
    fn value(&self) -> i64 {
        match self {
            Self::None => 0,
            Self::System(id) => i64::from(*id),
            Self::Image(_) => -1,
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Image(image) => image.rgba.len(),
            _ => 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct MenuAppearance {
    pub right_justify: bool,
    /// Item, checked, unchecked bitmaps, captured when the script sets them.
    pub bitmaps: [MenuBitmap; 3],
}

pub(crate) struct Item {
    constructed: bool,
    action_owner: Value,
    window: Value,
    caption: Value,
    checked: bool,
    enabled: bool,
    visible: bool,
    radio: bool,
    group: i32,
    shortcut: i32,
    parent: Option<usize>,
    display_parent: Option<usize>,
    children: Vec<usize>,
    display_children: Vec<usize>,
    array: Option<Value>,
    array_dirty: bool,
    right_justify: Option<bool>,
    bitmaps: [MenuBitmap; 3],
}
impl Default for Item {
    fn default() -> Self {
        Self {
            constructed: false,
            action_owner: Value::NULL,
            window: Value::NULL,
            caption: Value::string(""),
            checked: false,
            enabled: true,
            visible: true,
            radio: false,
            group: -1,
            shortcut: 0,
            parent: None,
            display_parent: None,
            children: Vec::new(),
            display_children: Vec::new(),
            array: None,
            array_dirty: true,
            right_justify: None,
            bitmaps: Default::default(),
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    pub(crate) links: BTreeSet<String>,
    items: BTreeMap<usize, Item>,
    roots: BTreeMap<usize, Value>,
    root_class: Option<Value>,
    text_to_key: Option<Value>,
    key_to_text: Option<Value>,
    pub(crate) pending: VecDeque<usize>,
}
fn id(value: &Value) -> Result<usize> {
    let Value::Object(r) = value else {
        bail!("MenuItem requires an object context");
    };
    r.object.context("MenuItem requires a non-null context")
}
fn bound(id: usize) -> Value {
    Value::Object(ObjectRef {
        object: Some(id),
        context: Some(id),
    })
}
fn charge(budget: &mut u64) -> Result<()> {
    *budget = budget
        .checked_sub(1)
        .ok_or_else(|| unsupported("MenuItem execution budget exceeded"))?;
    Ok(())
}
fn class(vm: &mut Vm) -> Result<Value> {
    let class = vm.new_native_class("MenuItem", "MenuItem.@initialize")?;
    for method in [
        "MenuItem",
        "finalize",
        "add",
        "insert",
        "remove",
        "popup",
        "onClick",
        "fireClick",
    ] {
        vm.register_native_method(&class, method, &format!("MenuItem.{method}"))?;
    }
    for key in [
        "caption", "checked", "enabled", "group", "radio", "shortcut", "visible", "parent",
        "children", "root", "window", "index", "HMENU",
    ] {
        let writable = matches!(
            key,
            "caption"
                | "checked"
                | "enabled"
                | "group"
                | "radio"
                | "shortcut"
                | "visible"
                | "index"
        );
        vm.register_native_property(
            &class,
            key,
            Some(&format!("MenuItem.get:{key}")),
            writable.then(|| format!("MenuItem.set:{key}")).as_deref(),
        )?;
    }
    for key in ["textToKeycode", "keycodeToText"] {
        vm.register_native_static_property(
            &class,
            key,
            Some(&format!("MenuItem.get:{key}")),
            None,
        )?;
    }
    Ok(class)
}
pub(crate) fn register(vm: &mut Vm) -> Result<State> {
    let root_class = class(vm)?;
    let public_class = class(vm)?;
    vm.globals.insert("MenuItem".into(), public_class);
    let window = vm
        .globals
        .get("Window")
        .context("Window class is missing")?
        .clone();
    vm.register_native_property(&window, "menu", Some("Window.get:menu"), None)?;
    let text_to_key = vm.new_dictionary()?;
    let key_to_text = vm.new_native_array(vec![Value::Void; 256])?;
    // Stable English headless defaults. The platform host can replace the
    // table contents for its keyboard layout before constructing menu items.
    let mut names = vec![
        (8, "Backspace".into()),
        (9, "Tab".into()),
        (12, "Num 5".into()),
        (13, "Enter".into()),
        (16, "Shift".into()),
        (17, "Ctrl".into()),
        (18, "Alt".into()),
        (19, "Pause".into()),
        (20, "Caps Lock".into()),
        (27, "Esc".into()),
        (32, "Space".into()),
        (33, "Page Up".into()),
        (34, "Page Down".into()),
        (35, "End".into()),
        (36, "Home".into()),
        (37, "Left".into()),
        (38, "Up".into()),
        (39, "Right".into()),
        (40, "Down".into()),
        (44, "Print Screen".into()),
        (45, "Insert".into()),
        (46, "Delete".into()),
        (106, "Num *".into()),
        (107, "Num +".into()),
        (109, "Num -".into()),
        (110, "Num Del".into()),
        (111, "Num /".into()),
        (144, "Num Lock".into()),
        (145, "Scroll Lock".into()),
    ];
    for key in (48..=57).chain(65..=90) {
        names.push((key, char::from_u32(key as u32).unwrap().to_string()));
    }
    for key in 96..=105 {
        names.push((key, format!("Num {}", key - 96)));
    }
    for key in 112..=135 {
        names.push((key, format!("F{}", key - 111)));
    }
    for (key, name) in names {
        vm.set_member(&key_to_text, &Value::Integer(key), Value::string(&name))?;
        vm.set_member(
            &text_to_key,
            &Value::string(&name.to_lowercase()),
            Value::Integer(key),
        )?;
    }
    for (alias, key) in [("bksp", 8), ("pgup", 33), ("pgdn", 34), ("del", 46)] {
        vm.set_member(&text_to_key, &Value::string(alias), Value::Integer(key))?;
    }
    Ok(State {
        root_class: Some(root_class),
        text_to_key: Some(text_to_key),
        key_to_text: Some(key_to_text),
        ..State::default()
    })
}
impl State {
    pub(crate) fn relink(&mut self, vm: &mut Vm, spelling: &str) -> Result<()> {
        if self.links.contains(spelling) {
            return Ok(());
        }
        let new = register(vm)?;
        self.root_class = new.root_class;
        self.text_to_key = new.text_to_key;
        self.key_to_text = new.key_to_text;
        self.links.insert(spelling.into());
        Ok(())
    }
    pub(crate) fn is_live(&self, id: usize) -> bool {
        self.items.contains_key(&id)
    }
    fn item(&self, id: usize) -> Result<&Item> {
        let item = self
            .items
            .get(&id)
            .context("context has no MenuItem native instance")?;
        ensure!(item.constructed, "MenuItem constructor has not run");
        Ok(item)
    }
    fn index(&self, id: usize) -> i64 {
        self.items
            .get(&id)
            .and_then(|i| i.display_parent)
            .and_then(|p| self.items.get(&p))
            .and_then(|p| p.display_children.iter().position(|c| *c == id))
            .map_or(-1, |i| i as i64)
    }
    fn root(&self, mut id: usize, budget: &mut u64) -> Result<usize> {
        while let Some(parent) = self.item(id)?.parent {
            charge(budget)?;
            id = parent;
        }
        Ok(id)
    }
    fn attach(
        &mut self,
        parent: usize,
        child: usize,
        index: Option<i32>,
        budget: &mut u64,
    ) -> Result<()> {
        self.item(parent)?;
        self.item(child)?;
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            charge(budget)?;
            ensure!(id != child, "MenuItem hierarchy cannot contain a cycle");
            ancestor = self.item(id)?.display_parent;
        }
        let old_parent = self.items[&child].display_parent;
        let remaining =
            self.items[&parent].display_children.len() - usize::from(old_parent == Some(parent));
        let index = index.unwrap_or(remaining as i32).max(0) as usize;
        ensure!(index <= remaining, "MenuItem insertion index out of range");
        if let Some(old) = old_parent {
            self.items
                .get_mut(&old)
                .unwrap()
                .display_children
                .retain(|id| *id != child);
        }
        let item = self.items.get_mut(&child).unwrap();
        item.display_parent = Some(parent);
        let parent_item = self.items.get_mut(&parent).unwrap();
        parent_item.display_children.insert(index, child);
        // Upstream keeps a separate insertion-order native object list. Moving
        // an existing item does not remove it from the old parent's object list.
        if !parent_item.children.contains(&child) {
            parent_item.children.push(child);
            parent_item.array_dirty = true;
            self.items.get_mut(&child).unwrap().parent = Some(parent);
        }
        if self.items[&child].checked && self.items[&child].radio {
            self.check_radio(child, budget)?;
        }
        Ok(())
    }
    fn check_radio(&mut self, id: usize, budget: &mut u64) -> Result<()> {
        let item = &self.items[&id];
        let (parent, group) = (item.display_parent, item.group);
        if let Some(parent) = parent {
            let siblings = self.items[&parent].display_children.clone();
            for sibling in siblings {
                charge(budget)?;
                let item = self.items.get_mut(&sibling).unwrap();
                if item.group == group && item.radio {
                    item.checked = false;
                }
            }
        }
        self.items.get_mut(&id).unwrap().checked = true;
        Ok(())
    }
    fn reconsider_radio(&mut self, id: usize, budget: &mut u64) -> Result<()> {
        let item = &self.items[&id];
        if item.radio
            && item.checked
            && let Some(parent) = item.display_parent
        {
            // The source's search includes this item, so setting the same group
            // also clears its check mark.
            let group = item.group;
            let siblings = self.items[&parent].display_children.clone();
            for sibling in siblings {
                charge(budget)?;
                let peer = &self.items[&sibling];
                if peer.radio && peer.checked && peer.group == group {
                    self.items.get_mut(&id).unwrap().checked = false;
                    break;
                }
            }
        }
        Ok(())
    }
}
impl Services {
    pub(crate) fn menu_appearance_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let id = id(context)?;
        let (access, property) = operation
            .split_once(':')
            .ok_or_else(|| unsupported(format!("windowEx MenuItem operation: {operation}")))?;
        ensure!(
            matches!(access, "get" | "set")
                && matches!(
                    property,
                    "rightJustify" | "bmpItem" | "bmpChecked" | "bmpUnchecked"
                ),
            unsupported(format!("windowEx MenuItem operation: {operation}"))
        );
        // windowEx creates its extension on first access. An unattached item
        // cannot initialize it; detaching an initialized item retains its state.
        if self.menus.item(id)?.right_justify.is_none() {
            self.menu_appearance_parent(vm, context, budget)?;
            self.menus
                .items
                .get_mut(&id)
                .context("MenuItem invalidated during extension initialization")?
                .right_justify = Some(false);
        }
        let bitmap = match property {
            "bmpItem" => 0,
            "bmpChecked" => 1,
            _ => 2,
        };
        if access == "get" {
            let item = self.menus.item(id)?;
            return Ok(Value::Integer(if property == "rightJustify" {
                item.right_justify.unwrap().into()
            } else {
                item.bitmaps[bitmap].value()
            }));
        }
        let value = args
            .first()
            .context("MenuItem appearance setter requires a value")?;
        if property == "rightJustify" {
            self.menus.items.get_mut(&id).unwrap().right_justify = Some(value.integer()? != 0);
        } else {
            self.menus.items.get_mut(&id).unwrap().bitmaps[bitmap] = MenuBitmap::None;
            let value = match value {
                Value::Void | Value::Integer(_) | Value::String(_) => {
                    MenuBitmap::System(value.integer()? as i32)
                }
                Value::Object(reference) => {
                    let layer = reference
                        .object
                        .and_then(|id| self.layers.get(&id))
                        .context("no layer object.")?;
                    let image = layer.image.as_ref().context("layer has no image")?;
                    let used: usize = self
                        .menus
                        .items
                        .values()
                        .flat_map(|item| &item.bitmaps)
                        .map(MenuBitmap::bytes)
                        .sum();
                    ensure!(
                        image.rgba.len() <= (64usize * 1024 * 1024).saturating_sub(used),
                        "menu bitmap memory limit exceeded"
                    );
                    *budget = budget
                        .checked_sub((image.rgba.len() / 4) as u64)
                        .ok_or_else(|| unsupported("menu bitmap copy execution budget exceeded"))?;
                    let mut image = image.clone();
                    for pixel in image.rgba.as_chunks_mut::<4>().0.iter_mut() {
                        pixel[3] = if pixel[3] >= 64 { 255 } else { 0 };
                    }
                    MenuBitmap::Image(Arc::new(image))
                }
                _ => MenuBitmap::None,
            };
            self.menus.items.get_mut(&id).unwrap().bitmaps[bitmap] = value;
        }
        // The plugin stores the value before attempting the native menu update.
        self.menu_appearance_parent(vm, context, budget)?;
        Ok(Value::Void)
    }
    fn menu_appearance_parent(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        budget: &mut u64,
    ) -> Result<()> {
        let parent = vm.get_property(
            context,
            &Value::string("parent"),
            false,
            false,
            self,
            budget,
        )?;
        let parent = id(&parent).context("Cannot get parent menu.")?;
        self.menus.item(parent).context("Cannot get parent menu.")?;
        self.menus.item(id(context)?)?;
        Ok(())
    }
    pub(crate) fn window_menu(
        &mut self,
        vm: &mut Vm,
        window: &Value,
        budget: &mut u64,
    ) -> Result<Value> {
        let id = id(window)?;
        ensure!(
            self.windows.get(&id).is_some_and(|w| w.constructed),
            "Window constructor has not run"
        );
        if let Some(menu) = self.menus.roots.get(&id) {
            return Ok(menu.clone());
        }
        let class = self
            .menus
            .root_class
            .clone()
            .context("menu plugin is not loaded")?;
        let menu = vm.construct(&class, &[window.clone(), window.clone()], self, budget)?;
        self.menus.roots.insert(id, menu.clone());
        Ok(menu)
    }
    fn menu_shortcut_text(&mut self, vm: &mut Vm, key: i32, budget: &mut u64) -> Result<Value> {
        let mut text = String::new();
        for (flag, name) in [(4, "Shift+"), (8, "Ctrl+"), (16, "Alt+")] {
            if (key >> 16) & flag != 0 {
                text.push_str(name);
            }
        }
        let key = key & 0xffff;
        if (8..=255).contains(&key) {
            let table = self.menus.key_to_text.clone().unwrap();
            let name = vm.get_property(
                &table,
                &Value::Integer(key as i64),
                true,
                false,
                self,
                budget,
            )?;
            text.push_str(&name.text());
        }
        Ok(Value::string(&text))
    }
    pub(crate) fn menu_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        if operation == "get:textToKeycode" {
            return Ok(self.menus.text_to_key.clone().unwrap());
        }
        if operation == "get:keycodeToText" {
            return Ok(self.menus.key_to_text.clone().unwrap());
        }
        let id = id(context)?;
        if operation == "@initialize" {
            self.menus.items.entry(id).or_default();
            return Ok(Value::Void);
        }
        if operation == "@invalidate" {
            let children = self
                .menus
                .items
                .get(&id)
                .map(|i| i.children.clone())
                .unwrap_or_default();
            self.menus.pending.retain(|item| *item != id);
            for child in children {
                charge(budget)?;
                if self
                    .menus
                    .items
                    .get(&id)
                    .is_some_and(|i| i.children.contains(&child))
                {
                    vm.invalidate(&Value::object(child), self, budget)?;
                }
            }
            if let Some(item) = self.menus.items.remove(&id)
                && let Some(parent) = item.display_parent
                && let Some(parent) = self.menus.items.get_mut(&parent)
            {
                parent.display_children.retain(|child| *child != id);
            }
            return Ok(Value::Void);
        }
        let arg = |n| {
            args.get(n)
                .with_context(|| format!("MenuItem.{operation}: missing argument {n}"))
        };
        if operation == "MenuItem" {
            ensure!(
                self.menus.items.contains_key(&id),
                "context has no MenuItem native instance"
            );
            let owner = arg(0)?.clone();
            let root = args
                .get(1)
                .filter(|v| matches!(v, Value::Object(_)))
                .cloned();
            let window = root.as_ref().unwrap_or(&owner);
            let window_id = crate::menu::id(window)?;
            ensure!(
                self.windows.get(&window_id).is_some_and(|w| w.constructed),
                "MenuItem requires a constructed Window"
            );
            let caption = if root.is_none() {
                args.get(1)
                    .cloned()
                    .unwrap_or(Value::Void)
                    .unary("string")?
            } else {
                Value::string("")
            };
            let previous = self.menus.items.remove(&id).unwrap();
            // Re-running the constructor replaces the platform item but retains
            // the native object's parent, child list and published array.
            if let Some(parent) = previous.display_parent {
                self.menus
                    .items
                    .get_mut(&parent)
                    .unwrap()
                    .display_children
                    .retain(|child| *child != id);
            }
            for child in &previous.display_children {
                if let Some(child) = self.menus.items.get_mut(child) {
                    child.display_parent = None;
                }
            }
            self.menus.items.insert(
                id,
                Item {
                    constructed: true,
                    action_owner: owner,
                    window: if root.is_some() {
                        bound(window_id)
                    } else {
                        Value::NULL
                    },
                    caption,
                    parent: previous.parent,
                    children: previous.children,
                    array: previous.array,
                    array_dirty: previous.array_dirty,
                    right_justify: previous.right_justify,
                    bitmaps: previous.bitmaps,
                    ..Item::default()
                },
            );
            return Ok(Value::Void);
        }
        let item = self.menus.item(id)?;
        match operation {
            "finalize" => return Ok(Value::Void),
            "get:caption" => return Ok(item.caption.clone()),
            "get:checked" => return Ok(Value::Integer(item.checked.into())),
            "get:enabled" => return Ok(Value::Integer(item.enabled.into())),
            "get:visible" => return Ok(Value::Integer(item.visible.into())),
            "get:radio" => return Ok(Value::Integer(item.radio.into())),
            "get:group" => return Ok(Value::Integer(item.group.into())),
            "get:index" => return Ok(Value::Integer(self.menus.index(id))),
            "get:parent" => return Ok(item.parent.map_or(Value::NULL, bound)),
            "get:window" => return Ok(item.window.clone()),
            "get:root" => return Ok(bound(self.menus.root(id, budget)?)),
            "get:shortcut" => return self.menu_shortcut_text(vm, item.shortcut, budget),
            "get:children" => {
                let array = match item.array.clone() {
                    Some(a) => a,
                    None => vm.new_native_array(vec![])?,
                };
                if item.array_dirty {
                    let children = item.children.clone();
                    // Clear native elements without invoking a script replacement
                    // for Array.clear on the published array.
                    vm.set_member(&array, &Value::string("count"), Value::Integer(0))?;
                    for (index, child) in children.iter().enumerate() {
                        charge(budget)?;
                        vm.set_member(&array, &Value::Integer(index as i64), bound(*child))?;
                    }
                }
                let item = self.menus.items.get_mut(&id).unwrap();
                item.array = Some(array.clone());
                item.array_dirty = false;
                return Ok(array);
            }
            "set:caption" => {
                self.menus.items.get_mut(&id).unwrap().caption = arg(0)?.unary("string")?
            }
            "set:enabled" => self.menus.items.get_mut(&id).unwrap().enabled = arg(0)?.truth()?,
            "set:visible" => self.menus.items.get_mut(&id).unwrap().visible = arg(0)?.truth()?,
            "set:checked" => {
                let checked = arg(0)?.truth()?;
                if checked && item.radio {
                    self.menus.check_radio(id, budget)?;
                } else {
                    self.menus.items.get_mut(&id).unwrap().checked = checked;
                }
            }
            "set:radio" => {
                let radio = arg(0)?.truth()?;
                if radio != item.radio {
                    self.menus.items.get_mut(&id).unwrap().radio = radio;
                    self.menus.reconsider_radio(id, budget)?;
                }
            }
            "set:group" => {
                self.menus.items.get_mut(&id).unwrap().group = arg(0)?.integer()? as i32;
                self.menus.reconsider_radio(id, budget)?;
            }
            "set:shortcut" => {
                let text = arg(0)?
                    .text()
                    .split('\0')
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                let mut flags = 0;
                let mut tail = 0;
                for (prefix, flag) in [("shift+", 4), ("ctrl+", 8), ("alt+", 16)] {
                    if let Some(pos) = text.find(prefix) {
                        flags |= flag;
                        tail = tail.max(pos + prefix.len());
                    }
                }
                let table = self.menus.text_to_key.clone().unwrap();
                let value = vm.get_property(
                    &table,
                    &Value::string(&text[tail..]),
                    true,
                    false,
                    self,
                    budget,
                )?;
                let code = if matches!(value, Value::Void) {
                    0
                } else {
                    flags |= 1;
                    value.integer()? as i32 & 0xffff
                };
                self.menus
                    .items
                    .get_mut(&id)
                    .context("MenuItem was invalidated during shortcut conversion")?
                    .shortcut = (flags << 16) | code;
            }
            "add" | "insert" => {
                let child = crate::menu::id(arg(0)?)?;
                let index = if operation == "insert" {
                    Some(arg(1)?.integer()? as i32)
                } else {
                    None
                };
                self.menus.attach(id, child, index, budget)?;
            }
            "remove" => {
                let child = crate::menu::id(arg(0)?)?;
                self.menus.item(child)?;
                ensure!(
                    item.display_children.contains(&child),
                    "MenuItem is not a child of this menu"
                );
                let item = self.menus.items.get_mut(&id).unwrap();
                item.display_children.retain(|i| *i != child);
                if item.children.contains(&child) {
                    item.children.retain(|i| *i != child);
                    item.array_dirty = true;
                    self.menus.items.get_mut(&child).unwrap().parent = None;
                }
                self.menus.items.get_mut(&child).unwrap().display_parent = None;
            }
            "set:index" => {
                let index = arg(0)?.integer()? as i32;
                if self.menus.index(id) != index as i64 {
                    let parent = item
                        .display_parent
                        .context("cannot reorder a detached MenuItem")?;
                    // Reordering affects only the display list.
                    let count = self.menus.items[&parent].display_children.len();
                    let index = index.max(0) as usize;
                    ensure!(index < count, "MenuItem index out of range");
                    let list = &mut self.menus.items.get_mut(&parent).unwrap().display_children;
                    list.retain(|child| *child != id);
                    list.insert(index, id);
                    if self.menus.items[&id].radio && self.menus.items[&id].checked {
                        self.menus.check_radio(id, budget)?;
                    }
                }
            }
            "onClick" => {
                let owner = item.action_owner.clone();
                if owner != Value::NULL {
                    let action = vm.get_property(
                        &owner,
                        &Value::string("action"),
                        true,
                        false,
                        self,
                        budget,
                    )?;
                    if !matches!(action, Value::Void) {
                        let event = vm.new_dictionary()?;
                        vm.set_member(&event, &Value::string("type"), Value::string("onClick"))?;
                        vm.set_member(&event, &Value::string("target"), bound(id))?;
                        return vm.call_function(&action, &owner, &[event], self, budget);
                    }
                }
            }
            "fireClick" => {
                let mut ancestor = Some(id);
                while let Some(current) = ancestor {
                    charge(budget)?;
                    let menu = self.menus.item(current)?;
                    if !menu.enabled {
                        return Ok(Value::Void);
                    }
                    ancestor = menu.display_parent;
                }
                let root = self.menus.root(id, budget)?;
                if self.menus.items[&root].window != Value::NULL {
                    self.menus.pending.push_back(id);
                }
            }
            _ => {
                return Err(unsupported(format!(
                    "MenuItem.{operation}: native menu presentation is not implemented"
                )));
            }
        }
        Ok(Value::Void)
    }
}

impl crate::Session {
    /// Menu appearance for platform presentation; this does not initialize the
    /// windowEx extension or execute script getters.
    pub fn menu_appearance(&self, menu: &Value) -> Result<MenuAppearance> {
        let item = self.services.menus.item(id(menu)?)?;
        Ok(MenuAppearance {
            right_justify: item.right_justify.unwrap_or(false),
            bitmaps: item.bitmaps.clone(),
        })
    }
}

impl State {
    pub(crate) fn gc_roots(&self, out: &mut Vec<Value>) {
        out.extend(self.root_class.iter().cloned());
        out.extend(self.text_to_key.iter().cloned());
        out.extend(self.key_to_text.iter().cloned());
        out.extend(self.pending.iter().copied().map(Value::object));
    }
    pub(crate) fn gc_trace(&self, id: usize, out: &mut Vec<Value>) {
        if let Some(root) = self.roots.get(&id) {
            out.push(root.clone());
        }
        if let Some(item) = self.items.get(&id) {
            out.extend([
                item.action_owner.clone(),
                item.window.clone(),
                item.caption.clone(),
            ]);
            out.extend(
                [item.parent, item.display_parent]
                    .into_iter()
                    .flatten()
                    .map(Value::object),
            );
            out.extend(
                item.children
                    .iter()
                    .chain(&item.display_children)
                    .copied()
                    .map(Value::object),
            );
            out.extend(item.array.iter().cloned());
        }
    }
}
