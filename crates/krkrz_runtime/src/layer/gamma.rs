//! Kirikiri gamma tables and straight/additive-alpha application.
use super::*;

fn gamma_table(gamma: f32, floor: i32, ceiling: i32) -> [u8; 256] {
    let inverse = 1.0 / gamma as f64;
    let amplitude = ceiling.wrapping_sub(floor) as f64;
    std::array::from_fn(|i| {
        let value = ((i as f64 / 255.0).ln() * inverse).exp() * amplitude + 0.5 + floor as f64;
        // The native x86 floating-to-integer conversion returns INT_MIN for
        // non-finite or out-of-range results, then the table clamps to 0..255.
        let value = if value.is_finite() && value >= i32::MIN as f64 && value < 2147483648.0 {
            value as i32
        } else {
            i32::MIN
        };
        value.clamp(0, 255) as u8
    })
}

impl Services {
    pub(super) fn layer_gamma(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        if args.is_empty() {
            return Ok(Value::Void);
        }
        let mut parameters = [(1.0_f32, 0_i32, 255_i32); 3];
        for (channel, (gamma, floor, ceiling)) in parameters.iter_mut().enumerate() {
            for field in 0..3 {
                if let Some(value) = args
                    .get(channel * 3 + field)
                    .filter(|v| !matches!(v, Value::Void))
                {
                    match field {
                        0 => {
                            *gamma = value.real()? as f32;
                            ensure!(gamma.is_finite(), "Layer.adjustGamma gamma must be finite");
                        }
                        1 => *floor = value.integer()? as i32,
                        _ => *ceiling = value.integer()? as i32,
                    }
                }
            }
        }
        let layer = &self.layers[&id];
        let image = layer.bitmap()?;
        let additive = layer.draw_face() == 4;
        let left = layer.clip[0].max(0) as usize;
        let top = layer.clip[1].max(0) as usize;
        let right = layer.clip[2].max(0).min(image.width as i32) as usize;
        let bottom = layer.clip[3].max(0).min(image.height as i32) as usize;
        if parameters != [(1.0, 0, 255); 3] && right > left && bottom > top {
            *budget = budget
                .checked_sub(((right - left) * (bottom - top) + 768) as u64)
                .ok_or_else(|| unsupported("Layer.adjustGamma execution budget exceeded"))?;
            let tables = parameters.map(|(g, f, c)| gamma_table(g, f, c));
            let available = image_available(&self.layers, id);
            let layer = self.layers.get_mut(&id).unwrap();
            layer.prepare_image_write(available)?;
            let image = Arc::make_mut(layer.image.as_mut().unwrap());
            for y in top..bottom {
                let start = (y * image.width as usize + left) * 4;
                for pixel in image.rgba[start..start + (right - left) * 4]
                    .as_chunks_mut::<4>()
                    .0
                {
                    let alpha = pixel[3] as u32;
                    if additive && alpha < 255 {
                        let adjusted = alpha + (alpha >> 7);
                        let reciprocal = 65536_u32.checked_div(alpha).unwrap_or(32767).min(32767);
                        for c in 0..3 {
                            let color = pixel[c] as u32;
                            pixel[c] = if color > alpha {
                                (((tables[c][255] as u32 * adjusted) >> 8) + color - alpha) as u8
                            } else {
                                ((tables[c][((reciprocal * color) >> 8) as usize] as u32
                                    * adjusted)
                                    >> 8) as u8
                            };
                        }
                    } else if alpha != 0 {
                        for c in 0..3 {
                            pixel[c] = tables[c][pixel[c] as usize];
                        }
                    }
                }
            }
        }
        self.layers.get_mut(&id).unwrap().image_modified = true;
        Ok(Value::Void)
    }
}
