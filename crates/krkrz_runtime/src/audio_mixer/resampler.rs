//! Streaming interpolation is owned by rubato; source-clock events use integer
//! frame counts so filter lookahead never delivers a label or EOF early.
use super::RATE;
use crate::{
    audio::{AudioLabel, SoundStream},
    phase_vocoder::Pipeline,
};
use anyhow::Result;
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Adjustable, Async, FixedAsync, PolynomialDegree, Resampler, Resizable};
use std::collections::VecDeque;

const CHUNK: usize = 256;

#[derive(Default)]
pub(crate) struct Source {
    pub(super) phase: u64,
    position: u64,
    decoded: u64,
    labels: VecDeque<(u64, AudioLabel)>,
    pending: VecDeque<[f32; 2]>,
    eof: bool,
    frequency: u32,
    resampler: Option<Async<f32>>,
    input: Vec<f32>,
    output: Vec<f32>,
}
impl Source {
    /// Keep the existing linear interpolation quality and small lookahead.
    /// Sinc filtering would change latency and conditional SLI read-ahead.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn mix(
        &mut self,
        pipeline: &mut Pipeline,
        stream: &mut SoundStream,
        channels: usize,
        frequency: u32,
        target: &mut [f32],
        gain: [f32; 2],
    ) -> Result<(Vec<AudioLabel>, bool)> {
        let frames = target.len() / 2;
        let end = self.phase + frames as u64 * frequency as u64;
        let through = self.position + end / RATE;
        // Decode through the logical end, including when downsampling skips
        // samples after the last output point. Those samples can contain labels
        // or EOF. Retain only the not-yet-submitted interpolation lookahead.
        let needed = (through + 2).saturating_sub(self.decoded) as usize;
        self.fetch(pipeline, stream, channels, needed)?;
        if self.resampler.is_none() {
            // The minimum supported frequency is 1 Hz, the maximum 768 kHz.
            // Fixed output bounds input storage at CHUNK * 16 + filter history,
            // even with the full adjustment range. No large output queue.
            let mut resampler = Async::new_poly(
                RATE as f64,
                768_000.0,
                PolynomialDegree::Linear,
                CHUNK,
                2,
                FixedAsync::Output,
            )?;
            // Prime at 1:1 so the first emitted sample is source sample zero,
            // including when playing very low source frequencies.
            resampler.set_resample_ratio(1.0, false)?;
            self.input = vec![0.; resampler.input_frames_max() * 2];
            self.output = vec![0.; CHUNK * 2];
            self.resampler = Some(resampler);
        }
        let mut written = 0;
        if self.frequency != frequency {
            // Rubato advances before emitting a sample. The first output after
            // a rate change must use the previous step to reach the next source
            // position, then subsequent outputs use the newly requested rate.
            self.chunk(pipeline, stream, channels, &mut target[..2], gain)?;
            written = 1;
            self.resampler
                .as_mut()
                .unwrap()
                .set_resample_ratio(RATE as f64 / frequency as f64, false)?;
            self.frequency = frequency;
        }
        for chunk in target[written * 2..].chunks_mut(CHUNK * 2) {
            self.chunk(pipeline, stream, channels, chunk, gain)?;
        }
        self.position = through;
        self.phase = end % RATE;
        let mut labels = Vec::new();
        while self
            .labels
            .front()
            .is_some_and(|(at, _)| *at < self.position)
        {
            labels.push(self.labels.pop_front().unwrap().1);
        }
        Ok((labels, self.eof && self.position >= self.decoded))
    }

    fn fetch(
        &mut self,
        pipeline: &mut Pipeline,
        stream: &mut SoundStream,
        channels: usize,
        needed: usize,
    ) -> Result<()> {
        if self.eof || needed == 0 {
            return Ok(());
        }
        let block = pipeline.render(stream, needed)?;
        let decoded = block.samples.len() / channels;
        self.eof = decoded < needed;
        self.pending
            .extend(block.samples.chunks_exact(channels).map(|frame| {
                [
                    frame[0] as f32 / 32768.,
                    frame[channels - 1] as f32 / 32768.,
                ]
            }));
        self.labels.extend(
            block
                .labels
                .into_iter()
                .map(|label| (self.decoded + label.offset as u64, label)),
        );
        self.decoded += decoded as u64;
        Ok(())
    }

    fn chunk(
        &mut self,
        pipeline: &mut Pipeline,
        stream: &mut SoundStream,
        channels: usize,
        target: &mut [f32],
        gain: [f32; 2],
    ) -> Result<()> {
        let resampler = self.resampler.as_mut().unwrap();
        resampler.set_chunk_size(target.len() / 2)?;
        let needed = resampler.input_frames_next();
        let missing = needed.saturating_sub(self.pending.len());
        self.fetch(pipeline, stream, channels, missing)?;
        let input = &mut self.input[..needed * 2];
        for out in input.as_chunks_mut::<2>().0 {
            *out = self.pending.pop_front().unwrap_or([0.; 2]);
        }
        let input = InterleavedSlice::new(input, 2, needed)?;
        let output_frames = target.len() / 2;
        let mut output =
            InterleavedSlice::new_mut(&mut self.output[..target.len()], 2, output_frames)?;
        self.resampler
            .as_mut()
            .unwrap()
            .process_into_buffer(&input, &mut output, None)?;
        for (out, frame) in target
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(self.output.as_chunks::<2>().0)
        {
            for channel in 0..2 {
                out[channel] += frame[channel] * gain[channel];
            }
        }
        Ok(())
    }
}
