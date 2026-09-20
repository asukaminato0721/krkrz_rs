//! Portable windowEx state, adapted from miahmie's windowEx (Kirikiri license).
//! Native window management remains a host operation, with explicit errors.
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm, unsupported};
use serde::Deserialize;
use std::collections::BTreeMap;

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
pub(crate) struct Window {
    disable_resize: bool,
    disable_move: bool,
    nc_mouse: bool,
    events: [bool; 4],
    hooks: [u32; 32],
}
impl Default for Window {
    fn default() -> Self {
        Self {
            disable_resize: false,
            disable_move: false,
            nc_mouse: false,
            events: [false; 4],
            hooks: [0; 32],
        }
    }
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
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("windowEx {operation}: missing argument {i}"))
        };
        match operation {
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
            _ => {}
        }
        if let Some(name) = operation.strip_prefix("Window.") {
            let id = id(context)?;
            ensure!(
                self.windows.get(&id).is_some_and(|w| w.constructed),
                "context has no constructed Window native instance"
            );
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
                    "exSystemMenu" => {
                        if state.is_some() {
                            Value::NULL
                        } else {
                            Value::Void
                        }
                    }
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
