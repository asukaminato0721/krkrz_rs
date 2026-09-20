//! Box blur with Kirikiri's clipped neighborhood and integer alpha arithmetic.
use super::*;

fn components(pixel: &[u8], alpha: bool) -> [u64; 4] {
    let mut value = std::array::from_fn(|i| pixel[i] as u64);
    if alpha {
        let scale = value[3] + (value[3] >> 7);
        for color in &mut value[..3] {
            *color = (*color * scale) >> 8;
        }
    }
    value
}

fn average(sum: [u64; 4], count: u64, small: bool, center: bool, alpha: bool) -> [u8; 4] {
    let mut value = sum.map(|v| {
        if small {
            (((v + count / 2) * (65536 / count)) >> 16) as u8
        } else if center {
            // The original optimized center loop uses a fixed-point reciprocal;
            // its boundary loop divides directly. Exact ties can differ by one.
            (((v + count / 2) * ((1u64 << 32) / count)) >> 32) as u8
        } else {
            ((v + count / 2) / count) as u8
        }
    });
    if alpha {
        let a = value[3] as u64;
        for color in &mut value[..3] {
            *color = (*color as u64 * 255).checked_div(a).unwrap_or(0).min(255) as u8;
        }
    }
    value
}

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
        let columns = end - start;
        let rows = bottom - top;
        let pixels = (right - left) * rows;
        ensure!(
            columns * 32 + pixels * 4 <= MAX_LAYER_IMAGE_BYTES,
            "Layer.doBoxBlur temporary memory limit exceeded"
        );
        let cost = columns as u64
            * ((top + ry + 1).min(height) - top.saturating_sub(ry) + 3 * rows) as u64
            + pixels as u64;
        *budget = budget
            .checked_sub(cost)
            .ok_or_else(|| unsupported("Layer.doBoxBlur execution budget exceeded"))?;
        let alpha = layer.draw_face() == 0;
        let mut vertical = vec![[0u64; 4]; columns];
        let mut output = Vec::with_capacity(pixels * 4);
        let update = |vertical: &mut [[u64; 4]], y: usize, add: bool| {
            for (x, sum) in vertical.iter_mut().enumerate() {
                let pos = (y * width + start + x) * 4;
                let value = components(&source.rgba[pos..pos + 4], alpha);
                for c in 0..4 {
                    if add {
                        sum[c] += value[c];
                    } else {
                        sum[c] -= value[c];
                    }
                }
            }
        };
        for y in top.saturating_sub(ry)..(top + ry + 1).min(height) {
            update(&mut vertical, y, true);
        }
        for y in top..bottom {
            let mut sum = [0u64; 4];
            for column in &vertical[..(left + rx + 1).min(width) - start] {
                for c in 0..4 {
                    sum[c] += column[c];
                }
            }
            let vertical_count = (y + ry + 1).min(height) - y.saturating_sub(ry);
            for x in left..right {
                let count = vertical_count * ((x + rx + 1).min(width) - x.saturating_sub(rx));
                let center = x >= rx && x + rx + 1 < width;
                output.extend(average(sum, count as u64, area < 256, center, alpha));
                if x >= rx {
                    for c in 0..4 {
                        sum[c] -= vertical[x - rx - start][c];
                    }
                }
                if x + rx + 1 < end {
                    for c in 0..4 {
                        sum[c] += vertical[x + rx + 1 - start][c];
                    }
                }
            }
            if y + 1 < bottom {
                if y >= ry {
                    update(&mut vertical, y - ry, false);
                }
                if y + ry + 1 < height {
                    update(&mut vertical, y + ry + 1, true);
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
            crate::Session::open(project.path(), Some(saves.path()), false, 1000).unwrap();
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
