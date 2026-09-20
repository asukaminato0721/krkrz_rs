use anyhow::{Context, Result, bail, ensure};
use audioadapter_buffers::direct::InterleavedSlice;
use cpal::{
    FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::{Consumer, Producer, RingBuffer};
use rubato::{
    Async, FixedAsync, Resampler, Resizable, SincInterpolationParameters, WindowFunction,
};
use std::sync::mpsc::{self, Receiver, SyncSender};

const INPUT_RATE: u32 = 48_000;
const CHUNK: usize = 1024;

struct OutputQueue {
    producer: Producer<[f32; 2]>,
    resampler: Option<Async<f32>>,
    output: Vec<f32>,
}
impl OutputQueue {
    fn new(rate: u32) -> Result<(Self, Consumer<[f32; 2]>)> {
        ensure!(
            (1..=768_000).contains(&rate),
            "invalid audio device sample rate"
        );
        // At most half a second of device-rate audio. No allocations or locks in
        // the rendering callback, including underruns.
        let (producer, consumer) = RingBuffer::new((rate as usize / 2).max(1));
        let resampler = if rate == INPUT_RATE {
            None
        } else {
            Some(Async::new_sinc(
                rate as f64 / INPUT_RATE as f64,
                1.0,
                &SincInterpolationParameters::new(64, WindowFunction::BlackmanHarris2),
                CHUNK,
                2,
                FixedAsync::Input,
            )?)
        };
        let output = vec![0.; resampler.as_ref().map_or(0, |r| r.output_frames_max() * 2)];
        Ok((
            Self {
                producer,
                resampler,
                output,
            },
            consumer,
        ))
    }

    fn push(&mut self, samples: &[f32]) -> Result<()> {
        ensure!(
            samples.len().is_multiple_of(2),
            "audio mixer returned an incomplete stereo frame"
        );
        for chunk in samples.chunks(CHUNK * 2) {
            let output = if let Some(resampler) = &mut self.resampler {
                // Variable input chunks avoid accumulating another block of
                // latency. The sinc filter itself delays audio by about 32 input
                // frames (<1 ms); 48 kHz devices bypass it entirely.
                resampler.set_chunk_size(chunk.len() / 2)?;
                let input = InterleavedSlice::new(chunk, 2, chunk.len() / 2)?;
                let frames = self.output.len() / 2;
                let mut output = InterleavedSlice::new_mut(&mut self.output, 2, frames)?;
                let (_, written) = resampler.process_into_buffer(&input, &mut output, None)?;
                &self.output[..written * 2]
            } else {
                chunk
            };
            for frame in output.as_chunks::<2>().0 {
                // A full SPSC ring drops incoming frames. Only its consumer may
                // remove queued frames; never block the game or audio callback.
                let _ = self.producer.push(frame.map(|v| v.clamp(-1., 1.)));
            }
        }
        Ok(())
    }
}

pub struct AudioOutput {
    _stream: Stream,
    queue: OutputQueue,
    errors: Receiver<String>,
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
        let (queue, consumer) = OutputQueue::new(config.sample_rate.0)?;
        let (sender, errors) = mpsc::sync_channel(1);
        let stream = match preferred.sample_format() {
            SampleFormat::I8 => build::<i8>(&device, &config, consumer, sender)?,
            SampleFormat::I16 => build::<i16>(&device, &config, consumer, sender)?,
            SampleFormat::I32 => build::<i32>(&device, &config, consumer, sender)?,
            SampleFormat::I64 => build::<i64>(&device, &config, consumer, sender)?,
            SampleFormat::U8 => build::<u8>(&device, &config, consumer, sender)?,
            SampleFormat::U16 => build::<u16>(&device, &config, consumer, sender)?,
            SampleFormat::U32 => build::<u32>(&device, &config, consumer, sender)?,
            SampleFormat::U64 => build::<u64>(&device, &config, consumer, sender)?,
            SampleFormat::F32 => build::<f32>(&device, &config, consumer, sender)?,
            SampleFormat::F64 => build::<f64>(&device, &config, consumer, sender)?,
            format => bail!("unsupported audio device sample format: {format}"),
        };
        stream.play().context("start audio output stream")?;
        Ok(Self {
            _stream: stream,
            queue,
            errors,
        })
    }
    pub fn submit(&mut self, samples: &[f32]) -> Result<()> {
        if let Ok(error) = self.errors.try_recv() {
            bail!("audio output stream failed: {error}");
        }
        self.queue.push(samples)
    }
}
fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut consumer: Consumer<[f32; 2]>,
    errors: SyncSender<String>,
) -> Result<Stream> {
    let channels = usize::from(config.channels);
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            for frame in output.chunks_mut(channels) {
                let sample = consumer.pop().unwrap_or([0.; 2]);
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
            let _ = errors.try_send(error.to_string());
        },
        None,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_rate_preserves_stereo_and_recovers_after_underrun() {
        let (mut queue, mut consumer) = OutputQueue::new(48_000).unwrap();
        assert!(consumer.pop().is_err());
        queue.push(&[0.25, -0.5, 1., -1.]).unwrap();
        assert_eq!(consumer.pop().unwrap(), [0.25, -0.5]);
        assert_eq!(consumer.pop().unwrap(), [1., -1.]);
        assert!(consumer.pop().is_err());
        queue.push(&[0.75, 0.5]).unwrap();
        assert_eq!(consumer.pop().unwrap(), [0.75, 0.5]);
        assert!(queue.push(&[1.]).is_err());
    }
    #[test]
    fn stalled_host_is_bounded_and_accepts_audio_after_draining() {
        let (mut queue, mut consumer) = OutputQueue::new(48_000).unwrap();
        queue.push(&vec![0.25; 96_000]).unwrap();
        assert_eq!(consumer.slots(), 24_000);
        while consumer.pop().is_ok() {}
        queue.push(&[0.75, -0.75]).unwrap();
        assert_eq!(consumer.pop().unwrap(), [0.75, -0.75]);
    }
    fn tone(rate: u32, frequency: f32) -> Vec<[f32; 2]> {
        let (mut queue, mut consumer) = OutputQueue::new(rate).unwrap();
        let mut result = Vec::new();
        for start in (0..4800).step_by(137) {
            let samples: Vec<_> = (start..(start + 137).min(4800))
                .flat_map(|i| {
                    let v = (std::f32::consts::TAU * frequency * i as f32 / 48_000.).sin() * 0.5;
                    [v, -v]
                })
                .collect();
            queue.push(&samples).unwrap();
            while let Ok(frame) = consumer.pop() {
                result.push(frame);
            }
        }
        result
    }
    #[test]
    fn device_resampling_preserves_rate_channels_and_rejects_aliases() {
        for rate in [32_000, 44_100, 96_000] {
            let frames = tone(rate, 1000.);
            assert!((frames.len() as i64 - rate as i64 / 10).abs() < 128);
            assert!(
                frames
                    .iter()
                    .all(|p| p[0].is_finite() && (p[0] + p[1]).abs() < 1e-6)
            );
            let rms = (frames[128..].iter().map(|p| p[0] * p[0]).sum::<f32>()
                / (frames.len() - 128) as f32)
                .sqrt();
            assert!((rms - 0.3535).abs() < 0.02, "{rate}: {rms}");
        }
        let frames = tone(32_000, 18_000.);
        let peak = frames[128..].iter().map(|p| p[0].abs()).fold(0., f32::max);
        assert!(peak < 0.03, "aliased tone peak: {peak}");
    }
}
