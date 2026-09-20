//! Sound control and fade timing from Kirikiri SoundBufferBaseIntf.cpp.
//! Decoded streams and device playback are not yet attached to these objects.
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{ObjectRef, Value, Vm, unsupported};

const BEAT_MS: u64 = 60;

struct Fade {
    target: i32,
    delta: i32,
    remaining: u64,
    delay: u64,
}

pub(crate) struct Sound {
    owner: Value,
    constructed: bool,
    volume: i32,
    volume2: i32,
    pan: i32,
    paused: bool,
    looping: bool,
    fade: Option<Fade>,
    pub(crate) fade_pending: bool,
}

impl Default for Sound {
    fn default() -> Self {
        Self {
            owner: Value::NULL,
            constructed: false,
            volume: 100_000,
            volume2: 100_000,
            pan: 0,
            paused: false,
            looping: false,
            fade: None,
            fade_pending: false,
        }
    }
}

pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.register_native_class("WaveSoundBuffer")?;
    for name in [
        "WaveSoundBuffer",
        "finalize",
        "open",
        "play",
        "stop",
        "fade",
        "stopFade",
        "setPos",
        "onStatusChanged",
        "onFadeCompleted",
        "onLabel",
    ] {
        vm.register_native(&format!("WaveSoundBuffer.{name}"))?;
    }
    for (name, writable) in [
        ("volume", true),
        ("volume2", true),
        ("pan", true),
        ("paused", true),
        ("looping", true),
        ("position", true),
        ("samplePosition", true),
        ("totalTime", true),
        ("frequency", true),
        ("posX", true),
        ("posY", true),
        ("posZ", true),
        ("status", false),
        ("bits", false),
        ("channels", false),
        ("flags", false),
        ("labels", false),
        ("filters", false),
    ] {
        vm.register_native_property(
            &class,
            name,
            Some(&format!("WaveSoundBuffer.get:{name}")),
            writable
                .then(|| format!("WaveSoundBuffer.set:{name}"))
                .as_deref(),
        )?;
    }
    for name in ["globalVolume", "globalFocusMode"] {
        vm.register_native_static_property(
            &class,
            name,
            Some(&format!("WaveSoundBuffer.get:{name}")),
            Some(&format!("WaveSoundBuffer.set:{name}")),
        )?;
    }
    Ok(())
}

impl Sound {
    pub(crate) fn beat(&mut self, mut beats: u64) {
        let Some(fade) = &mut self.fade else { return };
        let delay_beats = fade.delay.div_ceil(BEAT_MS).min(beats);
        fade.delay = fade.delay.saturating_sub(delay_beats * BEAT_MS);
        beats -= delay_beats;
        if beats == 0 || fade.remaining == 0 {
            // Upstream leaves a fade shorter than one beat pending forever if
            // it had a delay. stopFade can still finish it explicitly.
            return;
        }
        if beats >= fade.remaining {
            self.volume = fade.target.clamp(0, 100_000);
            self.fade = None;
            self.fade_pending = true;
        } else {
            self.volume =
                (self.volume as i64 + fade.delta as i64 * beats as i64).clamp(0, 100_000) as i32;
            fade.remaining -= beats;
        }
    }
}

