//! Scalar ripple displacement from SamplePlugin/extrans/ripple.cpp.
use super::*;

pub(in crate::layer) struct Ripple {
    center: [i32; 2],
    width: usize,
    height: usize,
    rwidth: usize,
    speed: f32,
    max_drift: i32,
    map: Vec<u16>,
    wave: Vec<i32>,
    directions: [[i32; 2]; 32],
}
impl Ripple {
    pub(super) fn new(
        vm: &mut Vm,
        host: &mut Services,
        options: &Value,
        size: (i32, i32),
        budget: &mut u64,
    ) -> Result<Self> {
        let mut option = |name: &str, default: Value| -> Result<Value> {
            let value =
                vm.get_property(options, &Value::string(name), true, false, host, budget)?;
            Ok(if matches!(value, Value::Void) {
                default
            } else {
                value
            })
        };
        let cx = option("centerx", Value::Integer((size.0 / 2) as i64))?.integer()? as i32;
        let cy = option("centery", Value::Integer((size.1 / 2) as i64))?.integer()? as i32;
        ensure!(
            cx >= 0 && cy >= 0 && cx < size.0 && cy < size.1,
            "centerx and centery cannot be out of the image"
        );
        let rw = option("rwidth", Value::Integer(128))?.integer()?;
        ensure!(
            matches!(rw, 16 | 32 | 64 | 128),
            "rwidth must be 16, 32, 64 or 128"
        );
        let roundness = option("roundness", Value::Real(1.0))?.real()? as f32;
        ensure!(
            roundness.is_finite() && roundness > 0.0,
            "roundness must be positive and finite"
        );
        let speed = option("speed", Value::Real(6.0))?.real()? as f32;
        ensure!(speed.is_finite(), "ripple speed must be finite");
        let max_drift = option("maxdrift", Value::Integer(24))?.integer()?;
        ensure!(
            (0..128).contains(&max_drift) && max_drift < size.0 as i64 && max_drift < size.1 as i64,
            "maxdrift must be 0..127 and smaller than both image dimensions"
        );
        let (width, height) = (size.0 as usize, size.1 as usize);
        let count = width
            .checked_mul(height)
            .context("ripple dimensions overflow")?;
        ensure!(
            count <= 16_777_216,
            "ripple image exceeds supported dimensions"
        );
        *budget = budget
            .checked_sub(count as u64)
            .ok_or_else(|| unsupported("ripple table execution budget exceeded"))?;
        let mut map = Vec::with_capacity(count);
        for y in 0..height {
            let yy = (if y < cy as usize {
                cy as usize - y - 1
            } else {
                y - cy as usize
            }) as f32;
            let yy = (yy + 0.5) * roundness;
            for x in 0..width {
                let xx = (if x < cx as usize {
                    cx as usize - x - 1
                } else {
                    x - cx as usize
                }) as f32
                    + 0.5;
                let dir = ((xx / yy).atan() as f64 * (32.0 / std::f64::consts::FRAC_PI_2)) as usize;
                let dist = (xx * xx + yy * yy).sqrt() as usize & (rw as usize - 1);
                map.push((dist * 32 + dir.min(31)) as u16);
            }
        }
        let wave = (0..rw)
            .map(|w| {
                let rad = (w as f32 * (1.0 / rw as f32)) as f64 * (-2.0 * std::f64::consts::PI);
                let s = ((rad.sin() + (rad * 2.0 - 2.0).sin() * 0.2) / 1.19) as f32;
                ((s * s).clamp(-1.0, 1.0) * 2048.0).round() as i32
            })
            .collect();
        let directions = std::array::from_fn(|w| {
            let a = (std::f64::consts::FRAC_PI_2
                - (w as f64 + 0.5) * (std::f64::consts::FRAC_PI_2 / 32.0))
                as f32;
            [
                (a.cos() * 2048.0).round() as i32,
                (a.sin() * 2048.0).round() as i32,
            ]
        });
        Ok(Self {
            center: [cx, cy],
            width,
            height,
            rwidth: rw as usize,
            speed,
            max_drift: max_drift as i32,
            map,
            wave,
            directions,
        })
    }
    pub(super) fn blend(
        &self,
        dst: &mut Image,
        src: &Image,
        origin: [i64; 2],
        elapsed: u64,
        duration: u64,
    ) {
        if elapsed >= duration {
            dst.rgba.copy_from_slice(&src.rgba);
            return;
        }
        let ratio = (elapsed as u128 * 255 / duration as u128) as i32;
        let phase = (self.speed as f64 / (2.0 * std::f64::consts::PI * 1000.0)
            * elapsed as f64
            * self.rwidth as f64) as i64
            % self.rwidth as i64;
        let phase = self.rwidth - phase.max(0) as usize - 1;
        let drift = (((std::f64::consts::PI * elapsed as f64 / duration as f64).sin() as f32
            * self.max_drift as f32
            * 4.0) as i32)
            .clamp(0, (self.max_drift * 4 - 1).max(0));
        let mut displacements = vec![[0i32; 2]; self.rwidth * 32];
        for w in 0..self.rwidth {
            let fd = (self.wave[(w + phase) & (self.rwidth - 1)] * (drift * 256)) >> 10;
            for dir in 0..32 {
                let dx = ((self.directions[dir][0] * fd) >> 11) >> 11;
                let dy = ((self.directions[dir][1] * fd) >> 11) >> 11;
                // The original packs signed byte offsets into one 16-bit word.
                let packed = ((dx << 8) + dy) as u16;
                displacements[w * 32 + dir] =
                    [(packed >> 8) as u8 as i8 as i32, packed as u8 as i8 as i32];
            }
        }
        let original = dst.rgba.clone();
        let reflect = |n: i64, len: usize| -> usize {
            let n = if n < 0 { -n } else { n };
            if n >= len as i64 {
                (2 * len as i64 - 1 - n).clamp(0, len as i64 - 1) as usize
            } else {
                n as usize
            }
        };
        for y in 0..dst.height as usize {
            for x in 0..dst.width as usize {
                let (gx, gy) = (origin[0] + x as i64, origin[1] + y as i64);
                if gx < 0 || gy < 0 || gx >= self.width as i64 || gy >= self.height as i64 {
                    continue;
                }
                let [dx, dy] =
                    displacements[self.map[gy as usize * self.width + gx as usize] as usize];
                let sx = reflect(
                    gx + if gx < self.center[0] as i64 {
                        dx as i64
                    } else {
                        -(dx as i64)
                    },
                    self.width,
                ) as i64
                    - origin[0];
                let sy = reflect(
                    gy + if gy < self.center[1] as i64 {
                        dy as i64
                    } else {
                        -(dy as i64)
                    },
                    self.height,
                ) as i64
                    - origin[1];
                if sx < 0 || sy < 0 || sx >= dst.width as i64 || sy >= dst.height as i64 {
                    continue;
                }
                let from = (sy as usize * dst.width as usize + sx as usize) * 4;
                let to = (y * dst.width as usize + x) * 4;
                for c in 0..3 {
                    let a = original[from + c] as i32;
                    let b = src.rgba[from + c] as i32;
                    dst.rgba[to + c] = (a + ((b - a) * ratio >> 8)) as u8;
                }
                dst.rgba[to + 3] = 255;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn displacement_matches_original_scalar_reference() {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        let mut session =
            crate::Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
        let options = session
            .evaluate("%[centerx:9,centery:7,rwidth:16,roundness:1.5,speed:6,maxdrift:8]")
            .unwrap();
        let ripple = Ripple::new(
            &mut session.vm,
            &mut session.services,
            &options,
            (32, 32),
            &mut session.budget,
        )
        .unwrap();
        let mut a = Image {
            width: 32,
            height: 32,
            rgba: vec![],
        };
        let mut b = a.clone();
        for y in 0..32 {
            for x in 0..32 {
                a.rgba.extend([
                    (x * 7 + y * 3) as u8,
                    (x * 2 + y * 5) as u8,
                    (x + y * 9) as u8,
                    255,
                ]);
                b.rgba.extend([
                    (255 - x * 3 - y * 2) as u8,
                    (x * 8 + y) as u8,
                    (255 - x - y * 4) as u8,
                    255,
                ]);
            }
        }
        let expected = include_bytes!("../../../tests/fixtures/ripple-scalar.rgba");
        for (frame, time) in [0, 25, 50, 75].into_iter().enumerate() {
            let mut actual = a.clone();
            ripple.blend(&mut actual, &b, [0, 0], time, 100);
            assert_eq!(
                actual.rgba,
                &expected[frame * 4096..(frame + 1) * 4096],
                "elapsed {time}"
            );
        }
    }
}
