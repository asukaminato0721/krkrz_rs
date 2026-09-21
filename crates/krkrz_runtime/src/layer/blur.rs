//! Box blur using imageproc summed-area tables and clipped neighborhoods.
use super::*;
use image::{GrayImage, Luma};
use imageproc::integral_image::{integral_image, sum_image_pixels};

impl Services {
    pub(super) fn layer_box_blur(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let radius = |i| -> Result<usize> {
            Ok(match args.get(i) {
                None | Some(Value::Void) => 1,
                Some(v) => int(v)?.unsigned_abs() as usize,
            })
        };
        let (rx, ry) = (radius(0)?, radius(1)?);
        let layer = &self.layers[&id];
        let source = layer.bitmap()?;
        let [left, top, right, bottom] = layer.clip.map(|v| v as usize);
        if right <= left || bottom <= top || rx == 0 && ry == 0 {
            return Ok(Value::Void);
        }
        let area = (rx as u64 * 2 + 1)
            .checked_mul(ry as u64 * 2 + 1)
            .context("Layer.doBoxBlur area overflow")?;
        ensure!(
            area < 1 << 24,
            "Layer.doBoxBlur area must be smaller than 16 million pixels"
        );
        let (width, height) = (source.width as usize, source.height as usize);
        let (start, end) = (left.saturating_sub(rx), (right + rx).min(width));
        let (first, last) = (top.saturating_sub(ry), (bottom + ry).min(height));
        let (columns, rows) = (end - start, last - first);
        let pixels = (right - left) * (bottom - top);
        ensure!(
            (columns + 1) * (rows + 1) * 8 + columns * rows + pixels * 4 <= MAX_LAYER_IMAGE_BYTES,
            "Layer.doBoxBlur temporary memory limit exceeded"
        );
        *budget = budget
            .checked_sub((columns * rows + pixels) as u64 * 4)
            .ok_or_else(|| unsupported("Layer.doBoxBlur execution budget exceeded"))?;
        let alpha = layer.draw_face() == 0;
        let mut output = vec![0; pixels * 4];
        // One channel at a time bounds table memory. imageproc owns the running
        // sum algorithm; this adapter supplies alpha semantics and clipped edges.
        for channel in 0..4 {
            let input = GrayImage::from_fn(columns as u32, rows as u32, |x, y| {
                let pos = (((first + y as usize) * width) + start + x as usize) * 4;
                let mut value = source.rgba[pos + channel] as u32;
                if alpha && channel != 3 {
                    let a = source.rgba[pos + 3] as u32;
                    value = (value * (a + (a >> 7))) >> 8;
                }
                Luma([value as u8])
            });
            let integral = integral_image::<_, u64>(&input);
            for y in top..bottom {
                for x in left..right {
                    let l = x.saturating_sub(rx);
                    let r = (x + rx + 1).min(width);
                    let t = y.saturating_sub(ry);
                    let b = (y + ry + 1).min(height);
                    let count = ((r - l) * (b - t)) as u64;
                    let sum = sum_image_pixels(
                        &integral,
                        (l - start) as u32,
                        (t - first) as u32,
                        (r - start - 1) as u32,
                        (b - first - 1) as u32,
                    )[0];
                    output[((y - top) * (right - left) + x - left) * 4 + channel] =
                        ((sum + count / 2) / count) as u8;
                }
            }
        }
        if alpha {
            for pixel in output.as_chunks_mut::<4>().0 {
                let a = pixel[3] as u32;
                for color in &mut pixel[..3] {
                    *color = (*color as u32 * 255).checked_div(a).unwrap_or(0).min(255) as u8;
                }
            }
        }
        let available = image_available(&self.layers, id);
        let layer = self.layers.get_mut(&id).unwrap();
        layer.prepare_image_write(available)?;
        let image = Arc::make_mut(layer.image.as_mut().unwrap());
        for (row, data) in output.chunks_exact((right - left) * 4).enumerate() {
            let pos = ((top + row) * width + left) * 4;
            image.rgba[pos..pos + data.len()].copy_from_slice(data);
        }
        layer.image_modified = true;
        Ok(Value::Void)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_budget_does_not_modify_image() {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        let mut session =
            crate::Session::open(project.path(), Some(saves.path()), None, 1000).unwrap();
        let pixels = vec![17, 31, 57, 255, 91, 0, 11, 32];
        session.services.layers.insert(
            123,
            Layer {
                image: Some(Arc::new(Image {
                    width: 2,
                    height: 1,
                    rgba: pixels.clone(),
                })),
                clip: [0, 0, 2, 1],
                image_modified: false,
                ..Layer::default()
            },
        );
        assert!(session.services.layer_box_blur(123, &[], &mut 0).is_err());
        assert_eq!(
            session.services.layers[&123].image.as_ref().unwrap().rgba,
            pixels
        );
        assert!(!session.services.layers[&123].image_modified);
    }
}
