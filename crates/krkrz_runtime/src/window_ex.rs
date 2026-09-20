//! Portable windowEx state, adapted from miahmie's windowEx (Kirikiri license).
//! Native window management remains a host operation, with explicit errors.
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm, unsupported};
use serde::Deserialize;
use std::collections::BTreeMap;

/// A snapshot of a script-defined system menu, shared by native and replay hosts.
#[derive(Clone, Debug)]
pub struct SystemMenuItem {
    pub command: Option<u16>,
    pub caption: String,
    pub checked: bool,
    pub radio: bool,
    pub enabled: bool,
    pub menu_break: i32,
    pub insert_position: Option<i32>,
    pub insert_command: Option<i32>,
    pub children: Vec<SystemMenuItem>,
    source: Value,
}

impl crate::Session {
    pub fn system_menu(&self, window: &Value) -> Result<&[SystemMenuItem]> {
        Ok(&self
            .services
            .window_ex
            .windows
            .get(&id(window)?)
            .context("Window extension is not registered")?
            .system_menu)
    }

    /// Deliver a host selection using the same command IDs as windowEx.
    pub fn select_system_menu(&mut self, window: &Value, command: u16) -> Result<Value> {
        fn find(items: &[SystemMenuItem], command: u16) -> Option<Value> {
            for item in items {
                if !item.enabled {
                    continue;
                }
                if item.command == Some(command) {
                    return Some(item.source.clone());
                }
                if let Some(value) = find(&item.children, command) {
                    return Some(value);
                }
            }
            None
        }
        let source = find(self.system_menu(window)?, command)
            .context("system menu command is absent or disabled")?;
        let callback = self.vm.get_property(
            window,
            &Value::string("onExSystemMenuSelected"),
            true,
            false,
            &mut self.services,
            &mut self.budget,
        )?;
        if matches!(callback, Value::Void) {
            return Ok(Value::Void);
        }
        self.vm.call_function(
            &callback,
            window,
            &[source],
            &mut self.services,
            &mut self.budget,
        )
    }
}

