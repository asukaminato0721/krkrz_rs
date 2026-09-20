//! Streaming Vorbis-window phase vocoder following Kirikiri PhaseVocoderDSP.cpp.
//! RustFFT replaces the original real FFT. Channels retain independent phase history.
use super::{Config, Settings};
use crate::audio::{AudioBlock, AudioLabel, SoundStream};
use anyhow::{Result, ensure};
use rustfft::{Fft, FftPlanner, num_complex::Complex32};
use std::{
    collections::VecDeque,
    f32::consts::{PI, TAU},
    sync::Arc,
};

#[derive(Clone, Default)]
struct Block {
    samples: Vec<f32>,
    labels: Vec<AudioLabel>,
    // Source frame position after each output frame, including loop jumps.
    positions: Vec<u64>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
    channels: usize,
    stages: Vec<Stage>,
    position: Option<u64>,
}
impl Pipeline {
    pub(crate) fn memory_bound(&self) -> usize {
        // FFT scratch, phase history, overlap buffers and source-position queues.
        // Reserve for unopened stages too, so repeated sound opens stay bounded.
        self.stages
            .iter()
            .map(|stage| {
                let window = stage
                    .dsp
                    .as_ref()
                    .map_or_else(|| stage.settings.get().window, |d| d.window);
                window * (self.channels + 1) * 96
            })
            .sum()
    }
    pub(super) fn new(channels: usize, filters: Vec<(usize, Settings)>) -> Self {
        Self {
            channels,
            stages: filters
                .into_iter()
                .map(|(id, settings)| Stage {
                    id,
                    settings,
                    dsp: None,
                })
                .collect(),
            position: None,
        }
    }
    pub(super) fn contains(&self, id: usize) -> bool {
        self.stages.iter().any(|s| s.id == id)
    }
    pub(crate) fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.dsp = None;
        }
        self.position = None;
    }
    pub(crate) fn position(&self, source: &SoundStream) -> u64 {
        self.position.unwrap_or_else(|| source.position())
    }
    pub(crate) fn render(&mut self, source: &mut SoundStream, frames: usize) -> Result<AudioBlock> {
        if self.stages.is_empty() {
            return source.render(frames);
        }
        ensure!(
            self.memory_bound() <= 128 << 20,
            "wave filter memory limit exceeded"
        );
        let mut work = 16_000_000usize;
        let block = pull(&mut self.stages, source, self.channels, frames, &mut work)?;
        if let Some(position) = block.positions.last() {
            self.position = Some(*position);
        }
        Ok(AudioBlock {
            samples: block
                .samples
                .into_iter()
                .map(|v| (v * 32768.0).round_ties_even().clamp(-32768.0, 32767.0) as i16)
                .collect(),
            labels: block.labels,
        })
    }
}

#[derive(Clone)]
struct Stage {
    id: usize,
    settings: Settings,
    dsp: Option<Dsp>,
}

