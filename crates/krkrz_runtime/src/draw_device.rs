//! Native draw-device ownership. OS presentation is a separate host operation.
use crate::{Services, layer::object};
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm, unsupported};
use std::collections::BTreeMap;
#[derive(Default)]
pub(crate) struct Devices {
    pub class: Option<Value>,
    live: BTreeMap<usize, u64>,
}
impl Devices {
    pub fn new(class: Value) -> Self {
        Self {
            class: Some(class),
            live: BTreeMap::new(),
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
                self.draw_devices.live.entry(id).or_insert(0);
            }
            "@invalidate" => {
                self.draw_devices.live.remove(&id);
            }
            "BasicDrawDevice" | "finalize" => {
                ensure!(
                    self.draw_devices.live.contains_key(&id),
                    "context has no BasicDrawDevice native instance"
                );
            }
            "get:interface" => {
                ensure!(
                    self.draw_devices.live.contains_key(&id),
                    "context has no BasicDrawDevice native instance"
                );
                return Ok(Value::Integer(id as i64 + 1));
            }
            "recreate" => {
                let generation = self
                    .draw_devices
                    .live
                    .get_mut(&id)
                    .context("context has no BasicDrawDevice native instance")?;
                // CPU pixels belong to Layers and survive device recreation.
                // Hosts must discard presentation resources for the old epoch.
                *generation = generation.wrapping_add(1);
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
            if !self.draw_devices.live.contains_key(&device) {
                return Err(unsupported(
                    "custom draw device interfaces are not implemented",
                ));
            }
        }
        Ok(())
    }
}

impl crate::Session {
    /// Identifies presentation resources. A device replacement or `recreate`
    /// changes this pair while preserving the script's layer contents.
    pub fn draw_device_epoch(&self, window: &Value) -> Result<Option<(usize, u64)>> {
        let id = object(window)?.context("Window requires a non-null context")?;
        let window = self
            .services
            .windows
            .get(&id)
            .context("Window is invalid")?;
        if let Value::Object(reference) = window.draw_device
            && let Some(id) = reference.object
        {
            let generation = self
                .services
                .draw_devices
                .live
                .get(&id)
                .context("Draw device is invalid")?;
            return Ok(Some((id, *generation)));
        }
        Ok(None)
    }
}
