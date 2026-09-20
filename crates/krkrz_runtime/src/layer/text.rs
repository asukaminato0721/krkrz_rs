use super::*;
use crate::fonts::TextGlyph;
use std::{collections::BTreeMap, sync::Arc};

fn charge(budget: &mut u64, amount: u64) -> Result<()> {
    *budget = budget
        .checked_sub(amount)
        .ok_or_else(|| unsupported("Layer text execution budget exceeded"))?;
    Ok(())
}

impl Services {
    pub(super) fn layer_draw_text(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(args.len() >= 4, "Layer.drawText: missing arguments");
        let option = |i: usize, default| -> Result<i32> {
            Ok(args
                .get(i)
                .filter(|v| !matches!(v, Value::Void))
                .map(int)
                .transpose()?
                .unwrap_or(default))
        };
        let (x, y) = (int(&args[0])? as i64, int(&args[1])? as i64);
        let Value::String(text) = args[2].unary("string")? else {
            unreachable!()
        };
        let color = rgb(args[3].integer()? as u32)?;
        let mut opacity = option(4, 255)?;
        let aa = option(5, 1)? != 0;
        let shadow_level = option(6, 0)?;
        let shadow_color = rgb(option(7, 0)? as u32)?;
        let shadow_width = option(8, 0)?;
        let shadow_offset = [option(9, 0)? as i64, option(10, 0)? as i64];
        let layer = &self.layers[&id];
        let face = layer.draw_face();
        ensure!(matches!(face, 0 | 1 | 4), "invalid drawText draw face");
        ensure!(
            face != 4 || opacity >= 0,
            "negative opacity is not supported on additive alpha face"
        );
        opacity = opacity.clamp(if face == 0 { -255 } else { 0 }, 255);
        layer.bitmap()?;
        if opacity == 0 || text.is_empty() || text[0] == 0 {
            return Ok(Value::Void);
        }
        ensure!(
            shadow_width.unsigned_abs() <= 64,
            "text shadow radius exceeds limit"
        );
        ensure!(
            (0..=65535).contains(&shadow_level),
            "text shadow level is outside supported range"
        );
        charge(budget, text.len() as u64)?;
        let font = layer
            .font
            .as_ref()
            .map(object)
            .transpose()?
            .flatten()
            .and_then(|id| self.font_objects.get(&id))
            .cloned()
            .unwrap_or_default();
        let mut raster = self.fonts.text_rasterizer(&font)?;
        let mut cache = BTreeMap::<u16, (Arc<TextGlyph>, Option<Arc<TextGlyph>>)>::new();
        let mut draws = Vec::new();
        let (mut x, mut y) = (x, y);
        let mut bytes = 0usize;
        for &unit in text.iter().take_while(|u| **u != 0) {
            if let std::collections::btree_map::Entry::Vacant(entry) = cache.entry(unit) {
                let glyph = raster.glyph(unit, aa)?;
                charge(budget, glyph.coverage.len() as u64)?;
                bytes += glyph.coverage.len();
                let glyph = Arc::new(glyph);
                let shadow = if shadow_level == 0 || glyph.coverage.is_empty() {
                    None
                } else if shadow_level == 255 && shadow_width == 0 {
                    Some(glyph.clone())
                } else {
                    let shadow = glyph.shadow(shadow_width, shadow_level, budget)?;
                    bytes += shadow.coverage.len();
                    Some(Arc::new(shadow))
                };
                ensure!(bytes <= 32 << 20, "text glyph memory limit exceeded");
                entry.insert((glyph, shadow));
            }
            let (glyph, shadow) = &cache[&unit];
            draws.push((x, y, glyph.clone(), shadow.clone()));
            x += glyph.advance[0] as i64;
            y += glyph.advance[1] as i64;
        }
        let layer = self.layers.get_mut(&id).unwrap();
        // All shadows precede all foreground glyphs, including overlapping characters.
        for (x, y, _, shadow) in &draws {
            if let Some(shadow) = shadow {
                layer.draw_glyph(
                    shadow,
                    x + shadow_offset[0],
                    y + shadow_offset[1],
                    shadow_color,
                    opacity,
                    budget,
                )?;
            }
        }
        for (x, y, glyph, _) in &draws {
            layer.draw_glyph(glyph, *x, *y, color, opacity, budget)?;
        }
        Ok(Value::Void)
    }
}

impl TextGlyph {
    fn shadow(&self, radius: i32, level: i32, budget: &mut u64) -> Result<Self> {
        let mut result = self.clone();
        if radius == 0 {
            charge(budget, self.coverage.len() as u64)?;
            for p in &mut result.coverage {
                *p = ((*p as i64 * level as i64) >> 8).min(64) as u8;
            }
            return Ok(result);
        }
        let radius = radius.unsigned_abs() as usize;
        result.width += 2 * radius;
        result.height += 2 * radius;
        result.left -= radius as i32;
        result.top -= radius as i32;
        ensure!(
            result.width * result.height <= 16_000_000,
            "text shadow bitmap exceeds size limit"
        );
        let mut kernel = Vec::new();
        let mut total = 0i64;
        for y in -(radius as i32)..=radius as i32 {
            for x in -(radius as i32)..=radius as i32 {
                // Kirikiri's integer approximation to the Euclidean radius.
                let (a, b) = (x.abs().max(y.abs()), x.abs().min(y.abs()));
                let t = b + (b >> 1);
                let distance = a - (a >> 5) - (a >> 7) + (t >> 2) + (t >> 6);
                if distance <= radius as i32 {
                    let weight = (radius as i32 - distance + 1) as i64;
                    total += weight;
                    kernel.push((x, y, weight));
                }
            }
        }
        charge(budget, self.coverage.len() as u64 * kernel.len() as u64)?;
        result.coverage = vec![0; result.width * result.height];
        let scale = (1 << 18) / total;
        for (dx, dy, weight) in kernel {
            let weight = (weight * scale * level as i64) >> 8;
            for y in 0..self.height {
                for x in 0..self.width {
                    let dest = ((y as i32 + dy + radius as i32) as usize) * result.width
                        + (x as i32 + dx + radius as i32) as usize;
                    result.coverage[dest] = (result.coverage[dest] as i64
                        + ((self.coverage[y * self.width + x] as i64 * weight) >> 18))
                        .min(64) as u8;
                }
            }
        }
        Ok(result)
    }
}

