//! fftgraph/Main.cpp from krkr2 (see docs/references.json).
//! The original's sine transform is computed with RealFFT over an odd extension.
use crate::{Services, layer};
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm, unsupported};
use realfft::{RealFftPlanner, RealToComplex, num_complex::Complex32};
use std::{f32::consts::PI, sync::Arc};

const SAMPLES: usize = 2048;

#[derive(Default)]
pub(crate) struct State {
    dsp: Option<Dsp>,
    cut: f32,
    bands: Vec<Band>,
}
#[derive(Default)]
struct Band {
    start: usize,
    end: usize,
    value: i32,
    peak: i32,
    age: i32,
}
struct Dsp {
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<Complex32>,
    scratch: Vec<Complex32>,
    window: Vec<f32>,
    rotation: Vec<Complex32>,
    spectrum: Vec<f32>,
}
impl Dsp {
    fn new() -> Self {
        let fft = RealFftPlanner::new().plan_fft_forward(SAMPLES * 2);
        Self {
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window: (0..SAMPLES)
                .map(|i| {
                    ((std::f64::consts::PI * (i as f64 + 0.5) / SAMPLES as f64).sin()
                        * (4.0 / 32768.0 / SAMPLES as f64)) as f32
                })
                .collect(),
            rotation: (0..SAMPLES)
                .map(|i| Complex32::from_polar(1.0, -PI * i as f32 / (2 * SAMPLES) as f32))
                .collect(),
            spectrum: vec![0.0; SAMPLES],
        }
    }
    fn transform(&mut self, samples: &[i16]) -> Result<()> {
        for (i, sample) in samples.iter().enumerate() {
            let v = *sample as f32 * self.window[i];
            self.input[i] = v;
            self.input[2 * SAMPLES - 1 - i] = -v;
        }
        self.fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)?;
        for i in 2..SAMPLES {
            self.spectrum[i] = -0.5 * (self.output[i] * self.rotation[i]).im;
        }
        // Native ddst puts its final bin at zero; the first two bins are discarded.
        self.spectrum[0..2].fill(0.0);
        Ok(())
    }
}
impl State {
    fn bands(
        &mut self,
        count: usize,
        cut: f32,
        maximum: i32,
        fall: i32,
        hold: i32,
        peak_fall: i32,
    ) {
        if self.bands.len() != count || self.cut != cut {
            self.cut = cut;
            self.bands = (0..count)
                .map(|i| {
                    let edge = |i: usize| {
                        let exponent = (i as f32 * ((cut - 1.0) / cut) / count as f32) as f64
                            + 1.0 / cut as f64;
                        (SAMPLES as f64).powf(exponent as f32 as f64) as usize
                    };
                    let start = edge(i).min(SAMPLES);
                    let end = edge(i + 1).max(start + 1).min(SAMPLES);
                    Band {
                        start,
                        end,
                        ..Band::default()
                    }
                })
                .collect();
        }
        let spectrum = &self.dsp.as_ref().unwrap().spectrum;
        for band in &mut self.bands {
            let amplitude = spectrum[band.start..band.end]
                .iter()
                .map(|v| v.abs())
                .fold(0.0f32, f32::max);
            let db = if amplitude == 0.0 {
                -70.0
            } else {
                ((amplitude * amplitude) as f64).log10() as f32 * 10.0
            }
            .clamp(-70.0, 0.0);
            let value = (maximum as f32 - (maximum as f32 / -70.0) * db) as i32;
            band.value = (band.value - fall).max(0).max(value).min(maximum);
            if band.age == hold {
                band.peak = (band.peak - peak_fall).max(0);
            } else {
                band.age += 1;
            }
            if band.peak < value {
                band.peak = value;
                band.age = 0;
            }
        }
    }
}

impl Services {
    fn fft_option(
        &mut self,
        vm: &mut Vm,
        options: &Value,
        name: &str,
        default: i32,
        budget: &mut u64,
    ) -> Result<i32> {
        if matches!(options, Value::Void)
            || matches!(options, Value::Object(reference) if reference.object.is_none())
        {
            return Ok(default);
        }
        let key = Value::string(name);
        let value = vm.get_property(options, &key, true, false, self, budget)?;
        if matches!(value, Value::Void) && !vm.has_member(options, &key)? {
            Ok(default)
        } else {
            Ok(value.integer()? as i32)
        }
    }