fn pull(
    stages: &mut [Stage],
    source: &mut SoundStream,
    channels: usize,
    frames: usize,
    work: &mut usize,
) -> Result<Block> {
    ensure!(
        frames <= 1_000_000,
        "vocoder output block exceeds frame limit"
    );
    let Some((stage, before)) = stages.split_last_mut() else {
        let mut positions = Vec::new();
        let block = source.render_tracked(frames, &mut positions)?;
        return Ok(Block {
            samples: block
                .samples
                .into_iter()
                .map(|v| v as f32 / 32768.0)
                .collect(),
            labels: block.labels,
            positions,
        });
    };
    let mut config = stage.settings.get();
    if stage.dsp.is_none() {
        stage.dsp = Some(Dsp::new(config.window, channels));
    }
    let dsp = stage.dsp.as_mut().unwrap();
    // Window changes take effect when the filter is reset, as in the native DSP.
    config.window = dsp.window;
    let (input_hop, output_hop, overlap) = hops(config)?;
    let mut result = Block::default();
    while result.positions.len() < frames {
        if dsp.output_positions.is_empty() {
            loop {
                let cost = dsp.window * channels;
                ensure!(*work >= cost, "vocoder processing budget exhausted");
                *work -= cost;
                let block = pull(before, source, channels, input_hop, work)?;
                if block.samples.is_empty() {
                    return Ok(result);
                }
                let offset = dsp.input.len() / channels;
                for mut label in block.labels {
                    label.offset += offset;
                    dsp.input_labels.push_back(label);
                }
                let count = block.positions.len();
                let last = block
                    .positions
                    .last()
                    .copied()
                    .unwrap_or_else(|| source.position());
                dsp.input.extend(block.samples);
                dsp.input
                    .extend(std::iter::repeat_n(0.0, (input_hop - count) * channels));
                dsp.input_positions.extend(block.positions);
                dsp.input_positions
                    .extend(std::iter::repeat_n(last, input_hop - count));
                if dsp.input.len() / channels >= dsp.window {
                    break;
                }
            }
            dsp.process(config, input_hop, output_hop, overlap);
        }
        let count = (frames - result.positions.len()).min(dsp.output_positions.len());
        let offset = result.positions.len();
        result.samples.extend(dsp.output.drain(..count * channels));
        result.positions.extend(dsp.output_positions.drain(..count));
        while dsp.output_labels.front().is_some_and(|l| l.offset < count) {
            let mut label = dsp.output_labels.pop_front().unwrap();
            label.offset += offset;
            ensure!(
                result.labels.len() < 65_536,
                "vocoder label output limit exceeded"
            );
            result.labels.push(label);
        }
        for label in &mut dsp.output_labels {
            label.offset -= count;
        }
    }
    Ok(result)
}

fn hops(config: Config) -> Result<(usize, usize, usize)> {
    ensure!(
        config.time.is_finite()
            && config.time > 0.0
            && config.pitch.is_finite()
            && config.pitch > 0.0,
        "vocoder playback requires finite positive time and pitch"
    );
    let overlap = if config.overlap != 0 {
        config.overlap
    } else if config.time <= 0.2 {
        2
    } else if config.time <= 1.2 {
        4
    } else {
        8
    };
    let input = config.window / overlap;
    let output = (input as f32 * config.time) as usize & !1;
    ensure!(
        (2..=config.window).contains(&output),
        "vocoder output hop is outside supported window bounds"
    );
    Ok((input, output, overlap))
}

