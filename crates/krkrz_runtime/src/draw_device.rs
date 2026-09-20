//! Native draw-device ownership. OS presentation is a separate host operation.
use crate::{Services, layer::object};
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm, unsupported};
use std::collections::BTreeSet;
#[derive(Default)]
pub(crate) struct Devices {
    pub class: Option<Value>,
    live: BTreeSet<usize>,
}
impl Devices {
    pub fn new(class: Value) -> Self {
        Self {
            class: Some(class),
            live: BTreeSet::new(),
        }
    }
}
pub(crate) fn register(vm: &mut Vm) -> Result<Value> {
    let class = vm.new_native_class("BasicDrawDevice", "BasicDrawDevice.@initialize")?;
    for method in ["BasicDrawDevice", "finalize", "recreate"] {
        vm.register_native_method(&class, method, &format!("BasicDrawDevice.{method}"))?;
    }
    vm.register_native_property(
        &class,
        "interface",
        Some("BasicDrawDevice.get:interface"),
        None,
    )?;
    let window = vm.globals["Window"].clone();
    vm.register_native_static_value(&window, "BasicDrawDevice", class.clone())?;
    vm.register_native_property(
        &window,
        "drawDevice",
        Some("Window.get:drawDevice"),
        Some("Window.set:drawDevice"),
    )?;
    Ok(class)
}
impl Services {
    pub(crate) fn draw_device_call(&mut self, op: &str, context: &Value) -> Result<Value> {
        let id = object(context)?.context("BasicDrawDevice requires a non-null context")?;
        match op {
            "@initialize" => {
                self.draw_devices.live.insert(id);
            }
            "@invalidate" => {
                self.draw_devices.live.remove(&id);
            }
            "BasicDrawDevice" | "finalize" => {
                ensure!(
                    self.draw_devices.live.contains(&id),
                    "context has no BasicDrawDevice native instance"
                );
            }
            _ => {
                return Err(unsupported(format!(
                    "unsupported BasicDrawDevice operation: {op}"
                )));
            }
        }
        Ok(Value::Void)
    }
    pub(crate) fn set_draw_device(
        &mut self,
        vm: &mut Vm,
        window: usize,
        value: Value,
        budget: &mut u64,
    ) -> Result<()> {
        let old = self
            .windows
            .get(&window)
            .context("Window is invalid")?
            .draw_device
            .clone();
        if matches!(old, Value::Object(_)) && old != Value::NULL {
            vm.invalidate(&old, self, budget)?;
        }
        self.windows
            .get_mut(&window)
            .context("Window invalidated during draw-device finalization")?
            .draw_device = value.clone();
        if let Value::Object(_) = value {
            let device = object(&value)?.context("draw device is null")?;
            if !self.draw_devices.live.contains(&device) {
                return Err(unsupported(
                    "custom draw device interfaces are not implemented",
                ));
            }
        }
        Ok(())
    }
}
