use anyhow::{Context, Result, bail, ensure};
use cpal::{
    FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

const INPUT_RATE: f64 = 48_000.;
const MAX_FRAMES: usize = 24_000;
#[derive(Default)]
struct Queue {
    frames: VecDeque<[f32; 2]>,
    phase: f64,
    error: Option<String>,
}
impl Queue {
    fn push(&mut self, samples: &[f32]) {
        let samples = &samples[samples.len().saturating_sub(MAX_FRAMES * 2)..];
        let remove = (self.frames.len() + samples.len() / 2).saturating_sub(MAX_FRAMES);
        if remove != 0 {
            self.frames.drain(..remove);
            self.phase = 0.;
        }
        self.frames
            .extend(samples.as_chunks::<2>().0.iter().copied());
    }
    fn sample(&mut self, rate: u32) -> [f32; 2] {
        let Some(a) = self.frames.front().copied() else {
            self.phase = 0.;
            return [0.; 2];
        };
        let b = self.frames.get(1).copied().unwrap_or(a);
        let output =
            std::array::from_fn(|i| (a[i] + (b[i] - a[i]) * self.phase as f32).clamp(-1., 1.));
        self.phase += INPUT_RATE / f64::from(rate);
        let consumed = self.phase as usize;
        self.phase -= consumed as f64;
        let consumed = consumed.min(self.frames.len());
        self.frames.drain(..consumed);
        output
    }
}
pub struct AudioOutput {
    _stream: Stream,
    queue: Arc<Mutex<Queue>>,
}
impl AudioOutput {
    pub fn open() -> Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .context("no audio output device; use --no-audio for a silent run")?;
        let default = device
            .default_output_config()
            .context("query default audio output format")?;
        let preferred = device
            .supported_output_configs()?
            .filter(|c| {
                c.channels() == 2
                    && c.min_sample_rate().0 <= 48_000
                    && c.max_sample_rate().0 >= 48_000
            })
            .max_by_key(|c| match c.sample_format() {
                SampleFormat::F32 => 3,
                SampleFormat::I16 => 2,
                _ => 1,
            })
            .map(|c| c.with_sample_rate(cpal::SampleRate(48_000)))
            .unwrap_or(default);
        let config = preferred.config();
        ensure!(
            config.channels > 0 && config.sample_rate.0 > 0,
            "audio device returned an invalid format"
        );
        let queue = Arc::new(Mutex::new(Queue::default()));
        let stream = match preferred.sample_format() {
            SampleFormat::I8 => build::<i8>(&device, &config, queue.clone())?,
            SampleFormat::I16 => build::<i16>(&device, &config, queue.clone())?,
            SampleFormat::I32 => build::<i32>(&device, &config, queue.clone())?,
            SampleFormat::I64 => build::<i64>(&device, &config, queue.clone())?,
            SampleFormat::U8 => build::<u8>(&device, &config, queue.clone())?,
            SampleFormat::U16 => build::<u16>(&device, &config, queue.clone())?,
            SampleFormat::U32 => build::<u32>(&device, &config, queue.clone())?,
            SampleFormat::U64 => build::<u64>(&device, &config, queue.clone())?,
            SampleFormat::F32 => build::<f32>(&device, &config, queue.clone())?,
            SampleFormat::F64 => build::<f64>(&device, &config, queue.clone())?,
            format => bail!("unsupported audio device sample format: {format}"),
        };
        stream.play().context("start audio output stream")?;
        Ok(Self {
            _stream: stream,
            queue,
        })
    }
    pub fn submit(&self, samples: &[f32]) -> Result<()> {
        ensure!(
            samples.len().is_multiple_of(2),
            "audio mixer returned an incomplete stereo frame"
        );
        let mut queue = self
            .queue
            .lock()
            .map_err(|_| anyhow::anyhow!("audio callback lock is poisoned"))?;
        if let Some(error) = queue.error.take() {
            bail!("audio output stream failed: {error}");
        }
        queue.push(samples);
        Ok(())
    }
}
fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &StreamConfig,
    queue: Arc<Mutex<Queue>>,
) -> Result<Stream> {
    let channels = usize::from(config.channels);
    let rate = config.sample_rate.0;
    let errors = queue.clone();
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            let Ok(mut queue) = queue.lock() else {
                output.fill(T::from_sample(0.));
                return;
            };
            for frame in output.chunks_mut(channels) {
                let sample = queue.sample(rate);
                for (channel, out) in frame.iter_mut().enumerate() {
                    let value = if channels == 1 {
                        (sample[0] + sample[1]) * 0.5
                    } else {
                        sample.get(channel).copied().unwrap_or(0.)
                    };
                    *out = T::from_sample(value);
                }
            }
        },
        move |error| {
            if let Ok(mut queue) = errors.lock() {
                queue.error = Some(error.to_string());
            }
        },
        None,
    )?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stereo_queue_preserves_samples_and_interpolates_rate_changes() {
        let mut queue = Queue::default();
        queue.push(&[0., 0., 1., -1., 0., 0.]);
        assert_eq!(queue.sample(96_000), [0., 0.]);
        assert_eq!(queue.sample(96_000), [0.5, -0.5]);
        assert_eq!(queue.sample(96_000), [1., -1.]);
        assert_eq!(queue.sample(96_000), [0.5, -0.5]);
        assert_eq!(queue.sample(48_000), [0., 0.]);
        assert_eq!(queue.sample(48_000), [0., 0.]);
    }
    #[test]
    fn stalled_host_cannot_build_an_unbounded_audio_queue() {
        let mut queue = Queue::default();
        queue.push(&vec![0.; MAX_FRAMES * 4]);
        assert_eq!(queue.frames.len(), MAX_FRAMES);
    }
}