#[derive(Clone)]
struct Dsp {
    window: usize,
    channels: usize,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    fft: Vec<Complex32>,
    scratch: Vec<Complex32>,
    magnitudes: Vec<f32>,
    frequencies: Vec<f32>,
    last_analysis: Vec<f32>,
    last_synthesis: Vec<f32>,
    window_values: Vec<f32>,
    accumulation: Vec<f32>,
    input: VecDeque<f32>,
    input_positions: VecDeque<u64>,
    input_labels: VecDeque<AudioLabel>,
    output: VecDeque<f32>,
    output_positions: VecDeque<u64>,
    output_labels: VecDeque<AudioLabel>,
}
impl Dsp {
    fn new(window: usize, channels: usize) -> Self {
        let mut planner = FftPlanner::new();
        let forward = planner.plan_fft_forward(window);
        let inverse = planner.plan_fft_inverse(window);
        let scratch_size = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        Self {
            window,
            channels,
            forward,
            inverse,
            fft: vec![Complex32::default(); window],
            scratch: vec![Complex32::default(); scratch_size],
            magnitudes: vec![0.0; window / 2],
            frequencies: vec![0.0; window / 2],
            last_analysis: vec![0.0; window / 2 * channels],
            last_synthesis: vec![0.0; window / 2 * channels],
            window_values: (0..window)
                .map(|i| {
                    let x = (i as f64 + 0.5) / window as f64;
                    (std::f64::consts::FRAC_PI_2 * (std::f64::consts::PI * x).sin().powi(2)).sin()
                        as f32
                })
                .collect(),
            accumulation: vec![0.0; window * channels],
            input: VecDeque::new(),
            input_positions: VecDeque::new(),
            input_labels: VecDeque::new(),
            output: VecDeque::new(),
            output_positions: VecDeque::new(),
            output_labels: VecDeque::new(),
        }
    }
    fn process(&mut self, config: Config, input_hop: usize, output_hop: usize, overlap: usize) {
        let n = self.window;
        let bins = n / 2;
        let step = TAU / overlap as f32;
        let scale = output_hop as f32 / input_hop as f32;
        // RealFFT's inverse has half the gain of RustFFT's complex inverse.
        let gain = config.time / n as f32 / config.pitch.sqrt() / overlap as f32 * 2.0;
        for channel in 0..self.channels {
            for i in 0..n {
                self.fft[i] = Complex32::new(
                    self.input[i * self.channels + channel] * self.window_values[i],
                    0.0,
                );
            }
            self.forward
                .process_with_scratch(&mut self.fft, &mut self.scratch);
            for i in 0..bins {
                let phase = self.fft[i].arg();
                let history = channel * bins + i;
                let delta = wrap(phase - self.last_analysis[history] - i as f32 * step);
                self.last_analysis[history] = phase;
                self.magnitudes[i] = self.fft[i].norm();
                self.frequencies[i] = (i as f32 + delta / step) * step;
            }
            self.fft.fill(Complex32::default());
            for i in 0..bins {
                let index = i as f32 / config.pitch;
                let low = index as usize;
                let fraction = index - low as f32;
                let (magnitude, frequency) = if low >= bins {
                    (0.0, 0.0)
                } else if low + 1 == bins {
                    (self.magnitudes[low], self.frequencies[low] * config.pitch)
                } else {
                    (
                        self.magnitudes[low]
                            + fraction * (self.magnitudes[low + 1] - self.magnitudes[low]),
                        (self.frequencies[low]
                            + fraction * (self.frequencies[low + 1] - self.frequencies[low]))
                            * config.pitch,
                    )
                };
                let history = channel * bins + i;
                let phase = wrap(self.last_synthesis[history] + frequency * scale);
                self.last_synthesis[history] = phase;
                self.fft[i] = Complex32::from_polar(magnitude, phase);
                if i > 0 {
                    self.fft[n - i] = self.fft[i].conj();
                }
            }
            self.fft[0].im = 0.0;
            self.inverse
                .process_with_scratch(&mut self.fft, &mut self.scratch);
            for i in 0..n {
                self.accumulation[i * self.channels + channel] +=
                    self.fft[i].re * self.window_values[i] * gain;
            }
        }
        self.output
            .extend(&self.accumulation[..output_hop * self.channels]);
        self.accumulation
            .copy_within(output_hop * self.channels.., 0);
        self.accumulation[(n - output_hop) * self.channels..].fill(0.0);
        for frame in 0..output_hop {
            let input_frame = ((frame + 1) * input_hop / output_hop).saturating_sub(1);
            self.output_positions
                .push_back(self.input_positions[input_frame]);
        }
        while self
            .input_labels
            .front()
            .is_some_and(|l| l.offset < input_hop)
        {
            let mut label = self.input_labels.pop_front().unwrap();
            label.offset = label.offset * output_hop / input_hop;
            self.output_labels.push_back(label);
        }
        for label in &mut self.input_labels {
            label.offset -= input_hop;
        }
        self.input.drain(..input_hop * self.channels);
        self.input_positions.drain(..input_hop);
    }
}
fn wrap(phase: f32) -> f32 {
    (phase + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use krkrz_assets::{media::Audio, sli::LoopInfo};
    use std::{cell::Cell, rc::Rc};

    fn stream(time: f32, pitch: f32, looping: bool) -> (Pipeline, SoundStream) {
        let audio = Arc::new(Audio {
            channels: 2,
            sample_rate: 8192,
            samples: (0..8192)
                .flat_map(|i| {
                    let sample = ((i as f32 * TAU / 64.0).sin() * 16000.0) as i16;
                    [sample, 0]
                })
                .collect(),
        });
        let info = if looping {
            LoopInfo::parse(
                "#2.00\nLink { From=2048; To=1024; }\nLabel { Position=1536; Name='lip'; }",
            )
            .unwrap()
        } else {
            LoopInfo::default()
        };
        let config = Config {
            window: 256,
            time,
            pitch,
            ..Config::default()
        };
        (
            Pipeline::new(2, vec![(1, Rc::new(Cell::new(config)))]),
            SoundStream::new(audio, info).unwrap(),
        )
    }
    #[test]
    fn time_changes_duration_and_preserves_pitch() {
        // Steady-state RMS from the pinned upstream scalar DSP, with the same
        // quantized synthetic input. Time stretching changes the native gain.
        for (time, pitch, reference_rms) in [
            (0.5, 1.0, 8749.25),
            (1.0, 1.0, 11313.19),
            (2.0, 1.0, 5103.39),
            (1.0, 2.0, 8949.45),
        ] {
            let (mut pipeline, mut source) = stream(time, pitch, false);
            let output = pipeline.render(&mut source, 20000).unwrap();
            let frames = output.samples.len() / 2;
            let (input, hop, _) = hops(Config {
                window: 256,
                time,
                pitch,
                ..Config::default()
            })
            .unwrap();
            assert_eq!(frames, (8192 / input - 256 / input + 1) * hop);
            let mono: Vec<_> = output
                .samples
                .as_chunks::<2>()
                .0
                .iter()
                .skip(512)
                .take(2048)
                .map(|f| {
                    assert_eq!(f[1], 0, "silent channel received signal");
                    f[0] as f32
                })
                .collect();
            let crossings = mono
                .windows(2)
                .filter(|s| s[0] < 0.0 && s[1] >= 0.0)
                .count();
            assert!(
                (crossings as f32 - 32.0 * pitch).abs() <= 2.0,
                "time={time}, pitch={pitch}, crossings={crossings}"
            );
            let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
            assert!(
                (rms - reference_rms).abs() < reference_rms * 0.02,
                "time={time}, pitch={pitch}, rms={rms}"
            );
        }
    }
    #[test]
    fn loop_labels_and_pcm_do_not_depend_on_pull_size() {
        let (mut pipeline, mut source) = stream(1.5, 1.0, true);
        let expected = pipeline.render(&mut source, 8192).unwrap();
        assert!(expected.labels.len() >= 3);
        assert_eq!(expected.labels[0].offset, 2304);
        let position = pipeline.position(&source);
        for size in [1, 7, 127, 509] {
            let (mut pipeline, mut source) = stream(1.5, 1.0, true);
            let mut actual = AudioBlock::default();
            while actual.samples.len() < expected.samples.len() {
                let offset = actual.samples.len() / 2;
                let block = pipeline
                    .render(&mut source, size.min(8192 - offset))
                    .unwrap();
                actual.samples.extend(block.samples);
                actual.labels.extend(block.labels.into_iter().map(|mut l| {
                    l.offset += offset;
                    l
                }));
            }
            assert_eq!(actual, expected, "pull size {size}");
            assert_eq!(pipeline.position(&source), position);
        }
    }
    #[test]
    fn seek_resets_phase_history_and_invalid_parameters_fail() {
        let (mut pipeline, mut source) = stream(0.5, 1.0, false);
        let expected = pipeline.render(&mut source, 1024).unwrap();
        source.seek(0).unwrap();
        pipeline.reset();
        assert_eq!(pipeline.render(&mut source, 1024).unwrap(), expected);
        for time in [0.0, -1.0, f32::NAN, f32::INFINITY, 0.00001, 100.0] {
            let (mut pipeline, mut source) = stream(time, 1.0, false);
            assert!(pipeline.render(&mut source, 1024).is_err(), "time={time}");
        }
    }
}