#[derive(Deserialize)]
struct Interface {
    exports: BTreeMap<String, Exports>,
    notifications: Vec<(String, i32, String)>,
}
#[derive(Deserialize)]
struct Exports {
    constants: BTreeMap<String, i64>,
    methods: Vec<String>,
    properties: Vec<String>,
}
fn interface() -> Result<Interface> {
    Ok(serde_json::from_str(include_str!(
        "../data/window_ex.json"
    ))?)
}
#[derive(Default)]
pub(crate) struct Window {
    disable_resize: bool,
    disable_move: bool,
    nc_mouse: bool,
    events: [bool; 4],
    hooks: [u32; 32],
    system_menu_source: Option<Value>,
    system_menu: Vec<SystemMenuItem>,
}
pub(crate) struct State {
    spelling: Option<String>,
    pub(crate) windows: BTreeMap<usize, Window>,
    eval_error_log: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            spelling: None,
            windows: BTreeMap::new(),
            eval_error_log: true,
        }
    }
}
impl State {
    pub(crate) fn link(&mut self, vm: &mut Vm, spelling: &str) -> Result<()> {
        if let Some(old) = &self.spelling {
            ensure!(old == spelling, "Already registerd class:");
            return Ok(());
        }
        // The installed version needs the game's k2compat Pad/console objects.
        // Resolve every receiver before changing registrations.
        let api = interface()?;
        let mut receivers = BTreeMap::new();
        for namespace in api.exports.keys() {
            let mut parts = namespace.split('.');
            let mut object = vm
                .globals
                .get(parts.next().unwrap())
                .cloned()
                .context("Cannot convert the variable type ((void) to Object)")?;
            for part in parts {
                object = vm.get_member(&object, &Value::string(part), false)?;
            }
            ensure!(
                matches!(object, Value::Object(r) if r.object.is_some()),
                "Cannot convert the variable type ((void) to Object)"
            );
            receivers.insert(namespace.clone(), object);
        }
        for (namespace, exports) in api.exports {
            let receiver = &receivers[&namespace];
            for (name, value) in exports.constants {
                vm.set_member(receiver, &Value::string(&name), Value::Integer(value))?;
            }
            for name in exports.methods {
                if namespace == "Scripts" && name == "eval" {
                    continue;
                }
                vm.register_native_method(
                    receiver,
                    &name,
                    &format!("WindowEx.{namespace}.{name}"),
                )?;
            }
            for name in exports.properties {
                vm.register_native_property(
                    receiver,
                    &name,
                    Some(&format!("WindowEx.{namespace}.get:{name}")),
                    Some(&format!("WindowEx.{namespace}.set:{name}")),
                )?;
            }
        }
        self.spelling = Some(spelling.into());
        Ok(())
    }
}
fn id(value: &Value) -> Result<usize> {
    if let Value::Object(r) = value {
        return r.object.context("WindowEx requires a non-null context");
    }
    anyhow::bail!("WindowEx requires an object context")
}
fn charge(budget: &mut u64) -> Result<()> {
    *budget = budget
        .checked_sub(1)
        .ok_or_else(|| unsupported("windowEx execution budget exceeded"))?;
    Ok(())
}
impl Services {
    fn build_system_menu(
        &mut self,
        vm: &mut Vm,
        list: &Value,
        command: &mut u16,
        depth: usize,
        budget: &mut u64,
    ) -> Result<Vec<SystemMenuItem>> {
        if *list == Value::NULL {
            return Ok(Vec::new());
        }
        if depth >= 64 {
            return Err(unsupported("system menu nesting limit exceeded"));
        }
        let count = vm.get_property(list, &Value::string("count"), true, false, self, budget)?;
        let Value::Integer(count) = count else {
            return Ok(Vec::new());
        };
        let count = count as i32;
        if count > 4096 {
            return Err(unsupported("system menu item limit exceeded"));
        }
        let mut items = Vec::new();
        for index in 0..count {
            charge(budget)?;
            let value = vm.get_property(
                list,
                &Value::Integer(index.into()),
                true,
                false,
                self,
                budget,
            )?;
            let Value::Object(reference) = value else {
                anyhow::bail!("system menu item must be an Object");
            };
            let Some(object) = reference.object else {
                continue;
            };
            // AsObjectNoAddRef ignores the input closure's bound context.
            let receiver = Value::Object(krkrz_tjs::ObjectRef {
                object: Some(object),
                context: Some(object),
            });
            let mut get = |services: &mut Self, vm: &mut Vm, key: &str| {
                vm.get_property(
                    &receiver,
                    &Value::string(key),
                    true,
                    false,
                    services,
                    budget,
                )
            };
            let visible = get(self, vm, "visible")?;
            if !matches!(visible, Value::Void) && visible.integer()? as i32 == 0 {
                continue;
            }
            let caption = get(self, vm, "caption")?.unary("string")?.text();
            let mut item = SystemMenuItem {
                command: None,
                caption,
                checked: false,
                radio: false,
                enabled: true,
                menu_break: 0,
                insert_position: None,
                insert_command: None,
                children: Vec::new(),
                source: Value::object(object),
            };
            if item.caption != "-" {
                let children = get(self, vm, "children")?;
                if let Value::Object(reference) = children {
                    let children = reference.object.map_or(Value::NULL, |object| {
                        Value::Object(krkrz_tjs::ObjectRef {
                            object: Some(object),
                            context: Some(object),
                        })
                    });
                    item.children =
                        self.build_system_menu(vm, &children, command, depth + 1, budget)?;
                }
                if item.children.is_empty() {
                    if *command < 0xe000 {
                        return Err(unsupported("system menu command limit exceeded"));
                    }
                    item.command = Some(*command);
                    *command -= 1;
                    let checked = vm.get_property(
                        &receiver,
                        &Value::string("checked"),
                        true,
                        false,
                        self,
                        budget,
                    )?;
                    item.checked =
                        !matches!(checked, Value::Void) && checked.integer()? as i32 != 0;
                    let group = vm.get_property(
                        &receiver,
                        &Value::string("group"),
                        true,
                        false,
                        self,
                        budget,
                    )?;
                    item.radio = matches!(group, Value::Integer(n) if n as i32 > 0);
                }
                let enabled = vm.get_property(
                    &receiver,
                    &Value::string("enabled"),
                    true,
                    false,
                    self,
                    budget,
                )?;
                item.enabled = matches!(enabled, Value::Void) || enabled.integer()? as i32 != 0;
            }
            item.menu_break = vm
                .get_property(
                    &receiver,
                    &Value::string("break"),
                    true,
                    false,
                    self,
                    budget,
                )?
                .integer()? as i32;
            let position = vm.get_property(
                &receiver,
                &Value::string("insertPos"),
                true,
                false,
                self,
                budget,
            )?;
            if let Value::Integer(n) = position {
                item.insert_position = (n as i32 >= 0).then_some(n as i32);
            } else {
                let command = vm.get_property(
                    &receiver,
                    &Value::string("insertID"),
                    true,
                    false,
                    self,
                    budget,
                )?;
                if let Value::Integer(n) = command {
                    item.insert_command = (n as i32 >= 0).then_some(n as i32);
                }
            }
            items.push(item);
        }
        Ok(items)
    }

