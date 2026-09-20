//! Separable resampling follows Kirikiri 1.2.0.3 visual/Resampler.cpp.
//! See THIRD_PARTY_NOTICES.md for source and license.
use super::*;
use std::ops::Range;

// The 1.2.0.3 engine uses the pre-April-2016 resampler, including its
// reflected borders, bell filter named stCubic, and horizontal-first rounding.
struct AxisSample {
    points: Vec<(usize, f32)>,
}
fn weight(mut x: f32, cubic: bool) -> f32 {
    x = x.abs();
    if !cubic {
        return (1.0 - x).max(0.0);
    }
    if x < 0.5 {
        (0.75f64 - (x * x) as f64) as f32
    } else if x < 1.5 {
        x = (x as f64 - 1.5) as f32;
        (0.5f64 * (x * x) as f64) as f32
    } else {
        0.0
    }
}
fn axis(
    source: i64,
    dest: i64,
    range: Range<i64>,
    cubic: bool,
    vertical: bool,
    budget: &mut u64,
) -> Result<Vec<AxisSample>> {
    let tap = if cubic { 1.5f32 } else { 1.0 };
    let scale = dest as f32 / source as f32;
    let radius = if scale < 1.0 { tap / scale } else { tap };
    let mut output = Vec::new();
    let mut allocated = 0;
    for i in range {
        let center = if vertical && scale < 1.0 {
            i as f32 * (1.0f64 / scale as f64) as f32
        } else {
            i as f32 / scale
        };
        let left = (center - radius).ceil() as i64;
        let right = (center + radius).floor() as i64;
        let count = (right - left + 1) as usize;
        allocated += count;
        ensure!(
            allocated <= (8 << 20),
            "stretchCopy filter memory limit exceeded"
        );
        *budget = budget
            .checked_sub(count as u64)
            .ok_or_else(|| unsupported("stretchCopy execution budget exceeded"))?;
        let mut points = Vec::with_capacity(count);
        let mut sum = 0.0f32;
        for j in left..=right {
            let distance = center - j as f32;
            let w = weight(
                if scale < 1.0 {
                    distance * scale
                } else {
                    distance
                },
                cubic,
            );
            sum += w;
            let n = if j < 0 {
                -j
            } else if j >= source {
                source * 2 - j - 1
            } else {
                j
            };
            points.push((n.clamp(0, source - 1) as usize, w));
        }
        if scale < 1.0 && sum != 0.0 {
            let reciprocal = (1.0f64 / sum as f64) as f32;
            for (_, w) in &mut points {
                *w *= reciprocal;
            }
        }
        output.push(AxisSample { points });
    }
    Ok(output)
}
fn sample(sample: &AxisSample, mut read: impl FnMut(usize) -> [u8; 4]) -> [u8; 4] {
    let first = read(sample.points[0].0);
    let mut changed = [false; 4];
    let mut channels = [0.0f32; 4];
    for &(pos, w) in sample.points.iter().rev() {
        let pixel = read(pos);
        for c in 0..4 {
            changed[c] |= pixel[c] != first[c];
            channels[c] += pixel[c] as f32 * w;
        }
    }
    std::array::from_fn(|c| {
        if changed[c] {
            channels[c].round().clamp(0.0, 255.0) as u8
        } else {
            first[c]
        }
    })
}
impl Services {
    pub(super) fn layer_stretch_copy(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(args.len() >= 9, "Layer.stretchCopy: missing arguments");
        let (dx, dy, dw, dh) = (
            int(&args[0])? as i64,
            int(&args[1])? as i64,
            int(&args[2])? as i64,
            int(&args[3])? as i64,
        );
        let (sx, sy, sw, sh) = (
            int(&args[5])? as i64,
            int(&args[6])? as i64,
            int(&args[7])? as i64,
            int(&args[8])? as i64,
        );
        let source = object(&args[4])?.context("stretchCopy source is null")?;
        let src = self
            .layers
            .get(&source)
            .context("stretchCopy source is not a Layer")?
            .bitmap()?;
        let dst = &self.layers[&id];
        let image = dst.bitmap()?;
        let clip = [
            dst.clip[0].max(0) as i64,
            dst.clip[1].max(0) as i64,
            (dst.clip[2] as i64).min(image.width as i64),
            (dst.clip[3] as i64).min(image.height as i64),
        ];
        if dw == 0
            || dh == 0
            || sw == 0
            || sh == 0
            || dx.min(dx + dw) >= clip[2]
            || dx.max(dx + dw) <= clip[0]
            || dy.min(dy + dh) >= clip[3]
            || dy.max(dy + dh) <= clip[1]
        {
            return Ok(Value::Void);
        }
        let face = dst.draw_face();
        ensure!(matches!(face, 0 | 1 | 4), "stretchCopy: invalid draw face");
        let mode = args.get(9).map(int).transpose()?.unwrap_or(0);
        let filter = mode & 15;
        // Unscaled blits keep the native clipping and overlap semantics.
        if dw == sw
            && dh == sh
            && dx >= clip[0]
            && dy >= clip[1]
            && dx + dw <= clip[2]
            && dy + dh <= clip[3]
        {
            return self.layer_blit(
                id,
                "copyRect",
                &[
                    args[0].clone(),
                    args[1].clone(),
                    args[4].clone(),
                    args[5].clone(),
                    args[6].clone(),
                    args[7].clone(),
                    args[8].clone(),
                ],
                budget,
            );
        }
        let hold = face == 1 && dst.hold_alpha;
        let resample = matches!(filter, 2 | 3)
            && !hold
            && dw > 0
            && dh > 0
            && sw > 0
            && sh > 0
            && dx >= clip[0]
            && dy >= clip[1]
            && dx + dw <= clip[2]
            && dy + dh <= clip[3];
        if !resample {
            let mut affine = vec![
                args[4].clone(),
                args[5].clone(),
                args[6].clone(),
                args[7].clone(),
                args[8].clone(),
                Value::Integer(0),
            ];
            affine.extend(
                [
                    dx as f64 - 0.5,
                    dy as f64 - 0.5,
                    (dx + dw) as f64 - 0.5,
                    dy as f64 - 0.5,
                    dx as f64 - 0.5,
                    (dy + dh) as f64 - 0.5,
                ]
                .map(Value::Real),
            );
            affine.push(Value::Integer(mode as i64));
            return self.layer_affine_copy(id, &affine, budget);
        }
        ensure!(
            sw > 0
                && sh > 0
                && sx >= 0
                && sy >= 0
                && sx + sw <= src.width as i64
                && sy + sh <= src.height as i64,
            "stretchCopy source rectangle outside image"
        );
        let cubic = filter == 3;
        let left = dx.max(clip[0]);
        let right = (dx + dw).min(clip[2]);
        let top = dy.max(clip[1]);
        let bottom = (dy + dh).min(clip[3]);
        let x_axis = axis(sw, dw, left - dx..right - dx, cubic, false, budget)?;
        let y_axis = axis(sh, dh, top - dy..bottom - dy, cubic, true, budget)?;
        let cost = x_axis
            .iter()
            .map(|s| s.points.len() as u64 * sh as u64)
            .sum::<u64>()
            + (right - left) as u64 * y_axis.iter().map(|s| s.points.len() as u64).sum::<u64>();
        *budget = budget
            .checked_sub(cost)
            .ok_or_else(|| unsupported("stretchCopy execution budget exceeded"))?;
        let mut output = vec![[0u8; 4]; (right - left) as usize * (bottom - top) as usize];
        let mut column = vec![[0u8; 4]; sh as usize];
        for (x, xs) in x_axis.iter().enumerate() {
            for (y, pixel) in column.iter_mut().enumerate() {
                *pixel = sample(xs, |i| {
                    let pos = ((sy as usize + y) * src.width as usize + sx as usize + i) * 4;
                    src.rgba[pos..pos + 4].try_into().unwrap()
                });
            }
            for (y, ys) in y_axis.iter().enumerate() {
                output[y * (right - left) as usize + x] = sample(ys, |i| column[i]);
            }
        }
        let dst = self.layers.get_mut(&id).unwrap();
        let image = dst.image.as_mut().unwrap();
        let mut pixels = output.iter();
        for y in top..bottom {
            for x in left..right {
                let i = (y as usize * image.width as usize + x as usize) * 4;
                let count = if hold { 3 } else { 4 };
                image.rgba[i..i + count].copy_from_slice(&pixels.next().unwrap()[..count]);
            }
        }
        dst.image_modified = true;
        Ok(Value::Void)
    }
}
