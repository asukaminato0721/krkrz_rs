//! AsyncTrigger and deferred event priority, following Kirikiri EventIntf.cpp.
use crate::Services;
use crate::scheduler::EventKind;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{ObjectRef, Value, Vm, unsupported};
use std::collections::BTreeMap;

struct Trigger {
    constructed: bool,
    owner: Value,
    action: Value,
    cached: bool,
    mode: i32,
}
impl Default for Trigger {
    fn default() -> Self {
        Self {
            constructed: false,
            owner: Value::NULL,
            action: Value::string("action"),
            cached: true,
            mode: 0,
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    triggers: BTreeMap<usize, Trigger>,
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.register_native_class("AsyncTrigger")?;
    for method in ["AsyncTrigger", "finalize", "trigger", "cancel", "onFire"] {
        vm.register_native_method(&class, method, &format!("AsyncTrigger.{method}"))?;
    }
    for property in ["cached", "mode"] {
        vm.register_native_property(
            &class,
            property,
            Some(&format!("AsyncTrigger.get:{property}")),
            Some(&format!("AsyncTrigger.set:{property}")),
        )?;
    }
    Ok(())
}
fn id(value: &Value) -> Result<usize> {
    if let Value::Object(r) = value {
        return r.object.context("AsyncTrigger requires a non-null context");
    }
    anyhow::bail!("AsyncTrigger requires an object context")
}
impl Services {
    pub(crate) fn async_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let id = id(context)?;
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("AsyncTrigger.{operation}: missing argument {i}"))
        };
        if operation == "@initialize" {
            self.async_triggers.triggers.entry(id).or_default();
            return Ok(Value::Void);
        }
        if operation == "@invalidate" {
            self.events.cancel(id, EventKind::Trigger);
            self.async_triggers.triggers.remove(&id);
            return Ok(Value::Void);
        }
        let trigger = self
            .async_triggers
            .triggers
            .get_mut(&id)
            .context("context has no AsyncTrigger native instance")?;
        if operation == "AsyncTrigger" {
            if !trigger.constructed {
                let owner = arg(0)?;
                ensure!(
                    matches!(owner, Value::Object(_)),
                    "AsyncTrigger action owner must be an Object"
                );
                trigger.owner = owner.clone();
                if let Some(name) = args.get(1).filter(|v| !matches!(v, Value::Void)) {
                    trigger.action = name.unary("string")?;
                }
                trigger.constructed = true;
            }
            return Ok(Value::Void);
        }
        ensure!(trigger.constructed, "AsyncTrigger constructor has not run");
        match operation {
            "finalize" => {}
            "get:cached" => return Ok(Value::Integer(trigger.cached.into())),
            "get:mode" => return Ok(Value::Integer(trigger.mode.into())),
            "set:cached" => {
                let value = arg(0)?.truth()?;
                if trigger.cached != value {
                    trigger.cached = value;
                    self.events.cancel(id, EventKind::Trigger);
                }
            }
            "set:mode" => {
                let value = arg(0)?.integer()? as i32;
                if trigger.mode != value {
                    trigger.mode = value;
                    self.events.cancel(id, EventKind::Trigger);
                }
            }
            "cancel" => self.events.cancel(id, EventKind::Trigger),
            "trigger" => {
                let cached = trigger.cached;
                let priority = match trigger.mode {
                    1 => 1,
                    2 => 2,
                    _ => 0,
                };
                if cached {
                    self.events.cancel(id, EventKind::Trigger);
                }
                self.events.post(id, EventKind::Trigger, priority)?;
            }
            "onFire" => {
                let owner = trigger.owner.clone();
                let action_name = trigger.action.clone();
                if owner == Value::NULL {
                    return Ok(Value::Void);
                }
                let event = vm.new_dictionary()?;
                vm.set_member(&event, &Value::string("type"), Value::string("onFire"))?;
                vm.set_member(
                    &event,
                    &Value::string("target"),
                    Value::Object(ObjectRef {
                        object: Some(id),
                        context: Some(id),
                    }),
                )?;
                let callback = if action_name == Value::string("") {
                    owner.clone()
                } else {
                    // Missing action methods are ignored by native FuncCall.
                    let callback = vm.get_member(&owner, &action_name, true)?;
                    if callback == Value::Void {
                        return Ok(Value::Void);
                    }
                    callback
                };
                return vm.call_function(&callback, &owner, &[event], self, budget);
            }
            _ => return Err(unsupported(format!("AsyncTrigger.{operation}"))),
        }
        Ok(Value::Void)
    }
    pub(crate) fn dispatch_async(
        &mut self,
        vm: &mut Vm,
        through: u64,
        priority: i32,
        budget: &mut u64,
    ) -> Result<()> {
        while let Some(event) = self.events.pop(through, priority) {
            let id = event.target;
            *budget = budget
                .checked_sub(1)
                .ok_or_else(|| unsupported("posted event execution budget exceeded"))?;
            if match event.kind {
                EventKind::Trigger => !self.async_triggers.triggers.contains_key(&id),
                EventKind::Timer => !self.timers.contains_key(&id),
            } {
                continue;
            }
            let target = Value::object(id);
            let callback = vm.get_property(
                &target,
                &Value::string(event.kind.method()),
                false,
                false,
                self,
                budget,
            )?;
            vm.call_function(&callback, &target, &[], self, budget)?;
        }
        Ok(())
    }
}
