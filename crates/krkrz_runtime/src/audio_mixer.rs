//! Shared host-clock audio: stereo 48 kHz PCM and deferred native label events.
//! Source-rate pulls remain available for decoder tools; interactive and replay
//! hosts use tick() followed by take_audio(). No audio device owns VM state.
use crate::{Session, audio::AudioLabel};
use anyhow::{Result, ensure};
use krkrz_tjs::{Value, unsupported};

const RATE: u64 = 48_000;
#[derive(Default)]
pub(crate) struct Source {
    phase: u64,
    samples: Vec<[f32; 2]>,
    labels: Vec<AudioLabel>,
    eof: bool,
}
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
            let mixer = &mut loaded.mixer;
            if mixer.samples.len() < needed && !mixer.eof {
                let requested = needed - mixer.samples.len();
                let block = loaded.pipeline.render(&mut loaded.stream, requested)?;
                let channels = loaded.audio.channels as usize;
                mixer.eof = block.samples.len() / channels < requested;
                let base = mixer.samples.len();
                for frame in block.samples.chunks_exact(channels) {
                    let left = frame[0] as f32 / 32768.0;
                    let right = frame[channels - 1] as f32 / 32768.0;
                    mixer.samples.push([left, right]);
                }
                mixer
                    .labels
                    .extend(block.labels.into_iter().map(|mut label| {
                        label.offset += base;
                        label
                    }));
            }
            let volume =
                sound.volume as f32 / 100_000.0 * sound.volume2 as f32 / 100_000.0 * global_volume;
            let gain = [
                volume * (1.0 - sound.pan.max(0) as f32 / 100_000.0),
                volume * (1.0 + sound.pan.min(0) as f32 / 100_000.0),
            ];
            for output in self.audio_output.as_chunks_mut::<2>().0 {
                let at = (mixer.phase / RATE) as usize;
                let fraction = (mixer.phase % RATE) as f32 / RATE as f32;
                let a = mixer.samples.get(at).copied().unwrap_or([0.0; 2]);
                let b = mixer.samples.get(at + 1).copied().unwrap_or([0.0; 2]);
                for channel in 0..2 {
                    output[channel] +=
                        (a[channel] + (b[channel] - a[channel]) * fraction) * gain[channel];
                }
                mixer.phase += frequency;
            }
            mixer.phase %= RATE;
            let old_labels = std::mem::take(&mut mixer.labels);
            for mut label in old_labels {
                if label.offset < consumed {
                    ensure!(
                        sound.events.len() < 65_536,
                        "sound event queue limit exceeded"
                    );
                    sound
                        .events
                        .push((sound.generation, "onLabel", Value::string(&label.name)));
                } else {
                    label.offset -= consumed;
                    mixer.labels.push(label);
                }
            }
            let drained = consumed.min(mixer.samples.len());
            mixer.samples.drain(..drained);
            if mixer.eof && mixer.samples.is_empty() {
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
