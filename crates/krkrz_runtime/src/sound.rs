//! Sound control and fade timing from Kirikiri SoundBufferBaseIntf.cpp.
//! Decoding and source-rate pulls share the headless session; device mixing is separate.
use crate::{Services, audio::SoundStream};
use anyhow::{Context, Result, ensure};
use krkrz_assets::media::Audio;
use krkrz_tjs::{ObjectRef, Value, Vm, unsupported};
use std::sync::Arc;

const BEAT_MS: u64 = 60;

struct Fade {
    target: i32,
    delta: i32,
    remaining: u64,
    delay: u64,
}

pub(crate) struct Loaded {
    pub audio: Arc<Audio>,
    pub stream: SoundStream,
    pub pipeline: crate::phase_vocoder::Pipeline,
    pub ended: bool,
    pub mixer: crate::audio_mixer::Source,
}
pub(crate) struct Sound {
    pub(crate) loaded: Option<Loaded>,
    pub(crate) status: &'static str,
    pub(crate) generation: u64,
    pub(crate) events: Vec<(u64, &'static str, Value)>,
    output_rate: u32,
    output_channels: u8,
    pub(crate) frequency: i32,
    owner: Value,
    pub(crate) filters: Value,
    flags_object: Option<Value>,
    pub(crate) labels_object: Option<Value>,
    constructed: bool,
    pub(crate) volume: i32,
    pub(crate) volume2: i32,
    pub(crate) pan: i32,
    pub(crate) paused: bool,
    pub(crate) looping: bool,
    pub(crate) use_vis_buffer: bool,
    fade: Option<Fade>,
    pub(crate) fade_pending: bool,
}

impl Default for Sound {
    fn default() -> Self {
        Self {
            loaded: None,
            status: "unload",
            generation: 0,
            events: vec![],
            output_rate: 0,
            output_channels: 0,
            frequency: 0,
            owner: Value::NULL,
            filters: Value::Void,
            flags_object: None,
            labels_object: None,
            constructed: false,
            volume: 100_000,
            volume2: 100_000,
            pan: 0,
            paused: false,
            looping: false,
            use_vis_buffer: false,
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
        "getVisBuffer",
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
        ("useVisBuffer", true),
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
        if matches!(
            operation,
            "getSample"
                | "setDefaultCounts"
                | "setDefaultAheads"
                | "get:sampleCount"
                | "set:sampleCount"
                | "get:sampleAhead"
                | "set:sampleAhead"
                | "get:sampleValue"
        ) {
            return self.sample_call(vm, operation, context, args, budget, true);
        }
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
            if let std::collections::btree_map::Entry::Vacant(entry) = self.sounds.entry(id) {
                let filters = vm.new_native_array(vec![])?;
                entry.insert(Sound {
                    filters,
                    ..Sound::default()
                });
            }
            return Ok(Value::Void);
        }
        if operation == "@invalidate" {
            if let Some(sound) = self.sounds.remove(&id)
                && let Some(labels) = sound.labels_object
            {
                vm.invalidate(&labels, self, budget)?;
            }
            self.sample_plugin.instances.remove(&id);
            return Ok(Value::Void);
        }
        ensure!(
            self.sounds.contains_key(&id),
            "context has no WaveSoundBuffer native instance"
        );
        if operation == "open" {
            ensure!(
                self.sounds[&id].constructed,
                "WaveSoundBuffer constructor has not run"
            );
            let storage = arg(0)?.text();
            return self.sound_open(vm, context, id, &storage, budget);
        }
        if operation == "get:flags" {
            if let Some(value) = &self.sounds[&id].flags_object {
                return Ok(value.clone());
            }
            let class = self.wave_flags_class.clone();
            let value = vm.construct(&class, std::slice::from_ref(context), self, budget)?;
            self.sounds.get_mut(&id).unwrap().flags_object = Some(value.clone());
            return Ok(value);
        }
        if operation == "get:labels" {
            if let Some(value) = &self.sounds[&id].labels_object {
                return Ok(value.clone());
            }
            let value = vm.new_dictionary()?;
            if let Some(loaded) = &self.sounds[&id].loaded {
                for label in loaded.stream.labels() {
                    *budget = budget
                        .checked_sub(1)
                        .ok_or_else(|| unsupported("sound label execution budget exceeded"))?;
                    let item = vm.new_dictionary()?;
                    for (key, val) in [
                        ("name", Value::string(&label.name)),
                        ("samplePosition", Value::Integer(label.position as i64)),
                        (
                            "position",
                            Value::Integer(
                                (label.position * 1000 / loaded.audio.sample_rate as u64) as i64,
                            ),
                        ),
                    ] {
                        vm.set_member(&item, &Value::string(key), val)?;
                    }
                    if !label.name.is_empty() {
                        vm.set_member(&value, &Value::string(&label.name), item)?;
                    }
                }
            }
            self.sounds.get_mut(&id).unwrap().labels_object = Some(value.clone());
            return Ok(value);
        }
        let sound = self.sounds.get_mut(&id).unwrap();
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
            "get:filters" => return Ok(sound.filters.clone()),
            "get:volume" => return Ok(Value::Integer(sound.volume.into())),
            "get:volume2" => return Ok(Value::Integer(sound.volume2.into())),
            "get:pan" => return Ok(Value::Integer(sound.pan.into())),
            "get:paused" => return Ok(Value::Integer(sound.paused.into())),
            "get:looping" => return Ok(Value::Integer(sound.looping.into())),
            "get:useVisBuffer" => return Ok(Value::Integer(sound.use_vis_buffer.into())),
            "set:useVisBuffer" => sound.use_vis_buffer = arg(0)?.truth()?,
            "getVisBuffer" => {
                ensure!(args.len() >= 3, "getVisBuffer requires three arguments");
                let handle = arg(0)?.integer()?;
                let ahead = args.get(3).map_or(Ok(0), Value::integer)? as i32;
                let frames = arg(1)?.integer()? as i32;
                let channels = arg(2)?.integer()? as i32;
                return self.sound_visualization(id, handle, frames, channels, ahead);
            }
            "get:status" => return Ok(Value::string(sound.status)),
            "get:frequency" => return Ok(Value::Integer(sound.frequency.into())),
            "set:frequency" => {
                let frequency = arg(0)?.integer()? as i32;
                ensure!(
                    frequency > 0 && frequency <= 768_000,
                    "sound frequency exceeds supported range"
                );
                sound.frequency = frequency;
            }
            "get:bits" => return Ok(Value::Integer(if sound.output_rate == 0 { 0 } else { 16 })),
            "get:channels" => return Ok(Value::Integer(sound.output_channels.into())),
            "get:position" | "get:samplePosition" => {
                let position = if sound.output_rate == 0 {
                    0
                } else {
                    sound
                        .loaded
                        .as_ref()
                        .map_or(0, |s| s.pipeline.position(&s.stream))
                };
                return Ok(Value::Integer(
                    if operation == "get:position" && sound.output_rate != 0 {
                        (position * 1000 / sound.output_rate as u64) as i64
                    } else {
                        position as i64
                    },
                ));
            }
            "get:totalTime" => {
                // The original can divide by zero before creating an output buffer.
                ensure!(
                    sound.output_rate != 0,
                    "sound output format is not initialized"
                );
                let frames = sound.loaded.as_ref().map_or(0, |s| s.audio.frames());
                return Ok(Value::Integer(
                    (frames as u64 * 1000 / sound.output_rate as u64) as i64,
                ));
            }
            "set:position" | "set:samplePosition" => {
                let mut position = arg(0)?.integer()? as u64;
                if operation == "set:position" {
                    position = position.wrapping_mul(sound.output_rate as u64) / 1000;
                }
                let loaded = sound
                    .loaded
                    .as_mut()
                    .context("cannot seek an unloaded sound")?;
                if position < loaded.audio.frames() as u64 {
                    loaded.stream.seek(position)?;
                    loaded.pipeline.reset();
                    loaded.mixer = Default::default();
                    loaded.ended = false;
                    sound.generation += 1;
                    sound.events.clear();
                }
            }
            "set:volume" => sound.volume = (arg(0)?.integer()? as i32).clamp(0, 100_000),
            "set:volume2" => sound.volume2 = (arg(0)?.integer()? as i32).clamp(0, 100_000),
            "set:pan" => sound.pan = (arg(0)?.integer()? as i32).clamp(-100_000, 100_000),
            "set:paused" => sound.paused = arg(0)?.truth()?,
            "set:looping" => {
                sound.looping = arg(0)?.truth()?;
                if let Some(loaded) = &mut sound.loaded {
                    loaded.stream.loop_at_end = sound.looping;
                }
            }
            "play" => {
                if sound.status != "play"
                    && let Some(loaded) = &mut sound.loaded
                {
                    sound.output_rate = loaded.audio.sample_rate;
                    sound.output_channels = loaded.audio.channels;
                    loaded.ended = false;
                    sound.generation += 1;
                    sound.events.clear();
                    self.sound_status(vm, context, id, "play", budget)?;
                }
            }
            "stop" => self.sound_stop(vm, context, id, budget)?,
            "finalize" | "set:totalTime" => {}
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

impl Sound {
    pub(crate) fn gc_trace(&self, out: &mut Vec<Value>) {
        out.extend([self.owner.clone(), self.filters.clone()]);
        out.extend(self.flags_object.iter().cloned());
        out.extend(self.labels_object.iter().cloned());
        out.extend(self.events.iter().map(|(_, _, value)| value.clone()));
    }
}
