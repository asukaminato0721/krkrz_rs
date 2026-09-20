//! Area averaging with the original shrinkCopy plugin's 1/256 pixel weights.
//! Algorithm by miahmie; see THIRD_PARTY_NOTICES.md and docs/references.json.
use super::*;

struct Sample {
    start: i64,
    step: i64,
    first: [u32; 2], // alpha coverage, color weight
    last: [u32; 2],
    total: u32,
}
impl Sample {
    fn points(&self) -> impl Iterator<Item = (i64, [u32; 2])> + '_ {
        std::iter::once((self.start, self.first))
            .chain((0..self.step).map(|i| (self.start + 1 + i, [256; 2])))
            .chain(std::iter::once((self.start + 1 + self.step, self.last)))
            .filter(|(_, weight)| weight[0] != 0)
    }
}
struct Axis {
    offset: i64,
    samples: Vec<Sample>,
}
fn axis(
    mut dest: f64,
    mut size: f64,
    mut src: i64,
    mut count: i64,
    source: u32,
    target: u32,
) -> Axis {
    let ratio = size / count as f64;
    if src < 0 {
        count += src;
        let cut = ratio * -src as f64;
        size -= cut;
        dest += cut;
        src = 0;
    }
    let cut = (src + count - source as i64).max(0);
    count -= cut;
    size -= ratio * cut as f64;
    let origin = dest.trunc() as i64;
    let extent = (dest + size).ceil() as i64 - origin;
    let begin = (-origin).max(0);
    let end = extent.min(target as i64 - origin);
    let mut samples = Vec::new();
    if count <= 0 || size <= 0.0 || end <= begin {
        return Axis { offset: 0, samples };
    }
    let ratio = count as f64 / size;
    for pos in begin..end {
        let a = (pos as f64 - (dest - origin as f64)) * ratio;
        let b = a + ratio;
        let (ta, tb) = (a.trunc() as i64, b.trunc() as i64);
        let (fa, fb) = if pos == begin || pos == end - 1 {
            (a.max(0.0), b.min(count as f64))
        } else {
            (a, b)
        };
        let (ua, ub) = (fa.trunc() as i64, fb.trunc() as i64);
        let first_color = 256 - ((a - ta as f64) * 256.0).trunc() as i64;
        let last_color = ((b - tb as f64) * 256.0).trunc() as i64;
        samples.push(Sample {
            start: src + ua,
            step: ub - ua - 1,
            first: [
                (256 - ((fa - ua as f64) * 256.0).trunc() as i64) as u32,
                first_color as u32,
            ],
            last: [((fb - ub as f64) * 256.0).trunc() as u32, last_color as u32],
            total: (first_color + last_color + (tb - ta - 1) * 256) as u32,
        });
    }
    Axis {
        offset: origin + begin,
        samples,
    }
}
impl Services {
    pub(super) fn layer_shrink_copy(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(args.len() >= 9, "Layer.shrinkCopy: missing arguments");
        let d: Vec<f64> = args[..4].iter().map(Value::real).collect::<Result<_>>()?;
        ensure!(
            d.iter()
                .all(|x| x.is_finite() && x.abs() <= i32::MAX as f64),
            "invalid shrinkCopy destination"
        );
        let source_id = object(&args[4])?.context("shrinkCopy source is null")?;
        let source = self
            .layers
            .get(&source_id)
            .context("shrinkCopy source is not a Layer")?
            .bitmap()?;
        let dest = self.layers[&id].bitmap()?;
        let (sx, sy, sw, sh) = (
            int(&args[5])? as i64,
            int(&args[6])? as i64,
            int(&args[7])? as i64,
            int(&args[8])? as i64,
        );
        ensure!(
            sw > 0 && sh > 0 && d[2] > 0.0 && d[3] > 0.0 && sw >= d[2] as i64 && sh >= d[3] as i64,
            "invalid shrinkCopy size"
        );
        // Bound tables as well as pixels, including very thin source images.
        let columns = (d[2].ceil() as usize + 1).min(dest.width as usize);
        let rows = (d[3].ceil() as usize + 1).min(dest.height as usize);
        ensure!(
            (columns + rows) * std::mem::size_of::<Sample>() + columns * rows * 4 <= 256 << 20,
            "shrinkCopy temporary memory limit exceeded"
        );
        *budget = budget
            .checked_sub((columns + rows) as u64)
            .ok_or_else(|| unsupported("shrinkCopy execution budget exceeded"))?;
        let x = axis(d[0], d[2], sx, sw, source.width, dest.width);
        let y = axis(d[1], d[3], sy, sh, source.height, dest.height);
        let mut output = Vec::with_capacity(x.samples.len() * y.samples.len() * 4);
        for vy in &y.samples {
            for hx in &x.samples {
                let divisor = hx.total.wrapping_mul(vy.total);
                ensure!(divisor != 0, "shrinkCopy weight overflow");
                let cost = (hx.step.max(0) as u64 + 2) * (vy.step.max(0) as u64 + 2);
                *budget = budget
                    .checked_sub(cost)
                    .ok_or_else(|| unsupported("shrinkCopy execution budget exceeded"))?;
                let mut sum = [0u32; 4];
                for (py, wy) in vy.points() {
                    for (px, wx) in hx.points() {
                        ensure!(
                            px >= 0
                                && py >= 0
                                && px < source.width as i64
                                && py < source.height as i64,
                            "shrinkCopy sample outside image"
                        );
                        let i = (py as usize * source.width as usize + px as usize) * 4;
                        for (c, total) in sum.iter_mut().enumerate() {
                            let k = usize::from(c != 3);
                            *total = total.wrapping_add(
                                (source.rgba[i + c] as u32).wrapping_mul(wx[k].wrapping_mul(wy[k])),
                            );
                        }
                    }
                }
                output.extend(sum.map(|s| (s / divisor) as u8));
            }
        }
        let dst = self.layers.get_mut(&id).unwrap();
        let bitmap = dst.image.as_mut().unwrap();
        let width = x.samples.len() * 4;
        if width > 0 {
            for (row, data) in output.chunks_exact(width).enumerate() {
                let start =
                    ((y.offset as usize + row) * bitmap.width as usize + x.offset as usize) * 4;
                bitmap.rgba[start..start + width].copy_from_slice(data);
            }
            dst.image_modified = true;
        }
        Ok(Value::Void)
    }
}
