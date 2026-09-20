//! Deterministic PCM source with Kirikiri SLI links and 50 ms integer crossfades.
//! Adapted from Kirikiri sound/WaveLoopManager.cpp; see THIRD_PARTY_NOTICES.md.
use anyhow::{Context, Result, ensure};
use krkrz_assets::{
    media::Audio,
    sli::{Link, LoopInfo},
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioLabel {
    /// Frame offset in this output block, after loop jumps have been applied.
    pub offset: usize,
    pub name: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioBlock {
    pub samples: Vec<i16>,
    pub labels: Vec<AudioLabel>,
}
#[derive(Clone)]
struct CrossFade {
    samples: Vec<i16>,
    cursor: usize,
}
#[derive(Clone)]
pub struct SoundStream {
    audio: Arc<Audio>,
    info: LoopInfo,
    flags: [i32; 16],
    position: u64,
    crossfade: Option<CrossFade>,
    pub ignore_links: bool,
    pub loop_at_end: bool,
}
impl SoundStream {
    pub fn new(audio: Arc<Audio>, mut info: LoopInfo) -> Result<Self> {
        ensure!(
            audio.channels > 0 && audio.sample_rate > 0 && audio.sample_rate <= 768_000,
            "invalid PCM stream format"
        );
        ensure!(
            audio.samples.len().is_multiple_of(audio.channels as usize),
            "incomplete PCM frame"
        );
        info.validate_frames(audio.frames() as u64)?;
        ensure!(
            info.labels.iter().all(|l| !l.name.starts_with(':')),
            "SLI label expressions are not implemented"
        );
        info.sort();
        Ok(Self {
            audio,
            info,
            flags: [0; 16],
            position: 0,
            crossfade: None,
            ignore_links: false,
            loop_at_end: false,
        })
    }
    pub fn position(&self) -> u64 {
        self.position
    }
    pub fn seek(&mut self, position: u64) -> Result<()> {
        ensure!(
            position <= self.audio.frames() as u64,
            "sound seek is outside audio frames"
        );
        self.position = position;
        self.crossfade = None;
        Ok(())
    }
    pub fn set_flag(&mut self, index: usize, value: i32) -> Result<()> {
        *self
            .flags
            .get_mut(index)
            .context("sound flag index out of range")? = value.clamp(0, 9999);
        Ok(())
    }
    /// Render at the source rate. EOF returns a short block; labels use output frames.
    /// Crossfade state persists across calls, so output does not depend on block size.
    pub fn render(&mut self, frames: usize) -> Result<AudioBlock> {
        ensure!(frames <= 1_000_000, "audio block exceeds frame limit");
        let channels = self.audio.channels as usize;
        let total = self.audio.frames() as u64;
        let mut out = AudioBlock::default();
        let mut written = 0;
        let mut jumps = 0;
        while written < frames {
            let link = if self.ignore_links {
                None
            } else {
                self.info
                    .next_link(self.position, Some(&self.flags))
                    .cloned()
            };
            let mut next = total;
            if let Some(link) = link {
                if link.from == self.position {
                    jumps += 1;
                    ensure!(jumps < 10, "SLI links form a cycle without producing audio");
                    self.position = link.to;
                    continue;
                }
                next = link.from;
                if link.smooth {
                    let half = self.audio.sample_rate as u64 * 25 / 1000;
                    let before = half.min(link.from).min(link.to);
                    if link.from - before > self.position {
                        next = link.from - before;
                    } else if self.crossfade.is_none() {
                        self.prepare_fade(&link, half)?;
                    }
                }
            }
            if self.position == total && self.crossfade.is_none() {
                if !self.loop_at_end || total == 0 {
                    break;
                }
                self.position = 0;
                continue;
            }
            let mut count = (next - self.position).min((frames - written) as u64) as usize;
            if let Some(fade) = &self.crossfade {
                count = count.min((fade.samples.len() - fade.cursor) / channels);
            }
            ensure!(count > 0, "audio source cannot make progress");
            jumps = 0;
            let first = self
                .info
                .labels
                .partition_point(|l| l.position < self.position);
            for label in &self.info.labels[first..] {
                if label.position >= self.position + count as u64 {
                    break;
                }
                ensure!(
                    out.labels.len() < 65_536,
                    "audio label output limit exceeded"
                );
                out.labels.push(AudioLabel {
                    offset: written + (label.position - self.position) as usize,
                    name: label.name.clone(),
                });
            }
            if let Some(fade) = &mut self.crossfade {
                let end = fade.cursor + count * channels;
                out.samples
                    .extend_from_slice(&fade.samples[fade.cursor..end]);
                fade.cursor = end;
                if end == fade.samples.len() {
                    self.crossfade = None;
                }
            } else {
                let start = self.position as usize * channels;
                out.samples
                    .extend_from_slice(&self.audio.samples[start..start + count * channels]);
            }
            self.position += count as u64;
            written += count;
        }
        Ok(out)
    }
    fn prepare_fade(&mut self, link: &Link, half: u64) -> Result<()> {
        let total = self.audio.frames() as u64;
        let before = link.from - self.position;
        let mut after = half.min(total - link.from).min(total - link.to);
        if let Some(next) = self.info.next_link(link.to, None) {
            after = after.min(next.from - link.to);
        }
        ensure!(before <= link.to, "invalid SLI crossfade range");
        let channels = self.audio.channels as usize;
        let mut samples = Vec::with_capacity((before + after) as usize * channels);
        for (offset, frames, ratio_start) in [(0, before, 0u64), (before, after, 1u64 << 31)] {
            if frames == 0 {
                continue;
            }
            let step = (1u64 << 31) / frames;
            for frame in 0..frames {
                let ratio = ratio_start + frame * step;
                let from = (self.position + offset + frame) as usize * channels;
                let to = (link.to - before + offset + frame) as usize * channels;
                for channel in 0..channels {
                    let a = self.audio.samples[from + channel] as i64;
                    let b = self.audio.samples[to + channel] as i64;
                    // Keep upstream's independent rounding for the two products.
                    let blended =
                        ((b * ratio as i64) >> 32) + ((a * ((1u64 << 32) - ratio) as i64) >> 32);
                    samples.push(blended as i16);
                }
            }
        }
        if !samples.is_empty() {
            self.crossfade = Some(CrossFade { samples, cursor: 0 });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream(source: &str) -> SoundStream {
        let audio = Audio {
            channels: 2,
            sample_rate: 80,
            samples: (0..10).flat_map(|i| [i * 100, -i * 100]).collect(),
        };
        SoundStream::new(Arc::new(audio), LoopInfo::parse(source).unwrap()).unwrap()
    }
    #[test]
    fn hard_loop_and_label_offsets() {
        let mut s = stream("#2.00\nLink { From=6; To=2; }\nLabel { Position=3; Name='mouth'; }");
        let block = s.render(12).unwrap();
        assert_eq!(
            block
                .samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|f| f[0])
                .collect::<Vec<_>>(),
            [0, 100, 200, 300, 400, 500, 200, 300, 400, 500, 200, 300]
        );
        assert_eq!(
            block.labels.iter().map(|l| l.offset).collect::<Vec<_>>(),
            [3, 7, 11]
        );
        assert_eq!(s.position(), 4);
    }
    #[test]
    fn smooth_samples_and_block_size_independence() {
        let source =
            "#2.00\nLink { From=6; To=2; Smooth=True; Condition=eq; CondVar=0; RefValue=0; }";
        let mut whole = stream(source);
        let expected = whole.render(17).unwrap().samples;
        // 25 ms at 80 Hz is two frames on each side of the jump.
        assert_eq!(
            &expected[..16],
            &[
                0, 0, 100, -100, 200, -200, 300, -300, 400, -400, 400, -400, 400, -400, 400, -400
            ]
        );
        for size in 1..8 {
            let mut split = stream(source);
            let mut actual = Vec::new();
            let mut remaining = 17;
            while remaining > 0 {
                let count = size.min(remaining);
                actual.extend(split.render(count).unwrap().samples);
                remaining -= count;
            }
            assert_eq!(actual, expected, "block size {size}");
            assert_eq!(split.position(), whole.position());
        }
        whole.seek(0).unwrap();
        whole.set_flag(0, 1).unwrap();
        assert_eq!(whole.render(20).unwrap().samples.len(), 20);
        assert!(whole.render(1).unwrap().samples.is_empty());
    }
    #[test]
    fn zero_progress_and_invalid_inputs_are_errors() {
        let mut s = stream("#2.00\nLink { From=0; To=0; }");
        assert!(s.render(1).is_err());
        assert!(s.seek(11).is_err());
        assert!(s.set_flag(16, 0).is_err());
        let audio = Arc::new(Audio {
            channels: 0,
            sample_rate: 0,
            samples: vec![],
        });
        assert!(SoundStream::new(audio, LoopInfo::default()).is_err());
    }
}
