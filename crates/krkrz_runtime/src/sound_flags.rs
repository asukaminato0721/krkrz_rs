//! Script-visible SLI flags, following Kirikiri sound/WaveIntf.cpp.
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm};

pub(crate) fn register(vm: &mut Vm) -> Result<Value> {
    let class = vm.new_native_class("WaveFlags", "WaveFlags.@initialize")?;
    for name in ["WaveFlags", "finalize", "reset"] {
        vm.register_native_method(&class, name, &format!("WaveFlags.{name}"))?;
    }
    vm.register_native_property(&class, "count", Some("WaveFlags.count"), None)?;
    for i in 0..16 {
        vm.register_native_property(
            &class,
            &i.to_string(),
            Some(&format!("WaveFlags.get:{i}")),
            Some(&format!("WaveFlags.set:{i}")),
        )?;
    }
    Ok(class)
}

impl Services {
    pub(crate) fn wave_flags_call(
        &mut self,
        op: &str,
        context: &Value,
        args: &[Value],
    ) -> Result<Value> {
        let Value::Object(r) = context else {
            anyhow::bail!("WaveFlags requires an object context")
        };
        let id = r.object.context("null WaveFlags receiver")?;
        if op == "@initialize" {
            self.wave_flags.entry(id).or_insert(None);
            return Ok(Value::Void);
        }
        if op == "@invalidate" {
            self.wave_flags.remove(&id);
            return Ok(Value::Void);
        }
        let owner = self
            .wave_flags
            .get_mut(&id)
            .context("context has no WaveFlags instance")?;
        if op == "WaveFlags" {
            let Some(Value::Object(r)) = args.first() else {
                anyhow::bail!("WaveFlags requires a sound buffer")
            };
            let sound = r.object.context("null sound buffer")?;
            ensure!(
                self.sounds.contains_key(&sound),
                "WaveFlags requires a sound buffer"
            );
            *owner = Some(sound);
            return Ok(Value::Void);
        }
        if op == "count" {
            return Ok(Value::Integer(16));
        }
        if op == "finalize" {
            return Ok(Value::Void);
        }
        let sound = self
            .sounds
            .get_mut(&owner.context("WaveFlags has not been constructed")?)
            .context("WaveFlags sound buffer is invalid")?;
        if op == "reset" {
            if let Some(loaded) = &mut sound.loaded {
                loaded.stream.reset_flags();
            }
            return Ok(Value::Void);
        }
        let (access, index) = op.split_once(':').context("invalid WaveFlags operation")?;
        let index: usize = index.parse()?;
        ensure!(index < 16, "sound flag index out of range");
        match access {
            "get" => Ok(Value::Integer(
                sound
                    .loaded
                    .as_ref()
                    .map(|l| l.stream.flag(index))
                    .transpose()?
                    .unwrap_or(0)
                    .into(),
            )),
            "set" => {
                let value = args.first().context("WaveFlags setter requires a value")?;
                // The unloaded engine does not convert the assigned value.
                if let Some(loaded) = &mut sound.loaded {
                    loaded.stream.set_flag(index, value.integer()? as i32)?;
                }
                Ok(Value::Void)
            }
            _ => anyhow::bail!("invalid WaveFlags operation"),
        }
    }
}