impl Layer {
    fn draw_glyph(
        &mut self,
        glyph: &TextGlyph,
        x: i64,
        y: i64,
        color: [u8; 3],
        opacity: i32,
        budget: &mut u64,
    ) -> Result<()> {
        let (x, y) = (x + glyph.left as i64, y + glyph.top as i64);
        let image = self.bitmap()?;
        let left = x.max(self.clip[0] as i64).max(0);
        let top = y.max(self.clip[1] as i64).max(0);
        let right = (x + glyph.width as i64)
            .min(self.clip[2] as i64)
            .min(image.width as i64);
        let bottom = (y + glyph.height as i64)
            .min(self.clip[3] as i64)
            .min(image.height as i64);
        if right <= left || bottom <= top {
            return Ok(());
        }
        charge(budget, ((right - left) * (bottom - top)) as u64)?;
        let face = self.draw_face();
        let image = self.image.as_mut().unwrap();
        for dy in top..bottom {
            for dx in left..right {
                let sample = (dy - y) as usize * glyph.width + (dx - x) as usize;
                let coverage = glyph.coverage[sample] as i32;
                // The historical MMX alpha routine skips fully empty pixel pairs.
                // Its scalar tail still performs table lookup for zero coverage.
                if opacity == 255 && face == 0 && coverage == 0 {
                    let column = (dx - left) as usize;
                    if column < (right - left) as usize / 2 * 2 {
                        let partner = if column & 1 == 0 {
                            sample + 1
                        } else {
                            sample - 1
                        };
                        if glyph.coverage[partner] == 0 {
                            continue;
                        }
                    }
                }
                let i = (dy as usize * image.width as usize + dx as usize) * 4;
                text_pixel(
                    &mut image.rgba[i..i + 4],
                    coverage,
                    color,
                    face,
                    self.hold_alpha,
                    opacity,
                );
            }
        }
        self.image_modified = true;
        Ok(())
    }
}

fn text_pixel(p: &mut [u8], coverage: i32, color: [u8; 3], face: i32, hold: bool, opacity: i32) {
    if opacity < 0 {
        let strength = -opacity;
        p[3] = if strength == 255 {
            ((p[3] as i32 * (64 - coverage)) >> 6) as u8
        } else {
            ((p[3] as i32 * (16384 - coverage * (strength + (strength >> 7)))) >> 14) as u8
        };
        return;
    }
    let a = if opacity == 255 {
        coverage
    } else {
        coverage * opacity >> 8
    };
    if face == 0 {
        let weight = text_alpha_weight(p[3], a as u8);
        for i in 0..3 {
            p[i] = (p[i] as i32 + ((color[i] as i32 - p[i] as i32) * weight >> 8)) as u8;
        }
        p[3] = (255 - (255 - p[3] as i32) * (255 - (a * 4).min(255)) / 255) as u8;
    } else if face == 1 {
        for i in 0..3 {
            p[i] = (p[i] as i32 + ((color[i] as i32 - p[i] as i32) * a >> 6)) as u8;
        }
        if !hold {
            p[3] = if opacity == 255 {
                (p[3] as i32 + (-(p[3] as i32) * a >> 6)) as u8
            } else {
                0
            };
        }
    } else {
        if opacity == 255 {
            for i in 0..3 {
                p[i] = (p[i] as i32 - (p[i] as i32 * a >> 6) + (color[i] as i32 * a >> 6)).min(255)
                    as u8;
            }
            p[3] = (p[3] as i32 + a * 4 - (p[3] as i32 * a >> 6)).min(255) as u8;
            return;
        }
        let alpha = (a * 4).min(255);
        for i in 0..3 {
            p[i] = ((p[i] as i32 * (255 - alpha) >> 8) + (color[i] as i32 * a >> 6)).min(255) as u8;
        }
        let d = p[3] as i32;
        p[3] = (d + alpha - (d * alpha >> 8)).min(255) as u8;
    }
}

fn text_alpha_weight(destination: u8, source: u8) -> i32 {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Box<[u8]>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = vec![0; 65 * 256];
        for b in 0..65 {
            for a in 0..256 {
                table[b * 256 + a] = if a == 0 {
                    255
                } else {
                    let at = (a as f64 / 255.0) as f32;
                    let bt = (b as f64 / 64.0) as f32;
                    let mut c = bt / at;
                    c /= (1.0 - bt as f64 + c as f64) as f32;
                    (c * 255.0).min(255.0) as u8
                };
            }
        }
        table.into_boxed_slice()
    });
    table[source as usize * 256 + destination as usize] as i32
}