    fn rebuild_system_menu(&mut self, vm: &mut Vm, window: usize, budget: &mut u64) -> Result<()> {
        let state = self
            .window_ex
            .windows
            .get_mut(&window)
            .context("Window extension is not registered")?;
        state.system_menu.clear();
        let source = state.system_menu_source.clone().unwrap_or(Value::NULL);
        let items = self.build_system_menu(vm, &source, &mut 0xefff, 0, budget)?;
        self.window_ex
            .windows
            .get_mut(&window)
            .context("Window invalidated while building its system menu")?
            .system_menu = items;
        Ok(())
    }
    fn notification_dictionary(&mut self, vm: &mut Vm) -> Result<Value> {
        let class = vm
            .globals
            .get("Window")
            .cloned()
            .context("cache setup failed.")?;
        let key = Value::string("_Notifications");
        if vm.has_member(&class, &key)? {
            return vm.get_member(&class, &key, false);
        }
        let dictionary = vm.new_dictionary()?;
        for (name, number, reverse) in interface()?.notifications {
            vm.set_member(
                &dictionary,
                &Value::string(&name),
                Value::Integer(number.into()),
            )?;
            vm.set_member(
                &dictionary,
                &Value::Integer(number.into()),
                Value::string(&reverse),
            )?;
        }
        vm.set_member(&class, &key, dictionary.clone())?;
        Ok(dictionary)
    }
    fn notification_value(
        &mut self,
        vm: &mut Vm,
        key: &Value,
        budget: &mut u64,
    ) -> Result<Option<Value>> {
        let dictionary = self.notification_dictionary(vm)?;
        if !vm.has_member(&dictionary, key)? {
            return Ok(None);
        }
        // ncbind's checkVariant then get*Value each read the property.
        vm.get_property(&dictionary, key, false, false, self, budget)?;
        Ok(Some(vm.get_property(
            &dictionary,
            key,
            false,
            false,
            self,
            budget,
        )?))
    }
    pub(crate) fn window_ex_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        charge(budget)?;
        if let Some(name) = operation.strip_prefix("MenuItem.") {
            return self.menu_appearance_call(vm, name, context, args, budget);
        }
        if matches!(
            operation,
            "System.getDisplayMonitors" | "System.getMonitorInfo"
        ) {
            return self.display_call(vm, operation, args);
        }
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("windowEx {operation}: missing argument {i}"))
        };
        match operation {
            // The terminal/headless host has no TTVPConsoleForm. windowEx
            // explicitly returns void/false when this optional window is absent.
            "Debug.console.getPlacement" | "Debug.console.getRect" | "Debug.console.bringAfter" => return Ok(Value::Void),
            "Debug.console.maximize" | "Debug.console.restoreMaximize" => return Ok(Value::Integer(0)),
            "Debug.console.setPlacement" => {
                ensure!(matches!(arg(0)?, Value::Object(_)), "console placement requires an Object");
                return Ok(Value::Integer(0));
            }
            "Debug.console.setPos" => {
                arg(1)?;
                return Ok(Value::Void);
            }
            "Window.getNotificationNum" => {
                let key = arg(0)?.unary("string")?;
                return Ok(Value::Integer(
                    self.notification_value(vm, &key, budget)?
                        .map(|v| v.integer())
                        .transpose()?
                        .unwrap_or(-1) as i32 as i64,
                ));
            }
            "Window.getNotificationName" => {
                let key = Value::Integer(arg(0)?.integer()? as i32 as i64);
                return self
                    .notification_value(vm, &key, budget)?
                    .unwrap_or(Value::Void)
                    .unary("string");
            }
            "Scripts.setEvalErrorLog" => {
                let new = arg(0)?.truth()?;
                let old = std::mem::replace(&mut self.window_ex.eval_error_log, new);
                return Ok(Value::Integer(old.into()));
            }
            "System.clearGraphicCache" => {
                self.image_cache.clear();
                return Ok(Value::Void);
            }
            "System.setIconicPreview" => {
                arg(0)?.truth()?;
                // The plugin returns SUCCEEDED(DwmSetWindowAttribute(...)).
                // A Linux/headless host has no Windows application HWND or DWM.
                return Ok(Value::Integer(0));
            }
            _ => {}
        }
        if let Some(name) = operation.strip_prefix("Window.") {
            let id = id(context)?;
            ensure!(
                self.windows.get(&id).is_some_and(|w| w.constructed),
                "context has no constructed Window native instance"
            );
            if matches!(name, "getWindowRect" | "getClientRect" | "getNormalRect") {
                if name == "getNormalRect"
                    && let Some(value) = args.first()
                {
                    value.integer()?;
                }
                let window = &self.windows[&id];
                let bounds = if name == "getClientRect" {
                    window.client_rect()
                } else {
                    window.window_rect()
                };
                let result = vm.new_dictionary()?;
                for (key, value) in ["x", "y", "w", "h"].into_iter().zip(bounds) {
                    vm.set_member(&result, &Value::string(key), Value::Integer(value.into()))?;
                }
                return Ok(result);
            }
            if name == "resetExSystemMenu" {
                if self
                    .window_ex
                    .windows
                    .get(&id)
                    .is_some_and(|w| !w.system_menu.is_empty())
                {
                    self.rebuild_system_menu(vm, id, budget)?;
                }
                return Ok(Value::Void);
            }
            if name == "set:exSystemMenu" {
                let state = self
                    .window_ex
                    .windows
                    .get_mut(&id)
                    .context("Invalid operation for Read-only or Write-only property")?;
                state.system_menu.clear();
                let Value::Object(reference) = arg(0)? else {
                    anyhow::bail!("exSystemMenu requires an Object");
                };
                state.system_menu_source = reference.object.map(|object| {
                    Value::Object(krkrz_tjs::ObjectRef {
                        object: Some(object),
                        context: Some(object),
                    })
                });
                self.rebuild_system_menu(vm, id, budget)?;
                return Ok(Value::Void);
            }
            if name == "registerExEvent" {
                self.window_ex.windows.entry(id).or_default();
                for (index, event) in ["onResizing", "onMoving", "onMove", "onNcMouseMove"]
                    .iter()
                    .enumerate()
                {
                    let key = Value::string(event);
                    let exists = vm.has_member(context, &key)?;
                    if exists {
                        vm.get_property(context, &key, false, false, self, budget)?;
                    }
                    self.window_ex
                        .windows
                        .get_mut(&id)
                        .context("Window invalidated during event registration")?
                        .events[index] = exists;
                }
                return Ok(Value::Void);
            }
            if let Some(property) = name.strip_prefix("get:") {
                let state = self.window_ex.windows.get(&id);
                return Ok(match property {
                    "minimized" => Value::Integer(self.windows[&id].minimized.into()),
                    "maximized" => Value::Integer(self.windows[&id].maximized.into()),
                    "disableResize" => {
                        Value::Integer(state.is_some_and(|s| s.disable_resize).into())
                    }
                    "disableMove" => Value::Integer(state.is_some_and(|s| s.disable_move).into()),
                    "enableNCMouseEvent" => {
                        Value::Integer(state.is_some_and(|s| s.nc_mouse).into())
                    }
                    "exSystemMenu" => state.map_or(Value::Void, |state| {
                        state.system_menu_source.clone().unwrap_or(Value::NULL)
                    }),
                    _ => {
                        return Err(unsupported(format!(
                            "windowEx native window property: {property}"
                        )));
                    }
                });
            }
            if matches!(
                name,
                "set:disableResize" | "set:disableMove" | "set:enableNCMouseEvent"
            ) {
                let state = self
                    .window_ex
                    .windows
                    .get_mut(&id)
                    .context("Invalid operation for Read-only or Write-only property")?;
                let value = arg(0)?.integer()? != 0;
                match name {
                    "set:disableResize" => state.disable_resize = value,
                    "set:disableMove" => state.disable_move = value,
                    _ => state.nc_mouse = value,
                }
                return Ok(Value::Void);
            }
            if name == "setMessageHook" {
                ensure!(
                    self.window_ex.windows.contains_key(&id),
                    "Invalid operation for Read-only or Write-only property"
                );
                let on = args.first().map(Value::integer).transpose()?.unwrap_or(0) != 0;
                let number = if let Some(key) = args.get(1) {
                    let number = if matches!(key, Value::String(_)) {
                        self.notification_value(vm, key, budget)?
                            .map(|v| v.integer())
                            .transpose()?
                            .unwrap_or(-1)
                    } else {
                        key.integer()?
                    } as i32;
                    ensure!((0..1024).contains(&number), "Unknown failure : FFFFFFFF");
                    Some(number as usize)
                } else {
                    None
                };
                let state = self
                    .window_ex
                    .windows
                    .get_mut(&id)
                    .context("Window invalidated during notification lookup")?;
                if let Some(number) = number {
                    if on {
                        state.hooks[number / 32] |= 1 << (number % 32);
                    } else {
                        state.hooks[number / 32] &= !(1 << (number % 32));
                    }
                } else {
                    state.hooks.fill(if on { u32::MAX } else { 0 });
                }
                return Ok(Value::Integer(state.hooks.iter().any(|v| *v != 0).into()));
            }
        }
        Err(unsupported(format!("windowEx operation: {operation}")))
    }
}
