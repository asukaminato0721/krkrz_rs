//! Fixed-point rasterization from Kirikiri's LayerBitmapIntf.cpp.
//! See THIRD_PARTY_NOTICES.md. Arithmetic is widened before multiplying so
//! malformed coordinates cannot overflow the native 16.16 intermediates.
use super::*;

const ONE: i64 = 65536;
type Point = [i64; 2];
fn mul(a: i64, b: i64) -> i64 {
    a * b / ONE
}
fn interpolate(a: Point, b: Point, t: i64) -> Point {
    [a[0] + mul(b[0] - a[0], t), a[1] + mul(b[1] - a[1], t)]
}
fn edge(points: &[Point; 4], code: usize, y: i64) -> Option<i64> {
    let (a, b) = (points[code], points[(code + 1) & 3]);
    if a[1] == y && b[1] == y {
        Some(if b[0] > a[0] { 0 } else { ONE })
    } else if (a[1] <= y && b[1] > y) || (a[1] > y && b[1] <= y) {
        Some((y - a[1]) * ONE / (b[1] - a[1]))
    } else {
        None
    }
}
fn intersections(points: &[Point; 4], source: &[Point; 4], y: i64) -> Option<[(i64, Point); 2]> {
    let horizontal = (0..4).find(|&i| points[i][1] == y && points[(i + 1) & 3][1] == y);
    let (a, ta, b, tb) = if let Some(i) = horizontal {
        let t = edge(points, i, y)?;
        (i, t, i, ONE - t)
    } else {
        let mut edges = (0..4).filter_map(|i| edge(points, i, y).map(|t| (i, t)));
        let (a, ta) = edges.next()?;
        let (b, tb) = edges.next()?;
        (a, ta, b, tb)
    };
    let at = |i, t| {
        (
            interpolate(points[i], points[(i + 1) & 3], t)[0],
            interpolate(source[i], source[(i + 1) & 3], t),
        )
    };
    let mut pair = [at(a, ta), at(b, tb)];
    if pair[0].0 > pair[1].0 {
        pair.swap(0, 1);
    }
    Some(pair)
}
fn pixel(image: &Image, x: i64, y: i64) -> [u8; 4] {
    let start = (y as usize * image.width as usize + x as usize) * 4;
    image.rgba[start..start + 4].try_into().unwrap()
}
fn blend(a: [u8; 4], b: [u8; 4], weight: i64) -> [u8; 4] {
    std::array::from_fn(|i| (a[i] as i64 + (((b[i] as i64 - a[i] as i64) * weight) >> 8)) as u8)
}
fn linear(image: &Image, pos: Point, bounds: [i64; 4]) -> [u8; 4] {
    let [x, y] = pos.map(|v| v >> 16);
    let read = |x: i64, y: i64| {
        pixel(
            image,
            x.clamp(bounds[0], bounds[2] - 1),
            y.clamp(bounds[1], bounds[3] - 1),
        )
    };
    let mut wx = (pos[0] & 65535) >> 8;
    let mut wy = (pos[1] & 65535) >> 8;
    // The native MMX interior loop adjusts Y only. The scalar edge loop
    // adjusts both. Retain this visible one-level rounding difference.
    if x < bounds[0] || x + 1 >= bounds[2] || y < bounds[1] || y + 1 >= bounds[3] {
        wx += wx >> 7;
    }
    wy += wy >> 7;
    blend(
        blend(read(x, y), read(x + 1, y), wx),
        blend(read(x, y + 1), read(x + 1, y + 1), wx),
        wy,
    )
}
impl Services {
    pub(super) fn layer_affine_copy(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(args.len() >= 12, "Layer.affineCopy: missing arguments");
        let src_id = object(&args[0])?.context("affineCopy source is null")?;
        let src = self
            .layers
            .get(&src_id)
            .context("affineCopy source is not a Layer")?
            .bitmap()?;
        let dst = &self.layers[&id];
        let image = dst.bitmap()?;
        let face = dst.draw_face();
        ensure!(matches!(face, 0 | 1 | 4), "affineCopy: invalid draw face");
        let hold = face == 1 && dst.hold_alpha;
        let (sx, sy, sw, sh) = (
            int(&args[1])? as i64,
            int(&args[2])? as i64,
            int(&args[3])? as i64,
            int(&args[4])? as i64,
        );
        if sw <= 0 || sh <= 0 {
            return Ok(Value::Void);
        }
        ensure!(
            sx >= 0 && sy >= 0 && sx + sw <= src.width as i64 && sy + sh <= src.height as i64,
            "affineCopy source rectangle outside image"
        );
        let coordinates: Vec<f64> = args[6..12].iter().map(Value::real).collect::<Result<_>>()?;
        ensure!(
            coordinates.iter().all(|v| v.is_finite()),
            "affineCopy coordinates must be finite"
        );
        let mut points = [[0.0; 2]; 3];
        if args[5].truth()? {
            let [a, b, c, d, tx, ty]: [f64; 6] = coordinates.try_into().unwrap();
            for (point, [x, y]) in points.iter_mut().zip([
                [-0.5, -0.5],
                [sw as f64 - 0.5, -0.5],
                [-0.5, sh as f64 - 0.5],
            ]) {
                *point = [a * x + c * y + tx, b * x + d * y + ty];
            }
        } else {
            for (point, coordinates) in points.iter_mut().zip(coordinates.as_chunks::<2>().0) {
                *point = *coordinates;
            }
        }
        ensure!(
            points
                .iter()
                .flatten()
                .all(|v| v.is_finite() && v.abs() < 16384.0),
            "affineCopy transformed coordinates exceed fixed-point range"
        );
        let [a, b, d] = points.map(|p| p.map(|v| (v * ONE as f64) as i64));
        let points = [a, b, [b[0] - a[0] + d[0], b[1] - a[1] + d[1]], d];
        let source = [[sx, sy], [sx + sw, sy], [sx + sw, sy + sh], [sx, sy + sh]]
            .map(|p| p.map(|v| v * ONE - 32768));
        let mode = args.get(12).map(int).transpose()?.unwrap_or(0);
        let clear = args.get(13).map(Value::truth).transpose()?.unwrap_or(false);
        let filtered = mode & 15 >= 1 && !hold;
        let bounds = if mode & 16 != 0 {
            [0, 0, src.width as i64, src.height as i64]
        } else {
            [sx, sy, sx + sw, sy + sh]
        };
        let clip = [
            dst.clip[0].max(0) as i64,
            dst.clip[1].max(0) as i64,
            (dst.clip[2] as i64).min(image.width as i64),
            (dst.clip[3] as i64).min(image.height as i64),
        ];
        if clip[0] >= clip[2] || clip[1] >= clip[3] {
            return Ok(Value::Void);
        }
        let cost = (clip[2] - clip[0]) as u64 * (clip[3] - clip[1]) as u64;
        *budget = budget
            .checked_sub(cost)
            .ok_or_else(|| unsupported("affineCopy execution budget exceeded"))?;
        let mut output = image.clone();
        let write = |output: &mut Image, x: i64, y: i64, p: [u8; 4]| {
            let i = (y as usize * output.width as usize + x as usize) * 4;
            output.rgba[i..i + if hold { 3 } else { 4 }]
                .copy_from_slice(&p[..if hold { 3 } else { 4 }]);
        };
        let v1 = [
            (b[0] - a[0]) as f64 / ONE as f64,
            (b[1] - a[1]) as f64 / ONE as f64,
        ];
        let v2 = [
            (d[0] - a[0]) as f64 / ONE as f64,
            (d[1] - a[1]) as f64 / ONE as f64,
        ];
        let determinant = v1[0] * v2[1] - v1[1] * v2[0];
        let steps = if b[1] == a[1] {
            [(sw * ONE) as f64 / v1[0], 0.0]
        } else if d[1] == a[1] {
            [0.0, (sh * ONE) as f64 / v2[0]]
        } else {
            let len = v1[0] / v1[1] - v2[0] / v2[1];
            [
                (sw * ONE) as f64 / v1[1] / len,
                -(sh * ONE) as f64 / v2[1] / len,
            ]
        };
        ensure!(
            determinant == 0.0
                || steps
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= i32::MAX as f64),
            "affineCopy sampling step exceeds fixed-point range"
        );
        let steps = steps.map(|v| v as i64);
        let top = points.iter().map(|p| p[1]).min().unwrap();
        let bottom = points.iter().map(|p| p[1]).max().unwrap();
        let mut first = None;
        let mut last = 0;
        if determinant != 0.0 {
            for y in clip[1].max((top + 32768) / ONE)..clip[3].min((bottom + 32768) / ONE + 1) {
                if y * ONE < top || y * ONE >= bottom {
                    continue;
                }
                let Some([(left, mut pos), (right, _)]) = intersections(&points, &source, y * ONE)
                else {
                    continue;
                };
                let mut l = -(-left).div_euclid(ONE);
                let r = (-(-right).div_euclid(ONE)).min(clip[2]);
                let adjust = l * ONE - left;
                for i in 0..2 {
                    pos[i] += mul(adjust, steps[i]);
                }
                if l < clip[0] {
                    for i in 0..2 {
                        pos[i] += (clip[0] - l) * steps[i];
                    }
                    l = clip[0];
                }
                if l >= r {
                    continue;
                }
                first.get_or_insert(y);
                last = y;
                if clear {
                    for x in clip[0]..(l + 1).min(clip[2]) {
                        write(&mut output, x, y, dst.neutral);
                    }
                    for x in (r - 1).max(clip[0])..clip[2] {
                        write(&mut output, x, y, dst.neutral);
                    }
                }
                for x in l..r {
                    let nearest = pos.map(|v| (v + 32768) >> 16);
                    if nearest[0] >= sx
                        && nearest[0] < sx + sw
                        && nearest[1] >= sy
                        && nearest[1] < sy + sh
                    {
                        let p = if filtered {
                            linear(src, pos, bounds)
                        } else {
                            pixel(src, nearest[0], nearest[1])
                        };
                        write(&mut output, x, y, p);
                    }
                    for i in 0..2 {
                        pos[i] += steps[i];
                    }
                }
            }
        }
        if clear {
            let first = first.unwrap_or(clip[3]);
            if first == clip[3] {
                last = clip[3] - 1;
            }
            for y in (clip[1]..first).chain(last + 1..clip[3]) {
                for x in clip[0]..clip[2] {
                    write(&mut output, x, y, dst.neutral);
                }
            }
        }
        let dst = self.layers.get_mut(&id).unwrap();
        dst.image = Some(output);
        dst.image_modified = true;
        Ok(Value::Void)
    }
}
