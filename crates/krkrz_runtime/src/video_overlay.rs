//! VideoOverlay object state before a movie is opened.
//! Follows visual/VideoOvlIntf.cpp and visual/win32/VideoOvlImpl.cpp.
//! Opening a movie requires the media backend and remains an explicit error.
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm, unsupported};

pub(crate) struct Video {
    owner: Value,
    bounds: [i32; 4],
    visible: bool,
    looping: bool,
    mode: i32,
    layers: [Value; 2],
    segment: [i32; 2],
    period: i32,
}
impl Default for Video {
    fn default() -> Self {
        Self {
            owner: Value::NULL,
            bounds: [0, 0, 320, 240],
            visible: false,
            looping: false,
            mode: 0,
            layers: [Value::NULL, Value::NULL],
            segment: [-1, -1],
            period: -1,
        }
    }
}

pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.register_native_class("VideoOverlay")?;
    for name in [
        "VideoOverlay",
        "finalize",
        "open",
        "play",
        "stop",
        "close",
        "setPos",
        "setSize",
        "setBounds",
        "pause",
        "rewind",
        "prepare",
        "setSegmentLoop",
        "cancelSegmentLoop",
        "setPeriodEvent",
        "cancelPeriodEvent",
        "selectAudioStream",
        "setMixingLayer",
        "resetMixingLayer",
        "onStatusChanged",
        "onCallbackCommand",
        "onPeriod",
        "onFrameUpdate",
    ] {
        vm.register_native(&format!("VideoOverlay.{name}"))?;
    }
    for name in [
        "position",
        "left",
        "top",
        "width",
        "height",
        "originalWidth",
        "originalHeight",
        "visible",
        "loop",
        "frame",
        "fps",
        "numberOfFrame",
        "totalTime",
        "layer1",
        "layer2",
        "mode",
        "playRate",
        "segmentLoopStartFrame",
        "segmentLoopEndFrame",
        "periodEventFrame",
        "audioBalance",
        "audioVolume",
        "numberOfAudioStream",
        "enabledAudioStream",
        "numberOfVideoStream",
        "enabledVideoStream",
        "mixingMovieAlpha",
        "mixingMovieBGColor",
    ] {
        let readonly = matches!(
            name,
            "originalWidth"
                | "originalHeight"
                | "fps"
                | "numberOfFrame"
                | "totalTime"
                | "segmentLoopStartFrame"
                | "segmentLoopEndFrame"
                | "numberOfAudioStream"
                | "numberOfVideoStream"
        );
        vm.register_native_property(
            &class,
            name,
            Some(&format!("VideoOverlay.get:{name}")),
            (!readonly)
                .then(|| format!("VideoOverlay.set:{name}"))
                .as_deref(),
        )?;
    }
    for kind in ["contrast", "brightness", "hue", "saturation"] {
        for suffix in ["", "RangeMin", "RangeMax", "DefaultValue", "StepSize"] {
            let name = format!("{kind}{suffix}");
            vm.register_native_property(
                &class,
                &name,
                Some(&format!("VideoOverlay.get:{name}")),
                suffix
                    .is_empty()
                    .then(|| format!("VideoOverlay.set:{name}"))
                    .as_deref(),
            )?;
        }
    }
    Ok(())
}
impl Services {
    pub(crate) fn video_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let Value::Object(reference) = context else {
            anyhow::bail!("VideoOverlay requires an object context")
        };
        let id = reference
            .object
            .context("VideoOverlay requires a non-null context")?;
        if operation == "@initialize" {
            self.videos.entry(id).or_default();
            return Ok(Value::Void);
        }
        if operation == "@invalidate" {
            self.videos.remove(&id);
            return Ok(Value::Void);
        }
        ensure!(
            self.videos.contains_key(&id),
            "context has no VideoOverlay native instance"
        );
        let arg = |i| args.get(i).context("VideoOverlay: missing argument");
        if operation == "VideoOverlay" {
            let owner = arg(0)?.clone();
            let Value::Object(window) = &owner else {
                anyhow::bail!("VideoOverlay requires a Window owner")
            };
            ensure!(
                window
                    .object
                    .is_some_and(|id| self.windows.contains_key(&id)),
                "VideoOverlay requires a Window owner"
            );
            self.videos.get_mut(&id).unwrap().owner = owner;
            return Ok(Value::Void);
        }
        if operation == "open" {
            let storage = arg(0)?.text();
            let Value::Object(window) = &self.videos[&id].owner else {
                unreachable!()
            };
            ensure!(
                window
                    .object
                    .is_some_and(|id| self.windows.contains_key(&id)),
                "VideoOverlay owner window is missing"
            );
            let name = storage.split('?').next().unwrap_or(&storage);
            // Missing storage is an ordinary script exception. A valid movie
            // must not silently disappear behind KAG's fallback exception handler.
            self.read_storage(name)?;
            return Err(unsupported(format!(
                "VideoOverlay movie decoding is not implemented: {storage}"
            )));
        }
        if let Some(event) = operation.strip_prefix("on") {
            let names: &[&str] = match event {
                "StatusChanged" => &["status"],
                "CallbackCommand" => &["command", "arg"],
                "Period" => &["reason"],
                "FrameUpdate" => &["frame"],
                _ => anyhow::bail!("unknown video event: {operation}"),
            };
            let owner = self.videos[&id].owner.clone();
            if owner == Value::NULL {
                return Ok(Value::Void);
            }
            ensure!(
                args.len() >= names.len(),
                "video event has missing arguments"
            );
            let event = vm.new_dictionary()?;
            vm.set_member(&event, &Value::string("type"), Value::string(operation))?;
            vm.set_member(
                &event,
                &Value::string("target"),
                Value::Object(krkrz_tjs::ObjectRef {
                    object: Some(id),
                    context: Some(id),
                }),
            )?;
            for (name, value) in names.iter().zip(args) {
                vm.set_member(&event, &Value::string(name), value.clone())?;
            }
            let action =
                vm.get_property(&owner, &Value::string("action"), true, false, self, budget)?;
            if !matches!(action, Value::Void) {
                vm.call_function(&action, &owner, &[event], self, budget)?;
            }
            return Ok(Value::Void);
        }
        if matches!(operation, "set:layer1" | "set:layer2" | "setMixingLayer") {
            let layer = arg(0)?.clone();
            let Value::Object(reference) = &layer else {
                anyhow::bail!("video target must be a Layer or null")
            };
            ensure!(
                reference
                    .object
                    .is_none_or(|id| self.layers.contains_key(&id)),
                "video target must be a Layer or null"
            );
            if operation != "setMixingLayer" {
                self.videos.get_mut(&id).unwrap().layers[usize::from(operation.ends_with('2'))] =
                    layer;
            }
            return Ok(Value::Void);
        }
        let video = self.videos.get_mut(&id).unwrap();
        let color_control = |name: &str| {
            ["contrast", "brightness", "hue", "saturation"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        };
        if let Some(name) = operation.strip_prefix("get:") {
            return Ok(match name {
                "left" => Value::Integer(video.bounds[0].into()),
                "top" => Value::Integer(video.bounds[1].into()),
                "width" => Value::Integer(video.bounds[2].into()),
                "height" => Value::Integer(video.bounds[3].into()),
                "visible" => Value::Integer(video.visible.into()),
                "loop" => Value::Integer(video.looping.into()),
                "mode" => Value::Integer(video.mode.into()),
                "layer1" | "layer2" => {
                    let layer = &video.layers[usize::from(name.ends_with('2'))];
                    if let Value::Object(reference) = layer
                        && reference
                            .object
                            .is_some_and(|id| !self.layers.contains_key(&id))
                    {
                        Value::NULL
                    } else {
                        layer.clone()
                    }
                }
                "segmentLoopStartFrame" => Value::Integer(video.segment[0].into()),
                "segmentLoopEndFrame" => Value::Integer(video.segment[1].into()),
                "periodEventFrame" => Value::Integer(video.period.into()),
                "enabledAudioStream" | "enabledVideoStream" => Value::Integer(-1),
                "audioVolume" => Value::Integer(100_000),
                "fps" | "playRate" | "mixingMovieAlpha" => Value::Real(0.0),
                "position"
                | "frame"
                | "numberOfFrame"
                | "totalTime"
                | "originalWidth"
                | "audioBalance"
                | "numberOfAudioStream"
                | "numberOfVideoStream" => Value::Integer(0),
                "originalHeight" => {
                    anyhow::bail!("video height is unavailable before opening a movie")
                }
                // The old engine exposes uninitialized memory here. Return an
                // explicit error instead of a nondeterministic machine value.
                "mixingMovieBGColor" => {
                    anyhow::bail!("movie background color is unavailable before opening a movie")
                }
                _ if color_control(name) => Value::Real(-1.0),
                _ => anyhow::bail!("unknown VideoOverlay property: {name}"),
            });
        }
        if video.mode == 1 && matches!(operation, "set:left" | "set:top" | "setPos") {
            arg(0)?.integer()?;
            if operation == "setPos" {
                arg(1)?.integer()?;
            }
            let layers = video.layers.clone();
            let layer_operation = if operation == "setPos" {
                "setPos"
            } else {
                operation
            };
            // Native methods operate on Layer instances, bypassing script overrides.
            for layer in layers {
                if layer != Value::NULL {
                    self.layer_call(vm, layer_operation, &layer, args, budget)?;
                }
            }
            return Ok(Value::Void);
        }
        match operation {
            "set:left" => video.bounds[0] = arg(0)?.integer()? as i32,
            "set:top" => video.bounds[1] = arg(0)?.integer()? as i32,
            "set:width" | "set:height" => {
                let n = arg(0)?.integer()? as i32;
                if video.mode != 1 {
                    video.bounds[if operation == "set:width" { 2 } else { 3 }] = n;
                }
            }
            "setPos" => {
                video.bounds[0] = arg(0)?.integer()? as i32;
                video.bounds[1] = arg(1)?.integer()? as i32;
            }
            "setSize" | "setBounds" => {
                let count = if operation == "setSize" { 2 } else { 4 };
                let values: Vec<i32> = (0..count)
                    .map(|i| Ok(arg(i)?.integer()? as i32))
                    .collect::<Result<_>>()?;
                if video.mode != 1 {
                    video.bounds[4 - count..].copy_from_slice(&values);
                }
            }
            "set:visible" => video.visible = arg(0)?.truth()?,
            "set:loop" => video.looping = arg(0)?.truth()?,
            "set:mode" => video.mode = arg(0)?.integer()? as i32,
            "set:periodEventFrame" => video.period = arg(0)?.integer()? as i32,
            "setPeriodEvent" => match args.len() {
                0 => video.period = -1,
                1 => video.period = arg(0)?.integer()? as i32,
                _ => {}
            },
            "cancelPeriodEvent" => video.period = -1,
            "setSegmentLoop" => {
                video.segment = [arg(0)?.integer()? as i32, arg(1)?.integer()? as i32]
            }
            "cancelSegmentLoop" => video.segment = [-1, -1],
            "set:playRate"
            | "set:mixingMovieAlpha"
            | "set:contrast"
            | "set:brightness"
            | "set:hue"
            | "set:saturation" => {
                arg(0)?.real()?;
            }
            "set:position"
            | "set:frame"
            | "set:audioBalance"
            | "set:audioVolume"
            | "set:enabledAudioStream"
            | "set:enabledVideoStream"
            | "set:mixingMovieBGColor"
            | "selectAudioStream" => {
                arg(0)?.integer()?;
            }
            // These native operations have no effect while no movie is open.
            "finalize" | "play" | "stop" | "close" | "pause" | "rewind" | "prepare"
            | "resetMixingLayer" => {}
            _ => anyhow::bail!("unknown VideoOverlay operation: {operation}"),
        }
        Ok(Value::Void)
    }
}
