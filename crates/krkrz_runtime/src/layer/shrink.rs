//! Area downsampling through fast_image_resize, with the plugin's clipping rules.
use super::*;

struct Axis {
    offset: usize,
    count: u32,
    source: f64,
    extent: f64,
    dest: f64,
    size: f64,
}
fn axis(
    mut dest: f64,
    mut size: f64,
    mut src: i64,
    mut count: i64,
    source: u32,
    target: u32,
) -> Option<Axis> {
    let scale = size / count as f64;
    if src < 0 {
        count += src;
        let cut = scale * -src as f64;
        dest += cut;
        size -= cut;
        src = 0;
    }
    let cut = (src + count - source as i64).max(0);
    count -= cut;
    size -= scale * cut as f64;
    if count <= 0 || size <= 0.0 {
        return None;
    }
    let begin = dest.trunc().max(0.0).min(target as f64);
    let end = (dest + size).ceil().min(target as f64);
    if end <= begin {
        return None;
    }
    let first = ((begin - dest) / scale).clamp(0.0, count as f64);
    let last = ((end - dest) / scale).clamp(0.0, count as f64);
    if last <= first {
        return None;
    }
    Some(Axis {
        offset: begin as usize,
        count: (end - begin) as u32,
        source: src as f64 + first,
        extent: last - first,
        dest,
        size,
    })
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
        // shrinkCopy writes the bitmap, ignoring draw face and Layer.clip.
        let Some(x) = axis(d[0], d[2], sx, sw, source.width, dest.width) else {
            return Ok(Value::Void);
        };
        let Some(y) = axis(d[1], d[3], sy, sh, source.height, dest.height) else {
            return Ok(Value::Void);
        };
        let mut output = super::stretch::resize(
            source,
            [x.source, y.source, x.extent, y.extent],
            x.count,
            y.count,
            fast_image_resize::FilterType::Box,
            budget,
        )?;
        // Fractional destination edges cover only part of the boundary pixel.
        // Preserve coverage in alpha instead of stretching an opaque edge across
        // the entire enclosing integer rectangle.
        let coverage = |axis: &Axis, i: usize| {
            let pos = (axis.offset + i) as f64;
            ((pos + 1.0).min(axis.dest + axis.size) - pos.max(axis.dest)).clamp(0.0, 1.0)
        };
        for (i, pixel) in output.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            pixel[3] = (pixel[3] as f64
                * coverage(&x, i % x.count as usize)
                * coverage(&y, i / x.count as usize))
            .round() as u8;
        }
        let available = image_available(&self.layers, id);
        let dst = self.layers.get_mut(&id).unwrap();
        dst.prepare_image_write(available)?;
        let bitmap = Arc::make_mut(dst.image.as_mut().unwrap());
        let width = x.count as usize * 4;
        if width > 0 {
            for (row, data) in output.chunks_exact(width).enumerate() {
                let start = ((y.offset + row) * bitmap.width as usize + x.offset) * 4;
                bitmap.rgba[start..start + width].copy_from_slice(data);
            }
            dst.image_modified = true;
        }
        Ok(Value::Void)
    }
}
