//! Kirikiri's nested PhaseVocoder class and filter-chain connections.
//! Adapted from sound/PhaseVocoderFilter.{cpp,h}; see THIRD_PARTY_NOTICES.md.
mod dsp;
use crate::Services;
use anyhow::{Context, Result, ensure};
pub(crate) use dsp::Pipeline;
use krkrz_tjs::{Value, Vm};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Config {
    window: usize,
    overlap: usize,
    pitch: f32,
    time: f32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            window: 4096,
            overlap: 0,
            pitch: 1.0,
            time: 1.0,
        }
    }
}
pub(crate) type Settings = Rc<Cell<Config>>;

pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.new_native_class("PhaseVocoder", "PhaseVocoder.@initialize")?;
    for name in ["PhaseVocoder", "finalize"] {
        vm.register_native_method(&class, name, &format!("PhaseVocoder.{name}"))?;
    }
    for name in ["window", "overlap", "pitch", "time", "interface"] {
        vm.register_native_property(
            &class,
            name,
            Some(&format!("PhaseVocoder.get:{name}")),
            (name != "interface")
                .then(|| format!("PhaseVocoder.set:{name}"))
                .as_deref(),
        )?;
    }
    let sound = vm.globals["WaveSoundBuffer"].clone();
    vm.register_native_static_value(&sound, "PhaseVocoder", class)
}

impl Services {
    pub(crate) fn validate_sound_filter_memory(&self) -> Result<()> {
        let bytes: usize = self
            .sounds
            .values()
            .filter_map(|s| s.loaded.as_ref())
            .map(|s| s.pipeline.memory_bound())
            .sum();
        ensure!(
            bytes <= 128 << 20,
            "session wave filter memory limit exceeded"
        );
        Ok(())
    }
    pub(crate) fn vocoder_call(
        &mut self,
        op: &str,
        context: &Value,
        args: &[Value],
    ) -> Result<Value> {
        let Value::Object(reference) = context else {
            anyhow::bail!("PhaseVocoder requires an object context")
        };
        let id = reference
            .object
            .context("PhaseVocoder requires a non-null context")?;
        if op == "@initialize" {
            self.vocoders
                .entry(id)
                .or_insert_with(|| Rc::new(Cell::new(Config::default())));
            return Ok(Value::Void);
        }
        if op == "@invalidate" {
            self.vocoders.remove(&id);
            return Ok(Value::Void);
        }
        let settings = self
            .vocoders
            .get(&id)
            .context("context has no PhaseVocoder native instance")?;
        let mut config = settings.get();
        let arg = || args.first().context("PhaseVocoder setter requires a value");
        match op {
            "PhaseVocoder" | "finalize" => return Ok(Value::Void),
            // A session-local handle, never a machine address.
            "get:interface" => return Ok(Value::Integer(id as i64 + 1)),
            "get:window" => return Ok(Value::Integer(config.window as i64)),
            "get:overlap" => return Ok(Value::Integer(config.overlap as i64)),
            "get:pitch" => return Ok(Value::Real(config.pitch as f64)),
            "get:time" => return Ok(Value::Real(config.time as f64)),
            "set:window" => {
                let size = arg()?.integer()? as i32;
                ensure!(
                    (64..=32768).contains(&size) && (size as u32).is_power_of_two(),
                    "invalid vocoder window: expected a power of two from 64 to 32768"
                );
                config.window = size as usize;
            }
            "set:overlap" => {
                let overlap = arg()?.integer()? as i32;
                ensure!(
                    matches!(overlap, 0 | 2 | 4 | 8 | 16 | 32),
                    "invalid vocoder overlap: expected 0, 2, 4, 8, 16 or 32"
                );
                config.overlap = overlap as usize;
            }
            "set:pitch" => config.pitch = arg()?.real()? as f32,
            "set:time" => config.time = arg()?.real()? as f32,
            _ => anyhow::bail!("unknown PhaseVocoder operation: {op}"),
        }
        settings.set(config);
        Ok(Value::Void)
    }

    pub(crate) fn connect_sound_filters(
        &mut self,
        vm: &mut Vm,
        sound: usize,
        channels: usize,
        budget: &mut u64,
    ) -> Result<Pipeline> {
        let array = self
            .sounds
            .get(&sound)
            .context("sound invalidated while connecting filters")?
            .filters
            .clone();
        let count = vm
            .get_property(&array, &Value::string("count"), false, false, self, budget)?
            .integer()?;
        ensure!(
            (0..=32).contains(&count),
            "sound filter count exceeds limit"
        );
        let mut connected = Vec::new();
        for index in 0..count {
            let value =
                vm.get_property(&array, &Value::Integer(index), false, false, self, budget)?;
            let interface = vm.get_property(
                &value,
                &Value::string("interface"),
                true,
                false,
                self,
                budget,
            )?;
            if matches!(interface, Value::Void) {
                continue;
            }
            let handle = interface.integer()?;
            let id = usize::try_from(handle)
                .ok()
                .and_then(|n| n.checked_sub(1))
                .context("invalid wave filter handle")?;
            let settings = self
                .vocoders
                .get(&id)
                .context("unknown or invalidated wave filter handle")?
                .clone();
            ensure!(
                !connected.iter().any(|(other, _)| *other == id)
                    && !self
                        .sounds
                        .values()
                        .filter_map(|s| s.loaded.as_ref())
                        .any(|s| s.pipeline.contains(id)),
                "cannot connect a wave filter to multiple sources at once"
            );
            connected.push((id, settings));
        }
        let pipeline = Pipeline::new(channels, connected);
        let other_bytes: usize = self
            .sounds
            .values()
            .filter_map(|s| s.loaded.as_ref())
            .map(|s| s.pipeline.memory_bound())
            .sum();
        ensure!(
            other_bytes + pipeline.memory_bound() <= 128 << 20,
            "session wave filter memory limit exceeded"
        );
        Ok(pipeline)
    }
}