    pub(crate) fn draw_fft_graph(
        &mut self,
        vm: &mut Vm,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(args.len() >= 6, "drawFFTGraph requires six arguments");
        let Value::Object(reference) = &args[0] else {
            anyhow::bail!("drawFFTGraph requires a Layer");
        };
        let id = reference
            .object
            .context("drawFFTGraph requires a non-null Layer")?;
        let left = args[2].integer()? as i32;
        let top = args[3].integer()? as i32;
        let width = args[4].integer()? as i32;
        let height = args[5].integer()? as i32;
        ensure!(
            left >= 0 && top >= 0 && width > 0 && height > 0,
            "invalid FFT graph rectangle"
        );
        let validate = |layers: &std::collections::BTreeMap<usize, layer::Layer>| -> Result<()> {
            let image = layers
                .get(&id)
                .context("drawFFTGraph target has no Layer instance")?
                .image
                .as_ref()
                .context("FFT graph layer has no image")?;
            ensure!(
                left as u64 + width as u64 <= image.width as u64
                    && top as u64 + height as u64 <= image.height as u64,
                "FFT graph rectangle exceeds layer image"
            );
            Ok(())
        };
        validate(&self.layers)?;
        *budget = budget
            .checked_sub(width as u64 * height as u64 + (SAMPLES * 12) as u64)
            .ok_or_else(|| unsupported("FFT graph execution budget exceeded"))?;
        let (mut samples, written) =
            self.read_vis_buffer(vm, &args[1], SAMPLES as i32, Some(0), budget)?;
        if (written.integer()? as i32) < SAMPLES as i32 {
            samples.fill(0);
        }
        let options = args.get(6).unwrap_or(&Value::Void);
        let kind = self.fft_option(vm, options, "type", 0, budget)?;
        let mut division = 16;
        let mut thick = 2;
        let mut colors = [0xff000000u32, 0xffb0b0b0, 0xffc0c0c0, 0xff707070];
        if kind == 1 {
            division = self.fft_option(vm, options, "division", division, budget)?;
            thick = self.fft_option(vm, options, "thick", thick, budget)?;
            for (color, name) in
                colors
                    .iter_mut()
                    .zip(["oncolor", "offcolor", "bgcolor", "peakcolor"])
            {
                *color = self.fft_option(vm, options, name, *color as i32, budget)? as u32;
            }
            ensure!(
                division > 0 && division <= width && thick > 0 && thick <= height,
                "invalid FFT graph division or thickness"
            );
        }
        // Script getters/callbacks can resize or invalidate the target.
        validate(&self.layers)?;
        let available = layer::image_available(&self.layers, id);
        let image = self
            .layers
            .get_mut(&id)
            .unwrap()
            .plugin_image_mut(available)?;
        let state = &mut self.fftgraph;
        state.dsp.get_or_insert_with(Dsp::new).transform(&samples)?;
        let mut pixel = |x: i32, y: i32, color: u32| {
            // A previous call may have held a peak for a taller graph.
            if y < 0 || y >= height {
                return;
            }
            let offset =
                ((top + height - 1 - y) as usize * image.width as usize + (left + x) as usize) * 4;
            let [a, r, g, b] = color.to_be_bytes();
            image.rgba[offset..offset + 4].copy_from_slice(&[r, g, b, a]);
        };
        match kind {
            0 => {
                const FIRE: [u32; 16] = [
                    0xff20ff00, 0xff40ff00, 0xff60ff00, 0xff80ff00, 0xffa0ff00, 0xffc0ff00,
                    0xffe0ff00, 0xffffff00, 0xffffe000, 0xffffc000, 0xffffa000, 0xffff8000,
                    0xffff6000, 0xffff4000, 0xffff2000, 0xffffff00,
                ];
                state.bands(
                    width as usize,
                    3.7,
                    height - 1,
                    (height + 15) / 16,
                    30,
                    (height + 31) / 32,
                );
                for (x, band) in state.bands.iter().enumerate() {
                    for y in 0..height {
                        let c = 15 - band.value + y;
                        let color = if y >= band.value {
                            0
                        } else if c < 0 {
                            0xff00ff00
                        } else {
                            FIRE[c as usize]
                        };
                        pixel(x as i32, y, color);
                    }
                    pixel(x as i32, band.peak, 0xff808080);
                }
            }
            1 => {
                let step = 1000 / (height / thick);
                // Bar levels use a fixed scale independent of image height.
                state.bands(division as usize, 8.0, 1000 - step * 2, 40, 40, 10);
                let bw = width / division;
                let [on, off, background, peak] = colors;
                for (i, band) in state.bands.iter().enumerate() {
                    let mut peak_drawn = false;
                    for row in 0..height / thick {
                        let value = row * step;
                        let color = if !peak_drawn && band.peak <= value && peak != off {
                            peak_drawn = true;
                            peak
                        } else if band.value >= value {
                            on
                        } else {
                            off
                        };
                        for y in 0..thick {
                            for x in 0..bw {
                                pixel(
                                    i as i32 * bw + x,
                                    row * thick + y,
                                    if y == 0 || x == 0 { background } else { color },
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        let update = vm.get_member(&args[0], &Value::string("update"), false)?;
        vm.call_function(&update, &args[0], &args[2..6], self, budget)?;
        Ok(Value::Void)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_matches_direct_sine_sum() {
        let samples: Vec<i16> = (0..SAMPLES)
            .map(|i| ((i * 7919 % 65536) as i32 - 32768) as i16)
            .collect();
        let mut dsp = Dsp::new();
        dsp.transform(&samples).unwrap();
        for k in [2, 3, 17, 127, 1024, 2047] {
            let expected: f64 = samples
                .iter()
                .enumerate()
                .map(|(i, sample)| {
                    let window = (std::f64::consts::PI * (i as f64 + 0.5) / SAMPLES as f64).sin()
                        * 4.0
                        / 32768.0
                        / SAMPLES as f64;
                    *sample as f64
                        * window
                        * (std::f64::consts::PI * (i as f64 + 0.5) * k as f64 / SAMPLES as f64)
                            .sin()
                })
                .sum();
            assert!(
                (dsp.spectrum[k] as f64 - expected).abs() < 1e-6,
                "bin {k}: {} != {expected}",
                dsp.spectrum[k]
            );
        }
        assert_eq!(&dsp.spectrum[..2], &[0.0, 0.0]);
    }
}
