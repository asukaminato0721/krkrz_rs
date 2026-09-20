//! Bounded decoded sources for WaveSoundBuffer and deterministic host pulls.
use crate::{
    Services, Session, audio::AudioBlock, audio::SoundStream, save_storage, sound::Loaded,
};
use anyhow::{Context, Result, ensure};
use krkrz_assets::{media::Audio, sli::LoopInfo, text};
use krkrz_tjs::{Value, Vm, unsupported};
use std::sync::Arc;

impl Services {
    pub(crate) fn sound_status(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        id: usize,
        status: &'static str,
        budget: &mut u64,
    ) -> Result<()> {
        let sound = self
            .sounds
            .get_mut(&id)
            .context("sound invalidated during callback")?;
        if sound.status != status {
            sound.status = status;
            self.sound_event(
                vm,
                context,
                "onStatusChanged",
                &[Value::string(status)],
                budget,
            )?;
        }
        Ok(())
    }
    pub(crate) fn sound_stop(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        id: usize,
        budget: &mut u64,
    ) -> Result<()> {
        let sound = self
            .sounds
            .get_mut(&id)
            .context("sound invalidated during callback")?;
        sound.generation += 1;
        sound.events.clear();
        if sound.status != "unload" {
            self.sound_status(vm, context, id, "stop", budget)?;
        }
        // The original rewinds after the status callback.
        if let Some(sound) = self.sounds.get_mut(&id)
            && let Some(loaded) = &mut sound.loaded
        {
            loaded.stream.seek(0)?;
            loaded.pipeline.reset();
            loaded.mixer = Default::default();
            loaded.ended = false;
        }
        Ok(())
    }
    pub(crate) fn sound_open(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        id: usize,
        storage: &str,
        budget: &mut u64,
    ) -> Result<Value> {
        self.sound_stop(vm, context, id, budget)?;
        let sound = self
            .sounds
            .get_mut(&id)
            .context("sound invalidated during open callback")?;
        sound.loaded = None;
        sound.paused = false;
        self.sound_status(vm, context, id, "unload", budget)?;
        ensure!(
            self.sounds.contains_key(&id),
            "sound invalidated during open callback"
        );
        let allocated: usize = self
            .sounds
            .values()
            .filter_map(|s| s.loaded.as_ref())
            .map(|s| s.audio.samples.len() * 2)
            .sum();
        // Use the maximum supported channel count when setting a frame limit,
        // so decoding cannot allocate more than the session's remaining budget.
        let max_frames = ((256usize << 20).saturating_sub(allocated) / 16).min(32_000_000);
        let bytes = self.read_storage(storage)?;
        let audio = if bytes.starts_with(b"RIFF") {
            Audio::decode_wave(&bytes, max_frames)?
        } else if bytes.starts_with(b"OggS") {
            Audio::decode_vorbis(&bytes, max_frames)?
        } else {
            return Err(unsupported(format!("unsupported sound format: {storage}")));
        };
        ensure!(audio.channels <= 8, "sound channel count exceeds limit");
        let sli = format!("{storage}.sli");
        let exists = save_storage::path(&self.storage.project, &self.save_dir, &sli)
            .is_ok_and(|p| p.is_file())
            || self.storage.resolve(&sli).is_ok();
        let info = if exists {
            LoopInfo::parse(&text::decode(&self.read_storage(&sli)?)?)?
        } else {
            LoopInfo::default()
        };
        let pipeline = self.connect_sound_filters(vm, id, audio.channels as usize, budget)?;
        let audio = Arc::new(audio);
        let mut stream = SoundStream::new(audio.clone(), info)?;
        let sound = self
            .sounds
            .get_mut(&id)
            .context("sound invalidated during open callback")?;
        stream.loop_at_end = sound.looping;
        sound.frequency = audio.sample_rate as i32;
        sound.loaded = Some(Loaded {
            audio,
            stream,
            pipeline,
            ended: false,
            mixer: Default::default(),
        });
        // WaveImpl recreates this dictionary only when an SLI file was read.
        if exists && let Some(labels) = sound.labels_object.take() {
            vm.invalidate(&labels, self, budget)?;
        }
        self.sound_status(vm, context, id, "stop", budget)?;
        Ok(Value::Void)
    }
    pub(crate) fn sound_visualization(
        &mut self,
        id: usize,
        handle: i64,
        frames: i32,
        channels: i32,
        ahead: i32,
    ) -> Result<Value> {
        self.validate_sound_filter_memory()?;
        let sound = &self.sounds[&id];
        let Some(loaded) = &sound.loaded else {
            return Ok(Value::Integer(0));
        };
        if !sound.use_vis_buffer
            || sound.status != "play"
            || sound.paused
            || loaded.ended
            || frames <= 0
            || (channels != 1 && channels != loaded.audio.channels as i32)
        {
            return Ok(Value::Integer(0));
        }
        if ahead < 0 {
            return Err(unsupported(
                "negative visualization offsets require audio history",
            ));
        }
        ensure!(
            frames <= 1_000_000 && ahead <= 1_000_000,
            "visualization frame limit exceeded"
        );
        let dest = self
            .sample_plugin
            .buffers
            .get_mut(&handle)
            .context("visualization destination is not an active sample buffer")?;
        ensure!(
            dest.len() >= frames as usize * channels as usize,
            "visualization destination is too small"
        );
        let mut preview = loaded.stream.clone();
        let mut pipeline = loaded.pipeline.clone();
        pipeline.render(&mut preview, ahead as usize)?;
        let block = pipeline.render(&mut preview, frames as usize)?;
        let source_channels = loaded.audio.channels as usize;
        let written = block.samples.len() / source_channels;
        if channels == 1 {
            for (out, frame) in dest
                .iter_mut()
                .zip(block.samples.chunks_exact(source_channels))
            {
                *out =
                    (frame.iter().map(|v| *v as i32).sum::<i32>() / source_channels as i32) as i16;
            }
        } else {
            dest[..block.samples.len()].copy_from_slice(&block.samples);
        }
        // Upstream's output ring contains silence past EOF. We retain the same
        // padding for positive preview ranges without exposing raw pointers.
        dest[written * channels as usize..frames as usize * channels as usize].fill(0);
        Ok(Value::Integer(if written == 0 { 0 } else { frames.into() }))
    }
}

