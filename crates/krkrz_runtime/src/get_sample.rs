//! getSample.dll, following Kirikiroid2 src/plugins/getSample.cpp.
//! Native sample buffers use scoped integer handles, never script pointers.
use crate::Services;
use anyhow::{Context, Result};
use krkrz_tjs::{Value, Vm, unsupported};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(crate) struct Settings {
    count: i32,
    ahead: i32,
}
pub(crate) struct State {
    defaults: Settings,
    pub(crate) instances: BTreeMap<usize, Settings>,
    pub(crate) buffers: BTreeMap<i64, Vec<i16>>,
    next: i64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            defaults: Settings {
                count: 100,
                ahead: 0,
            },
            instances: BTreeMap::new(),
            buffers: BTreeMap::new(),
            next: 1,
        }
    }
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.globals["WaveSoundBuffer"].clone();
    for name in ["getSample", "setDefaultCounts", "setDefaultAheads"] {
        vm.register_native(&format!("WaveSoundBuffer.{name}"))?;
    }
    for name in ["sampleCount", "sampleAhead", "sampleValue"] {
        vm.register_native_property(
            &class,
            name,
            Some(&format!("WaveSoundBuffer.get:{name}")),
            (name != "sampleValue")
                .then(|| format!("WaveSoundBuffer.set:{name}"))
                .as_deref(),
        )?;
    }
    Ok(())
}
fn count(value: i32) -> Result<i32> {
    if !(0..=1_000_000).contains(&value) {
        return Err(unsupported("sample buffer size exceeds supported range"));
    }
    Ok(value)
}
impl Services {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sample_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
        result_needed: bool,
    ) -> Result<Value> {
        let arg = || {
            args.first()
                .context("missing getSample argument")?
                .integer()
                .map(|n| n as i32)
        };
        match operation {
            "setDefaultCounts" => {
                self.sample_plugin.defaults.count = count(arg()?)?;
                return Ok(Value::Void);
            }
            "setDefaultAheads" => {
                self.sample_plugin.defaults.ahead = arg()?;
                return Ok(Value::Void);
            }
            _ => {}
        }
        let Value::Object(reference) = context else {
            anyhow::bail!("sample access requires an object context")
        };
        let id = reference
            .object
            .context("sample access requires a non-null context")?;
        let legacy = operation == "getSample";
        let settings = if legacy {
            let n = if args.is_empty() { 100 } else { arg()? };
            if n <= 0 || !result_needed {
                return Ok(Value::Void);
            }
            Settings {
                count: count(n)?,
                ahead: 0,
            }
        } else {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.sample_plugin.instances.entry(id)
            {
                entry.insert(self.sample_plugin.defaults);
                vm.set_property(
                    context,
                    &Value::string("useVisBuffer"),
                    Value::Integer(1),
                    false,
                    self,
                    budget,
                )?;
            }
            let settings = self
                .sample_plugin
                .instances
                .get_mut(&id)
                .context("sample owner invalidated during initialization")?;
            match operation {
                "get:sampleCount" => return Ok(Value::Integer(settings.count.into())),
                "get:sampleAhead" => return Ok(Value::Integer(settings.ahead.into())),
                "set:sampleCount" => {
                    settings.count = count(arg()?)?;
                    return Ok(Value::Void);
                }
                "set:sampleAhead" => {
                    settings.ahead = arg()?;
                    return Ok(Value::Void);
                }
                "get:sampleValue" => *settings,
                _ => return Err(unsupported(format!("getSample operation {operation}"))),
            }
        };
        let handle = self.sample_plugin.next;
        self.sample_plugin.next = handle
            .checked_add(1)
            .context("sample handle limit reached")?;
        self.sample_plugin
            .buffers
            .insert(handle, vec![0; settings.count as usize]);
        let result = (|| -> Result<Value> {
            let callback = vm.get_member(context, &Value::string("getVisBuffer"), false)?;
            let mut args = vec![
                Value::Integer(handle),
                Value::Integer(settings.count.into()),
                Value::Integer(1),
            ];
            if !legacy {
                args.push(Value::Integer(settings.ahead.into()));
            }
            vm.call_function(&callback, context, &args, self, budget)
        })();
        let buffer = self
            .sample_plugin
            .buffers
            .remove(&handle)
            .expect("scoped sample buffer");
        let result = result?;
        if legacy {
            // The original can read unwritten malloc bytes. Zero initialization
            // gives deterministic results without reproducing that memory bug.
            let (sum, n) = buffer
                .iter()
                .filter(|n| **n >= 0)
                .fold((0i32, 0i32), |(sum, n), v| {
                    (sum.wrapping_add(*v as i32), n + 1)
                });
            return Ok(Value::Integer(if n == 0 { 0 } else { (sum / n).into() }));
        }
        let written = result.integer()? as i32;
        let written = if written < 0 || written > settings.count {
            settings.count
        } else {
            written
        } as usize;
        let peak = buffer[..written]
            .iter()
            .map(|n| (*n as i32).unsigned_abs())
            .max()
            .unwrap_or(0) as f64
            / 32768.0;
        Ok(Value::Real(peak * peak))
    }
}
