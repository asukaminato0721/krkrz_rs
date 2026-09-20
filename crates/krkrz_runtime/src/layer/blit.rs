use super::*;

impl Services {
    pub(super) fn layer_blit(
        &mut self,
        id: usize,
        op: &str,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(args.len() >= 7, "Layer.{op}: missing arguments");
        let source = object(&args[2])?.context("copy source is null")?;
        let source = self
            .layers
            .get(&source)
            .context("copy source is not a Layer")?;
        let src = source.bitmap()?;
        let dst = &self.layers[&id];
        let face = dst.draw_face();
        let mode = match args
            .get(7)
            .filter(|v| !matches!(v, Value::Void))
            .map(int)
            .transpose()?
            .unwrap_or(128)
        {
            128 => match source.kind {
                2..=5 | 8..=28 => source.kind,
                _ => 1,
            },
            mode => mode,
        };
        let opacity = args
            .get(8)
            .filter(|v| !matches!(v, Value::Void))
            .map(int)
            .transpose()?
            .unwrap_or(255)
            .clamp(0, 255);
        let (dx, dy, sx, sy, w, h) = (
            int(&args[0])? as i64,
            int(&args[1])? as i64,
            int(&args[3])? as i64,
            int(&args[4])? as i64,
            int(&args[5])? as i64,
            int(&args[6])? as i64,
        );
        let dest = dst.bitmap()?;
        let left = 0.max(-sx).max(dst.clip[0] as i64 - dx).max(-dx);
        let top = 0.max(-sy).max(dst.clip[1] as i64 - dy).max(-dy);
        let right = w
            .min(src.width as i64 - sx)
            .min(dst.clip[2] as i64 - dx)
            .min(dest.width as i64 - dx);
        let bottom = h
            .min(src.height as i64 - sy)
            .min(dst.clip[3] as i64 - dy)
            .min(dest.height as i64 - dy);
        if right <= left || bottom <= top {
            return Ok(Value::Void);
        }
        if op == "operateRect" {
            ensure!(matches!(face, 0 | 1 | 4), "invalid operateRect draw face");
            if !matches!(mode, 1 | 2 | 12) {
                return Err(unsupported(format!("Layer.operateRect blend mode {mode}")));
            }
            if opacity == 0 {
                return Ok(Value::Void);
            }
        } else {
            ensure!(matches!(face, 0..=4), "invalid copyRect draw face");
        }
        let n = ((right - left) * (bottom - top)) as usize;
        *budget = budget
            .checked_sub(n as u64)
            .ok_or_else(|| unsupported("Layer blit execution budget exceeded"))?;
        // Snapshot only the clipped source; self-copy preserves overlapping data.
        let mut pixels = Vec::with_capacity(n * 4);
        let mut provinces = Vec::new();
        if op == "copyRect" && face == 3 {
            provinces.reserve(n);
        }
        for y in top..bottom {
            for x in left..right {
                let i = ((sy + y) * src.width as i64 + sx + x) as usize;
                pixels.extend_from_slice(&src.rgba[i * 4..i * 4 + 4]);
                if op == "copyRect" && face == 3 {
                    provinces.push(source.province.as_ref().map_or(0, |p| {
                        p.pixels[(sy + y) as usize * p.width + (sx + x) as usize]
                    }));
                }
            }
        }
        let has_province = source.province.is_some();
        let used: usize = self
            .layers
            .iter()
            .filter(|(key, _)| **key != id)
            .map(|(_, l)| {
                l.image.as_ref().map_or(0, |i| i.rgba.len())
                    + l.province.as_ref().map_or(0, |p| p.pixels.len())
            })
            .sum();
        let dst = self.layers.get_mut(&id).unwrap();
        if op == "copyRect" && face == 3 && has_province {
            dst.allocate_province((256usize << 20).saturating_sub(used))?;
        }
        let hold = dst.hold_alpha;
        let image = dst.image.as_mut().unwrap();
        let mut sample = 0;
        for y in top..bottom {
            for x in left..right {
                let i = ((dy + y) * image.width as i64 + dx + x) as usize;
                let p = &mut image.rgba[i * 4..i * 4 + 4];
                let s: [u8; 4] = pixels[sample * 4..sample * 4 + 4].try_into().unwrap();
                if op == "copyRect" {
                    if face == 3 {
                        if let Some(province) = &mut dst.province {
                            province.pixels[i] = provinces[sample];
                        }
                    } else {
                        if face != 2 {
                            p[..3].copy_from_slice(&s[..3]);
                        }
                        if face != 1 || !hold {
                            p[3] = s[3];
                        }
                    }
                } else {
                    blend(p, s, face, mode, opacity, hold);
                }
                sample += 1;
            }
        }
        dst.image_modified = true;
        Ok(Value::Void)
    }
}

fn blend(d: &mut [u8], mut s: [u8; 4], face: i32, mode: i32, opacity: i32, hold: bool) {
    let opacity = if mode == 12 && face == 1 && !hold && opacity != 255 { opacity + (opacity >> 7) } else { opacity };
    let alpha = if mode == 1 {
        opacity
    } else if opacity == 255 {
        s[3] as i32
    } else {
        (s[3] as i32 * opacity) >> 8
    };
    // Kirikiri 1.2.0.3's bmAddAlphaOnAlpha branch leaves pixels unchanged.
    if mode == 12 && face == 0 { return; }
    if mode == 1 && opacity == 255 {
        d[..3].copy_from_slice(&s[..3]);
        if face != 1 {
            d[3] = 255;
        } else if !hold {
            d[3] = s[3];
        }
    } else if face == 0 {
        let weight = straight_alpha_weight(d[3], alpha as u8);
        for c in 0..3 {
            d[c] = (d[c] as i32 + (((s[c] as i32 - d[c] as i32) * weight) >> 8)) as u8;
        }
        d[3] = (255 - (255 - d[3] as i32) * (255 - alpha) / 255) as u8;
    } else if face == 4 || mode == 12 {
        if mode == 12 {
            if opacity != 255 {
                for c in &mut s {
                    *c = ((*c as i32 * opacity) >> 8) as u8;
                }
            }
        } else if mode == 2 {
            let a = alpha;
            for c in &mut s[..3] {
                *c = ((*c as i32 * a) >> 8) as u8;
            }
        }
        for c in 0..3 {
            let remaining = if mode == 12 && (opacity == 255 || (face == 1 && !hold)) { d[c] as i32 - ((d[c] as i32 * alpha) >> 8) }
                else { (d[c] as i32 * (255-alpha)) >> 8 };
            d[c] = (remaining + s[c] as i32).min(255) as u8;
        }
        if face == 4 || !hold {
            d[3] = (d[3] as i32 + alpha - ((d[3] as i32 * alpha) >> 8)).min(255) as u8;
        }
    } else {
        for c in 0..if hold { 3 } else { 4 } {
            d[c] = if alpha == 255 && !hold {
                s[c]
            } else {
                (d[c] as i32 + (((s[c] as i32 - d[c] as i32) * alpha) >> 8)) as u8
            };
        }
    }
}
