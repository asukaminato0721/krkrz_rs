//! VideoOverlay state, built-in MPEG decoding, and session-clock playback.
//! Follows visual/VideoOvlIntf.cpp and visual/win32/VideoOvlImpl.cpp.
//! Frames and PCM share the host clock used by timers and sound buffers.
use crate::{Services, Session};
#[path = "movie_decoder.rs"]
mod decoder;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm, unsupported};

pub(crate) struct Video {
    owner: Value,
    movie: Option<decoder::Movie>,
    status: &'static str,
    position: f64,
    rate: f64,
    volume: i32,
    balance: i32,
    generation: u64,
    events: Vec<(u64, &'static str, Value)>,
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
            movie: None,
            status: "unload",
            position: 0.0,
            rate: 1.0,
            volume: 100_000,
            balance: 0,
            generation: 0,
            events: vec![],
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
            let bytes = self.read_storage(name)?;
            let mut movie = decoder::Movie::open(&bytes)
                .map_err(|e| unsupported(format!("VideoOverlay {storage}: {e:#}")))?;
            movie.frame(0)?;
            let video = self.videos.get_mut(&id).unwrap();
            video.generation += 1;
            video.movie = Some(movie);
            video.position = 0.0;
            video.status = "stop";
            video
                .events
                .push((video.generation, "onStatusChanged", Value::string("stop")));
            self.video_present(id)?;
            self.video_dispatch_native_events(vm, id, budget)?;
            return Ok(Value::Void);
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
        if operation == "setMixingLayer" && self.videos[&id].movie.is_some() {
            return Err(unsupported(
                "VideoOverlay mixing layers are not implemented",
            ));
        }
        if operation == "set:visible"
            && self.videos[&id].mode == 1
            && self.videos[&id].movie.is_some()
        {
            for layer in self.videos[&id].layers.clone() {
                if layer != Value::NULL {
                    self.layer_call(vm, operation, &layer, args, budget)?;
                }
            }
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
                self.video_present(id)?;
            }
            return Ok(Value::Void);
        }
        if self.videos[&id].movie.is_some() {
            let transport = matches!(
                operation,
                "play" | "pause" | "stop" | "close" | "finalize" | "prepare"
            );
            if transport {
                self.videos.get_mut(&id).unwrap().events.clear();
            }
            if let Some(value) = self.video_loaded_call(id, operation, args)? {
                if matches!(
                    operation,
                    "set:position" | "set:frame" | "rewind" | "prepare"
                ) {
                    self.video_present(id)?;
                }
                if transport {
                    self.video_dispatch_native_events(vm, id, budget)?;
                }
                return Ok(value);
            }
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

impl Video {
    fn event(&mut self, name: &'static str, argument: Value) {
        self.events.push((self.generation, name, argument));
    }
    fn status(&mut self, status: &'static str) {
        if self.status != status {
            self.status = status;
            self.event("onStatusChanged", Value::string(status));
        }
    }
    fn seek(&mut self, position: f64) -> Result<()> {
        let movie = self.movie.as_mut().unwrap();
        self.position = position.clamp(0.0, movie.duration_ms);
        movie.frame((self.position * movie.fps / 1000.0).floor() as u64)?;
        Ok(())
    }
    pub(crate) fn has_audio(&self) -> bool {
        self.status == "play" && self.movie.as_ref().is_some_and(|m| !m.audio.is_empty())
    }
    pub(crate) fn mix_audio(&self, output: &mut [f32]) {
        if !self.has_audio() {
            return;
        }
        let movie = self.movie.as_ref().unwrap();
        let volume = self.volume as f32 / 100_000.0;
        let gain = [
            volume * (1.0 - self.balance.max(0) as f32 / 100_000.0),
            volume * (1.0 + self.balance.min(0) as f32 / 100_000.0),
        ];
        for (i, frame) in output.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let mut position = self.position + i as f64 / 48.0 * self.rate;
            if self.segment[1] > 0 {
                let end = self.segment[1] as f64 * 1000.0 / movie.fps;
                let start = self.segment[0].max(0) as f64 * 1000.0 / movie.fps;
                if position >= end && end > start {
                    position = start + (position - end) % (end - start);
                }
            } else if self.looping {
                position %= movie.duration_ms;
            }
            for channel in 0..2 {
                frame[channel] += movie.audio_sample(position, channel) * gain[channel];
            }
        }
    }
    /// Window overlay frames are composed by the same host as Layer surfaces.
    pub(crate) fn overlay(&self, window: usize) -> Option<(&krkrz_assets::media::Image, [i32; 4])> {
        if self.mode == 1
            || !self.visible
            || !matches!(&self.owner, Value::Object(reference) if reference.object == Some(window))
        {
            return None;
        }
        self.movie
            .as_ref()?
            .image
            .as_ref()
            .map(|image| (image, self.bounds))
    }
}
impl Services {
    fn video_dispatch_native_events(
        &mut self,
        vm: &mut Vm,
        id: usize,
        budget: &mut u64,
    ) -> Result<()> {
        let events = std::mem::take(&mut self.videos.get_mut(&id).unwrap().events);
        for (generation, name, argument) in events {
            if !self
                .videos
                .get(&id)
                .is_some_and(|v| v.generation == generation)
            {
                break;
            }
            let target = Value::object(id);
            let callback = vm.get_member(&target, &Value::string(name), false)?;
            vm.call_function(&callback, &target, &[argument], self, budget)?;
        }
        Ok(())
    }
    fn video_loaded_call(
        &mut self,
        id: usize,
        operation: &str,
        args: &[Value],
    ) -> Result<Option<Value>> {
        let video = self.videos.get_mut(&id).unwrap();
        let arg = || args.first().context("VideoOverlay: missing argument");
        let movie = video.movie.as_ref().unwrap();
        let value = match operation {
            "get:position" => Value::Integer(video.position as i64),
            "get:frame" => Value::Integer(
                (video.position * movie.fps / 1000.0)
                    .floor()
                    .min((movie.frames - 1) as f64) as i64,
            ),
            "get:originalWidth" => Value::Integer(movie.width.into()),
            "get:originalHeight" => Value::Integer(movie.height.into()),
            "get:fps" => Value::Real(movie.fps),
            "get:numberOfFrame" => Value::Integer(movie.frames as i64),
            "get:totalTime" => Value::Integer(movie.duration_ms as i64),
            "get:playRate" => Value::Real(video.rate),
            "get:audioVolume" => Value::Integer(video.volume.into()),
            "get:audioBalance" => Value::Integer(video.balance.into()),
            "get:numberOfAudioStream" => Value::Integer((!movie.audio.is_empty()).into()),
            "get:enabledAudioStream" => Value::Integer(if movie.audio.is_empty() { -1 } else { 0 }),
            "get:numberOfVideoStream" => Value::Integer(1),
            "get:enabledVideoStream" => Value::Integer(0),
            "set:position" => {
                video.seek(arg()?.integer()? as f64)?;
                Value::Void
            }
            "set:frame" => {
                video.seek(arg()?.integer()? as f64 * 1000.0 / movie.fps)?;
                Value::Void
            }
            "set:playRate" => {
                let rate = arg()?.real()?;
                ensure!(
                    rate.is_finite() && rate > 0.0 && rate <= 16.0,
                    "unsupported movie playback rate"
                );
                video.rate = rate;
                Value::Void
            }
            "set:audioVolume" => {
                video.volume = arg()?.integer()?.clamp(0, 100_000) as i32;
                Value::Void
            }
            "set:audioBalance" => {
                video.balance = arg()?.integer()?.clamp(-100_000, 100_000) as i32;
                Value::Void
            }
            "set:mode" => {
                arg()?.integer()?;
                Value::Void
            }
            "play" => {
                video.status("play");
                Value::Void
            }
            "pause" => {
                video.status("pause");
                Value::Void
            }
            "stop" => {
                video.status("stop");
                Value::Void
            }
            "rewind" => {
                video.seek(0.0)?;
                Value::Void
            }
            "prepare" => {
                video.seek(0.0)?;
                video.event("onFrameUpdate", Value::Integer(0));
                video.event("onPeriod", Value::Integer(2));
                video.status("pause");
                Value::Void
            }
            "close" | "finalize" => {
                video.generation += 1;
                video.events.clear();
                video.movie = None;
                video.position = 0.0;
                video.status("unload");
                Value::Void
            }
            "set:enabledAudioStream" | "selectAudioStream" => {
                ensure!(
                    arg()?.integer()? == 0 && !movie.audio.is_empty(),
                    "unsupported movie audio stream selection"
                );
                Value::Void
            }
            "set:enabledVideoStream" => {
                ensure!(
                    arg()?.integer()? == 0,
                    "unsupported movie video stream selection"
                );
                Value::Void
            }
            "setMixingLayer"
            | "resetMixingLayer"
            | "set:mixingMovieAlpha"
            | "set:mixingMovieBGColor"
            | "set:contrast"
            | "set:brightness"
            | "set:hue"
            | "set:saturation" => return Err(unsupported(format!("VideoOverlay {operation}"))),
            _ => return Ok(None),
        };
        Ok(Some(value))
    }
    fn video_present(&mut self, id: usize) -> Result<()> {
        let video = &self.videos[&id];
        if video.mode != 1 {
            return Ok(());
        }
        let Some(image) = video.movie.as_ref().and_then(|m| m.image.as_ref()) else {
            return Ok(());
        };
        for value in &video.layers {
            if let Value::Object(reference) = value
                && let Some(target) = reference.object.filter(|id| self.layers.contains_key(id))
            {
                let available = crate::layer::image_available(&self.layers, target);
                self.layers
                    .get_mut(&target)
                    .unwrap()
                    .set_movie_image(image, available)?;
            }
        }
        Ok(())
    }
    pub(crate) fn video_advance(&mut self, time_ms: u64) -> Result<()> {
        let delta = time_ms.saturating_sub(self.time_ms) as f64;
        let ids: Vec<_> = self.videos.keys().copied().collect();
        for id in ids {
            let video = self.videos.get_mut(&id).unwrap();
            if video.status != "play" {
                continue;
            }
            let Some(movie) = &video.movie else {
                continue;
            };
            let duration = movie.duration_ms;
            let fps = movie.fps;
            let old_frame = movie.image_frame;
            let mut position = video.position + delta * video.rate;
            let segment_end = video.segment[1] as f64 * 1000.0 / fps;
            let segment_start = video.segment[0].max(0) as f64 * 1000.0 / fps;
            if video.segment[1] > 0 && segment_end > segment_start && position >= segment_end {
                position = segment_start + (position - segment_end) % (segment_end - segment_start);
                video.event("onPeriod", Value::Integer(3));
            } else if position >= duration && video.looping {
                position %= duration;
                video.event("onPeriod", Value::Integer(0));
            }
            let ended = position >= duration;
            video.seek(position)?;
            let frame = video.movie.as_ref().unwrap().image_frame.unwrap();
            if old_frame != Some(frame) {
                video.event("onFrameUpdate", Value::Integer(frame as i64));
            }
            if video.period >= 0 && frame >= video.period as u64 {
                video.period = -1;
                video.event("onPeriod", Value::Integer(1));
            }
            if ended {
                video.status("stop");
            }
            self.video_present(id)?;
        }
        Ok(())
    }
}
impl Session {
    pub(crate) fn dispatch_video_events(&mut self) -> Result<()> {
        let pending: Vec<_> = self
            .services
            .videos
            .iter_mut()
            .map(|(id, v)| (*id, std::mem::take(&mut v.events)))
            .collect();
        for (id, events) in pending {
            for (generation, name, value) in events {
                if !self
                    .services
                    .videos
                    .get(&id)
                    .is_some_and(|v| v.generation == generation)
                {
                    break;
                }
                let target = Value::object(id);
                let callback = self.vm.get_member(&target, &Value::string(name), false)?;
                self.vm.call_function(
                    &callback,
                    &target,
                    &[value],
                    &mut self.services,
                    &mut self.budget,
                )?;
            }
        }
        Ok(())
    }
}

impl Video {
    pub(crate) fn gc_trace(&self, out: &mut Vec<Value>) {
        out.push(self.owner.clone());
        out.extend(self.layers.iter().cloned());
        out.extend(self.events.iter().map(|(_, _, value)| value.clone()));
    }
}
