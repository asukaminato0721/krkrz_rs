//! Shared host-clock audio: stereo 48 kHz PCM and deferred native label events.
//! Source-rate pulls remain available for decoder tools; interactive and replay
//! hosts use tick() followed by take_audio(). No audio device owns VM state.
use crate::Session;
use anyhow::{Result, ensure};
use krkrz_tjs::{Value, unsupported};

const RATE: u64 = 48_000;
mod resampler;
pub(crate) use resampler::Source;
impl Session {
    /// Drain stereo PCM at 48,000 frames per second from the latest tick. A
    /// headless host may discard it; source clocks and callbacks still advance.
    /// Empty output denotes silence when all sources are stopped or paused.
    pub fn take_audio(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.audio_output)
    }

    pub(crate) fn mix_clock_audio(&mut self, time_ms: u64) -> Result<()> {
        ensure!(
            time_ms >= self.services.time_ms,
            "session clock cannot move backwards"
        );
        let frames = (time_ms - self.services.time_ms)
            .checked_mul(48)
            .ok_or_else(|| unsupported("audio clock overflow"))?;
        ensure!(frames <= 1_000_000, "audio tick exceeds frame limit");
        self.audio_output.clear();
        if !self
            .services
            .sounds
            .values()
            .any(|s| s.status == "play" && !s.paused && s.loaded.as_ref().is_some_and(|l| !l.ended))
            && !self.services.videos.values().any(|v| v.has_audio())
        {
            return Ok(());
        }
        self.services.validate_sound_filter_memory()?;
        self.budget = self
            .budget
            .checked_sub(frames)
            .ok_or_else(|| unsupported("audio mixing execution budget exceeded"))?;
        // Each host tick replaces unconsumed output, so unattended headless
        // replay cannot accumulate an unbounded audio queue.
        self.audio_output.clear();
        self.audio_output.resize(frames as usize * 2, 0.0);
        if frames == 0 {
            return Ok(());
        }
        let global_volume = self.services.sound_global_volume as f32 / 100_000.0;
        for sound in self.services.sounds.values_mut() {
            if sound.status != "play" || sound.paused {
                continue;
            }
            let Some(loaded) = &mut sound.loaded else {
                continue;
            };
            if loaded.ended {
                continue;
            }
            let frequency = sound.frequency as u64;
            let end = loaded.mixer.phase + frames * frequency;
            let consumed = (end / RATE) as usize;
            let needed = consumed + 2;
            ensure!(
                needed <= 1_000_000,
                "resampled audio tick exceeds source frame limit"
            );
            self.budget = self
                .budget
                .checked_sub(needed as u64)
                .ok_or_else(|| unsupported("audio source execution budget exceeded"))?;
            if loaded.audio.channels > 2 {
                return Err(unsupported(
                    "native audio mixing of more than two source channels",
                ));
            }
            let volume =
                sound.volume as f32 / 100_000.0 * sound.volume2 as f32 / 100_000.0 * global_volume;
            let gain = [
                volume * (1.0 - sound.pan.max(0) as f32 / 100_000.0),
                volume * (1.0 + sound.pan.min(0) as f32 / 100_000.0),
            ];
            let (labels, ended) = loaded.mixer.mix(
                &mut loaded.pipeline,
                &mut loaded.stream,
                loaded.audio.channels as usize,
                frequency as u32,
                &mut self.audio_output,
                gain,
            )?;
            ensure!(
                sound.events.len() + labels.len() < 65_536,
                "sound event queue limit exceeded"
            );
            for label in labels {
                sound
                    .events
                    .push((sound.generation, "onLabel", Value::string(&label.name)));
            }
            if ended {
                loaded.ended = true;
                sound
                    .events
                    .push((sound.generation, "onStatusChanged", Value::string("stop")));
            }
        }
        for video in self.services.videos.values() {
            video.mix_audio(&mut self.audio_output);
        }
        for sample in &mut self.audio_output {
            *sample = sample.clamp(-1.0, 1.0);
        }
        Ok(())
    }
}
