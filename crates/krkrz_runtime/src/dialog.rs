//! Portable WIN32Dialog data model. Reference: wtnbgo/win32dialog (Kirikiri license).
use crate::Services;
use anyhow::{Context, Result, bail, ensure};
use krkrz_tjs::{Value, Vm, unsupported};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Deserialize)]
struct Exports {
    methods: Vec<String>,
    properties: Vec<String>,
    constants: BTreeMap<String, i64>,
}
#[derive(Default)]
pub(crate) struct State {
    links: BTreeSet<String>,
    objects: BTreeMap<(usize, String), Object>,
}
#[derive(Clone, Default)]
struct Template {
    fields: BTreeMap<String, Value>,
    count: u16,
}
#[derive(Clone)]
enum Object {
    Pending,
    Dialog { modeless: bool, template: Vec<u8> },
    Template(Template),
    Blob(Vec<u8>),
}
fn id(v: &Value) -> Result<usize> {
    match v {
        Value::Object(r) => r.object.context("WIN32Dialog requires a non-null object"),
        _ => bail!("WIN32Dialog requires an object"),
    }
}
fn charge(budget: &mut u64, n: u64) -> Result<()> {
    *budget = budget
        .checked_sub(n)
        .ok_or_else(|| unsupported("WIN32Dialog execution budget exceeded"))?;
    Ok(())
}
impl State {
    pub(crate) fn link(&mut self, vm: &mut Vm, spelling: &str) -> Result<()> {
        if self.links.contains(spelling) {
            return Ok(());
        }
        ensure!(self.links.is_empty(), "Already registerd class.");
        let exports: BTreeMap<String, Exports> =
            serde_json::from_str(include_str!("../data/win32dialog.json"))?;
        let root = vm.new_native_class("WIN32Dialog", "WIN32Dialog.WIN32Dialog.@initialize")?;
        for (name, e) in exports {
            let prefix = format!("WIN32Dialog.{name}");
            let class = if name == "WIN32Dialog" {
                root.clone()
            } else {
                vm.new_native_class(&name, &format!("{prefix}.@initialize"))?
            };
            for method in e.methods {
                let operation = format!("{prefix}.{method}");
                if matches!(
                    method.as_str(),
                    "messageBox"
                        | "chooseColor"
                        | "initCommonControls"
                        | "initCommonControlsEx"
                        | "getOctetAddress"
                        | "getStringAddress"
                        | "ReferPointer"
                ) {
                    vm.register_native_static_method(&class, &method, &operation)?;
                } else {
                    vm.register_native_method(&class, &method, &operation)?;
                }
            }
            for property in e.properties {
                let write = matches!(
                    property.as_str(),
                    "modeless" | "icon" | "progressValue" | "progressCanceled" | "dlgItems"
                );
                vm.register_native_property(
                    &class,
                    &property,
                    Some(&format!("{prefix}.get:{property}")),
                    write.then(|| format!("{prefix}.set:{property}")).as_deref(),
                )?;
            }
            for (k, v) in e.constants {
                vm.set_member(&class, &Value::string(&k), Value::Integer(v))?;
            }
            if name != "WIN32Dialog" {
                vm.register_native_static_value(&root, &name, class)?;
            }
        }
        vm.globals.insert("WIN32Dialog".into(), root);
        self.links.insert(spelling.into());
        Ok(())
    }
}
impl Template {
    fn number(&self, k: &str) -> i64 {
        self.fields
            .get(k)
            .and_then(|v| v.integer().ok())
            .unwrap_or(0)
    }
    fn text(&self, k: &str) -> Vec<u16> {
        match self.fields.get(k) {
            Some(Value::String(s)) => s.clone(),
            _ => vec![],
        }
    }
    fn write(&self, bytes: &mut Vec<u8>, header: bool) {
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        if header {
            word(bytes, 1);
            word(bytes, 0xffff);
        }
        for k in ["helpID", "exStyle", "style"] {
            bytes.extend_from_slice(&(self.number(k) as u32).to_le_bytes());
        }
        if header {
            word(bytes, self.count);
        }
        for k in ["x", "y", "cx", "cy"] {
            word(bytes, self.number(k) as u16);
        }
        if !header {
            bytes.extend_from_slice(&(self.number("id") as u32).to_le_bytes());
        }
        if header {
            self.ordinal(bytes, "menu");
        }
        self.ordinal(bytes, "windowClass");
        if header {
            string(bytes, &self.text("title"));
        } else {
            self.ordinal(bytes, "title");
        }
        if header && self.number("style") & 0x40 != 0 {
            word(bytes, self.number("pointSize") as u16);
            word(bytes, self.number("weight") as u16);
            bytes.push(self.number("italic") as u8);
            bytes.push(self.number("charset") as u8);
            string(bytes, &self.text("typeFace"));
        }
        if !header {
            word(bytes, 0);
        }
    }
    fn ordinal(&self, bytes: &mut Vec<u8>, key: &str) {
        match self.fields.get(key) {
            Some(Value::String(v)) => string(bytes, v),
            Some(Value::Integer(v)) => {
                word(bytes, 0xffff);
                word(bytes, *v as u16);
            }
            _ => word(bytes, 0),
        }
    }
}
fn word(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn string(b: &mut Vec<u8>, s: &[u16]) {
    for &c in s.iter().take_while(|c| **c != 0) {
        word(b, c);
    }
    word(b, 0);
}
impl Services {
    pub(crate) fn dialog_call(
        &mut self,
        vm: &mut Vm,
        op: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let (class, method) = op.split_once('.').context("invalid dialog operation")?;
        let object_id = id(context)?;
        let key = (object_id, class.to_string());
        if method == "@initialize" {
            self.dialogs.objects.insert(key, Object::Pending);
            return Ok(Value::Void);
        }
        if method == "@invalidate" {
            self.dialogs.objects.remove(&key);
            return Ok(Value::Void);
        }
        let arg = |i| {
            args.get(i)
                .with_context(|| format!("WIN32Dialog.{op}: missing argument {i}"))
        };
        if method == class {
            let object = match class {
                "WIN32Dialog" => {
                    ensure!(
                        matches!(arg(0)?, Value::Object(_)),
                        "WIN32Dialog owner must be an object or null"
                    );
                    Object::Dialog {
                        modeless: false,
                        template: vec![],
                    }
                }
                "Header" | "Items" => Object::Template(Template::default()),
                "Blob" => {
                    let size = arg(0)?.integer()? as u32 as usize;
                    ensure!(
                        size <= 16 * 1024 * 1024,
                        "WIN32Dialog.Blob exceeds 16 MiB limit"
                    );
                    charge(budget, (size as u64).div_ceil(256))?;
                    Object::Blob(vec![0; size])
                }
                _ => return Err(unsupported(format!("WIN32Dialog.{op}"))),
            };
            ensure!(
                self.dialogs.objects.contains_key(&key),
                "context has no WIN32Dialog native instance"
            );
            self.dialogs.objects.insert(key, object);
            return Ok(Value::Void);
        }
        if matches!(
            method,
            "messageBox"
                | "chooseColor"
                | "initCommonControls"
                | "initCommonControlsEx"
                | "getOctetAddress"
                | "getStringAddress"
                | "ReferPointer"
        ) {
            return Err(unsupported(format!(
                "WIN32Dialog.{op}: platform operation is not implemented"
            )));
        }
        let object = self
            .dialogs
            .objects
            .get(&key)
            .context("context has no WIN32Dialog native instance")?;
        if method == "finalize" {
            return Ok(Value::Void);
        }
        match object {
            Object::Dialog { modeless, .. } => match method {
                "get:modeless" => return Ok(Value::Integer((*modeless).into())),
                "get:isValid" | "get:HWND" | "get:propsheet" | "get:progress" => {
                    return Ok(Value::Integer(0));
                }
                "get:icon" => return Ok(Value::Void),
                "set:modeless" => {
                    let value = arg(0)?.truth()?;
                    if let Object::Dialog { modeless, .. } =
                        self.dialogs.objects.get_mut(&key).unwrap()
                    {
                        *modeless = value;
                    }
                    return Ok(Value::Void);
                }
                "get:progressValue"
                | "set:progressValue"
                | "get:progressCanceled"
                | "set:progressCanceled" => bail!("dialog is not progress mode."),
                "close" => {
                    arg(0)?.integer()?;
                    return Ok(Value::Void);
                }
                "closeProgress" => return Ok(Value::Void),
                "onInit" | "onCommand" | "onHScroll" | "onVScroll" | "onSize" => {
                    for i in (0..3).rev() {
                        arg(i)?.integer()?;
                    }
                    return Ok(Value::Integer(0));
                }
                "makeTemplate" => {
                    let h = id(arg(0)?)?;
                    let Object::Template(head) = self
                        .dialogs
                        .objects
                        .get(&(h, "Header".into()))
                        .context("makeTemplate requires a Header")?
                    else {
                        bail!("makeTemplate requires a Header");
                    };
                    let mut bytes = vec![];
                    head.write(&mut bytes, true);
                    for v in &args[1..] {
                        charge(budget, 1)?;
                        let item = id(v)?;
                        let Object::Template(item) = self
                            .dialogs
                            .objects
                            .get(&(item, "Items".into()))
                            .context("makeTemplate requires Items")?
                        else {
                            bail!("makeTemplate requires Items");
                        };
                        item.write(&mut bytes, false);
                        ensure!(
                            bytes.len() <= 16 * 1024 * 1024,
                            "dialog template exceeds 16 MiB limit"
                        );
                    }
                    charge(budget, (bytes.len() as u64).div_ceil(256))?;
                    if let Object::Dialog { template, .. } =
                        self.dialogs.objects.get_mut(&key).unwrap()
                    {
                        *template = bytes;
                    }
                    return Ok(Value::Void);
                }
                _ => {}
            },
            Object::Template(_) => match method {
                "get:dlgItems" if class == "Header" => {
                    let Object::Template(t) = object else {
                        unreachable!()
                    };
                    return Ok(Value::Integer(t.count as i64));
                }
                "set:dlgItems" if class == "Header" => {
                    let n = arg(0)?.integer()? as u16;
                    if let Object::Template(t) = self.dialogs.objects.get_mut(&key).unwrap() {
                        t.count = n;
                    }
                    return Ok(Value::Void);
                }
                "store" => {
                    let input = arg(0)?.clone();
                    id(&input)?;
                    let keys = if class == "Header" {
                        &[
                            "helpID",
                            "exStyle",
                            "style",
                            "x",
                            "y",
                            "cx",
                            "cy",
                            "menu",
                            "windowClass",
                            "title",
                            "pointSize",
                            "weight",
                            "italic",
                            "charset",
                            "typeFace",
                        ][..]
                    } else {
                        &[
                            "helpID",
                            "exStyle",
                            "style",
                            "x",
                            "y",
                            "cx",
                            "cy",
                            "id",
                            "windowClass",
                            "title",
                        ][..]
                    };
                    for &field in keys {
                        charge(budget, 1)?;
                        let exists = vm.has_member(&input, &Value::string(field))?;
                        let first = vm.get_property(
                            &input,
                            &Value::string(field),
                            true,
                            false,
                            self,
                            budget,
                        )?;
                        if !exists {
                            continue;
                        }
                        let second = vm.get_property(
                            &input,
                            &Value::string(field),
                            true,
                            false,
                            self,
                            budget,
                        )?;
                        let ordinal = matches!(field, "menu" | "windowClass")
                            || class == "Items" && field == "title";
                        let is_string = if ordinal {
                            matches!(first, Value::String(_))
                        } else {
                            matches!(field, "title" | "typeFace")
                        };
                        let value = if is_string {
                            second.unary("string")?
                        } else {
                            Value::Integer(second.integer()? as i32 as i64)
                        };
                        if let Value::String(v) = &value {
                            ensure!(
                                v.len() <= 1_000_000,
                                "dialog template string exceeds one million UTF-16 units"
                            );
                            charge(budget, v.len() as u64)?;
                        }
                        let Some(Object::Template(t)) = self.dialogs.objects.get_mut(&key) else {
                            bail!("dialog template was invalidated during store");
                        };
                        t.fields.insert(field.into(), value);
                    }
                    return Ok(Value::Void);
                }
                _ => {}
            },
            Object::Blob(data) => {
                let width = match method {
                    "getByte" | "setByte" => 1,
                    "getWord" | "setWord" => 2,
                    "getDWord" | "setDWord" => 4,
                    _ => 0,
                };
                if width != 0 {
                    let value = if method.starts_with("set") {
                        Some(arg(1)?.integer()? as u32)
                    } else {
                        None
                    };
                    let offset = arg(0)?.integer()? as i32;
                    ensure!(offset >= 0, "negative Blob offset");
                    let offset = offset as usize;
                    ensure!(
                        offset
                            .checked_add(width)
                            .is_some_and(|end| end <= data.len()),
                        "Blob access out of bounds"
                    );
                    charge(budget, width as u64)?;
                    if let Some(value) = value {
                        let Object::Blob(data) = self.dialogs.objects.get_mut(&key).unwrap() else {
                            unreachable!()
                        };
                        data[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
                        return Ok(Value::Void);
                    }
                    let mut bytes = [0; 4];
                    bytes[..width].copy_from_slice(&data[offset..offset + width]);
                    return Ok(Value::Integer(u32::from_le_bytes(bytes) as i64));
                }
            }
            Object::Pending => bail!("WIN32Dialog constructor has not run"),
        }
        Err(unsupported(format!(
            "WIN32Dialog.{op}: operation is not implemented"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::Object;
    use crate::Session;
    #[test]
    fn template_owns_aligned_utf16_data_after_sources_are_invalidated() {
        let dir = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.tjs"),"Plugins.link('win32dialog.dll');var d=new WIN32Dialog(null),h=new WIN32Dialog.Header(),i=new WIN32Dialog.Items();h.store(%[style:64,title:'D',pointSize:12,weight:400,typeFace:'A']);h.dlgItems=1;i.store(%[id:42,x:1,y:2,cx:3,cy:4,windowClass:128,title:'OK']);d.makeTemplate(h,i);h.store(%[title:'changed']);invalidate h;invalidate i;").unwrap();
        let mut session = Session::open(dir.path(), Some(saves.path()), false, 100_000).unwrap();
        session.execute_storage("test.tjs").unwrap();
        assert_eq!(session.services.dialogs.objects.len(), 1);
        let Object::Dialog { template, .. } =
            session.services.dialogs.objects.values().next().unwrap()
        else {
            panic!("dialog missing")
        };
        assert_eq!(template.len(), 80);
        assert_eq!(&template[..4], &[1, 0, 255, 255]);
        assert_eq!(&template[12..18], &[64, 0, 0, 0, 1, 0]);
        assert_eq!(
            &template[30..44],
            &[68, 0, 0, 0, 12, 0, 144, 1, 0, 0, 65, 0, 0, 0]
        );
        assert_eq!(
            &template[56..80],
            &[
                1, 0, 2, 0, 3, 0, 4, 0, 42, 0, 0, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0
            ]
        );
    }
}