impl Services {
    pub(crate) fn sound_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("WaveSoundBuffer.{operation}: missing argument {i}"))
        };
        // Static properties also work with a class context.
        match operation {
            "get:globalVolume" => return Ok(Value::Integer(self.sound_global_volume.into())),
            "set:globalVolume" => {
                self.sound_global_volume = (arg(0)?.integer()? as i32).clamp(0, 100_000);
                return Ok(Value::Void);
            }
            "get:globalFocusMode" => return Ok(Value::Integer(0)),
            "set:globalFocusMode" if arg(0)?.integer()? == 0 => return Ok(Value::Void),
            "set:globalFocusMode" => {
                return Err(unsupported("sound focus muting is not implemented"));
            }
            _ => {}
        }
        let Value::Object(reference) = context else {
            anyhow::bail!("WaveSoundBuffer requires an object context")
        };
        let id = reference
            .object
            .context("WaveSoundBuffer requires a non-null context")?;
        if operation == "@initialize" {
            self.sounds.entry(id).or_default();
            return Ok(Value::Void);
        }
        if operation == "@invalidate" {
            self.sounds.remove(&id);
            return Ok(Value::Void);
        }
        let sound = self
            .sounds
            .get_mut(&id)
            .context("context has no WaveSoundBuffer native instance")?;
        if operation == "WaveSoundBuffer" {
            let owner = arg(0)?;
            ensure!(
                matches!(owner, Value::Object(_)),
                "sound action owner must be an object"
            );
            sound.owner = owner.clone();
            sound.constructed = true;
            return Ok(Value::Void);
        }
        ensure!(sound.constructed, "WaveSoundBuffer constructor has not run");
        match operation {
            "get:volume" => return Ok(Value::Integer(sound.volume.into())),
            "get:volume2" => return Ok(Value::Integer(sound.volume2.into())),
            "get:pan" => return Ok(Value::Integer(sound.pan.into())),
            "get:paused" => return Ok(Value::Integer(sound.paused.into())),
            "get:looping" => return Ok(Value::Integer(sound.looping.into())),
            "get:status" => return Ok(Value::string("unload")),
            "get:position" | "get:samplePosition" | "get:bits" | "get:channels" => {
                return Ok(Value::Integer(0));
            }
            "set:volume" => sound.volume = (arg(0)?.integer()? as i32).clamp(0, 100_000),
            "set:volume2" => sound.volume2 = (arg(0)?.integer()? as i32).clamp(0, 100_000),
            "set:pan" => sound.pan = (arg(0)?.integer()? as i32).clamp(-100_000, 100_000),
            "set:paused" => sound.paused = arg(0)?.truth()?,
            "set:looping" => sound.looping = arg(0)?.truth()?,
            // Upstream's unloaded play/stop and totalTime setter do nothing.
            // open remains an explicit error, so no loaded stream is hidden.
            "play" | "stop" | "finalize" | "set:totalTime" => {}
            "stopFade" => {
                if let Some(fade) = sound.fade.take() {
                    sound.volume = fade.target.clamp(0, 100_000);
                    self.sound_event(vm, context, "onFadeCompleted", &[], budget)?;
                }
            }
            "fade" => {
                let target = arg(0)?.integer()? as i32;
                let duration = arg(1)?.integer()? as i32;
                let delay = args.get(2).map_or(Ok(0), Value::integer)? as i32;
                ensure!(
                    duration > 0 && delay >= 0,
                    "invalid sound fade duration or delay"
                );
                if sound.fade.take().is_some() {
                    self.sound_event(vm, context, "onFadeCompleted", &[], budget)?;
                }
                let sound = self
                    .sounds
                    .get_mut(&id)
                    .context("sound invalidated during fade callback")?;
                sound.fade = Some(Fade {
                    target,
                    delta: target
                        .wrapping_sub(sound.volume)
                        .wrapping_mul(BEAT_MS as i32)
                        / duration,
                    remaining: duration as u64 / BEAT_MS,
                    delay: delay as u64,
                });
                if duration < BEAT_MS as i32 && delay == 0 {
                    sound.fade = None;
                    sound.volume = target.clamp(0, 100_000);
                    self.sound_event(vm, context, "onFadeCompleted", &[], budget)?;
                }
            }
            "onStatusChanged" | "onFadeCompleted" | "onLabel" => {
                let owner = sound.owner.clone();
                if owner == Value::NULL {
                    return Ok(Value::Void);
                }
                let event = vm.new_dictionary()?;
                vm.set_member(&event, &Value::string("type"), Value::string(operation))?;
                let target = Value::Object(ObjectRef {
                    object: Some(id),
                    context: Some(id),
                });
                vm.set_member(&event, &Value::string("target"), target)?;
                if operation != "onFadeCompleted" {
                    vm.set_member(
                        &event,
                        &Value::string(if operation == "onLabel" {
                            "name"
                        } else {
                            "status"
                        }),
                        arg(0)?.clone(),
                    )?;
                }
                let action = vm.get_member(&owner, &Value::string("action"), true)?;
                if !matches!(action, Value::Void) {
                    return vm.call_function(&action, &owner, &[event], self, budget);
                }
            }
            _ => {
                return Err(unsupported(format!(
                    "WaveSoundBuffer.{operation} is not implemented"
                )));
            }
        }
        Ok(Value::Void)
    }

    pub(crate) fn sound_event(
        &mut self,
        vm: &mut Vm,
        sound: &Value,
        event: &str,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<()> {
        let callback = vm.get_member(sound, &Value::string(event), false)?;
        vm.call_function(&callback, sound, args, self, budget)?;
        Ok(())
    }

    pub(crate) fn sound_advance(&mut self, time_ms: u64) {
        let beats = time_ms / BEAT_MS - self.time_ms / BEAT_MS;
        for sound in self.sounds.values_mut() {
            sound.beat(beats);
        }
    }
}