impl Session {
    /// Pull unscaled PCM at the source rate. This advances the source only;
    /// device mixing, gain, resampling, and clock synchronization are host work.
    /// Labels and EOF are delivered by `dispatch_events`, never during the pull.
    pub fn render_sound_source(&mut self, value: &Value, frames: usize) -> Result<AudioBlock> {
        ensure!(frames <= 1_000_000, "audio block exceeds frame limit");
        self.services.validate_sound_filter_memory()?;
        let Value::Object(reference) = value else {
            anyhow::bail!("sound must be an object");
        };
        let id = reference.object.context("sound must not be null")?;
        let sound = self
            .services
            .sounds
            .get_mut(&id)
            .context("object has no sound instance")?;
        if sound.status != "play" || sound.paused {
            return Ok(AudioBlock::default());
        }
        let loaded = sound
            .loaded
            .as_mut()
            .context("playing sound has no source")?;
        if loaded.ended {
            return Ok(AudioBlock::default());
        }
        let block = loaded.pipeline.render(&mut loaded.stream, frames)?;
        ensure!(
            sound.events.len() + block.labels.len() < 65_536,
            "sound event queue limit exceeded"
        );
        for label in &block.labels {
            sound
                .events
                .push((sound.generation, "onLabel", Value::string(&label.name)));
        }
        if block.samples.len() / (loaded.audio.channels as usize) < frames {
            loaded.ended = true;
            sound
                .events
                .push((sound.generation, "onStatusChanged", Value::string("stop")));
        }
        Ok(block)
    }
}
