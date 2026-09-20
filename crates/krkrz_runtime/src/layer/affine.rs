//! Affine rasterization through tiny-skia; Layer storage remains straight RGBA.
use super::*;
use tiny_skia::{
    BlendMode, FillRule, FilterQuality, Paint, PathBuilder, Pattern, Pixmap, Rect, SpreadMode,
    Transform,
};

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
            "affineCopy transformed coordinates exceed coordinate range"
        );
        let [a, b, d] = points;
        let mode = args.get(12).map(int).transpose()?.unwrap_or(0);
        let clear = args.get(13).map(Value::truth).transpose()?.unwrap_or(false);
        let clip = [
            dst.clip[0].max(0),
            dst.clip[1].max(0),
            dst.clip[2].min(image.width as i32),
            dst.clip[3].min(image.height as i32),
        ];
        if clip[0] >= clip[2] || clip[1] >= clip[3] {
            return Ok(Value::Void);
        }
        let (width, height) = ((clip[2] - clip[0]) as u32, (clip[3] - clip[1]) as u32);
        let bounds = if mode & 16 != 0 {
            [0, 0, src.width as i64, src.height as i64]
        } else {
            [sx, sy, sw, sh]
        };
        let pixels = width as u64 * height as u64;
        let source_pixels = bounds[2] as u64 * bounds[3] as u64;
        ensure!(
            (pixels * 2 + source_pixels) * 4 <= MAX_LAYER_IMAGE_BYTES as u64,
            "affineCopy temporary memory limit exceeded"
        );
        *budget = budget
            .checked_sub(pixels * 4 + source_pixels * 2)
            .ok_or_else(|| unsupported("affineCopy execution budget exceeded"))?;
        ensure!(
            image.rgba.len() + dst.province.as_ref().map_or(0, |p| p.pixels.len())
                <= image_available(&self.layers, id),
            "session layer image memory limit exceeded"
        );
        let xx = (b[0] - a[0]) / sw as f64;
        let yx = (b[1] - a[1]) / sw as f64;
        let xy = (d[0] - a[0]) / sh as f64;
        let yy = (d[1] - a[1]) / sh as f64;
        let determinant = xx * yy - xy * yx;
        ensure!(
            determinant == 0.0
                || [xx, xy, yx, yy]
                    .iter()
                    .all(|v| (v / determinant).abs() <= 32768.0),
            "affineCopy sampling step exceeds coordinate range"
        );
        let transform = Transform::from_row(
            xx as f32,
            yx as f32,
            xy as f32,
            yy as f32,
            (a[0] + 0.5
                - (sx - bounds[0]) as f64 * xx
                - (sy - bounds[1]) as f64 * xy
                - clip[0] as f64) as f32,
            (a[1] + 0.5
                - (sx - bounds[0]) as f64 * yx
                - (sy - bounds[1]) as f64 * yy
                - clip[1] as f64) as f32,
        );
        let path = PathBuilder::from_rect(
            Rect::from_xywh(
                (sx - bounds[0]) as f32,
                (sy - bounds[1]) as f32,
                sw as f32,
                sh as f32,
            )
            .context("invalid affineCopy rectangle")?,
        );
        let quality = if mode & 15 >= 1 && !hold {
            FilterQuality::Bilinear
        } else {
            FilterQuality::Nearest
        };
        let mut input = Pixmap::new(bounds[2] as u32, bounds[3] as u32)
            .context("invalid affineCopy source size")?;
        let mut colors =
            Pixmap::new(width, height).context("invalid affineCopy destination size")?;
        let mut alphas = colors.clone();
        // Two opaque planes avoid a premultiply/unpremultiply round trip. copyRect
        // semantics preserve hidden RGB and additive-alpha channels independently.
        for (alpha, target) in [(false, &mut colors), (true, &mut alphas)] {
            for (i, pixel) in input
                .data_mut()
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .enumerate()
            {
                let x = bounds[0] as usize + i % bounds[2] as usize;
                let y = bounds[1] as usize + i / bounds[2] as usize;
                let source = &src.rgba[(y * src.width as usize + x) * 4..][..4];
                let value = if alpha {
                    [source[3], source[3], source[3], 255]
                } else {
                    [source[0], source[1], source[2], 255]
                };
                pixel.copy_from_slice(&value);
            }
            if determinant != 0.0 {
                let paint = Paint {
                    shader: Pattern::new(
                        input.as_ref(),
                        SpreadMode::Pad,
                        quality,
                        1.0,
                        Transform::identity(),
                    ),
                    blend_mode: BlendMode::Source,
                    anti_alias: false,
                    ..Paint::default()
                };
                target.fill_path(&path, &paint, FillRule::Winding, transform, None);
            }
        }
        let mut output = image.clone();
        for y in 0..height as usize {
            for x in 0..width as usize {
                let i = (y * width as usize + x) * 4;
                let target =
                    ((y + clip[1] as usize) * output.width as usize + x + clip[0] as usize) * 4;
                let pixel = if colors.data()[i + 3] != 0 {
                    [
                        colors.data()[i],
                        colors.data()[i + 1],
                        colors.data()[i + 2],
                        alphas.data()[i],
                    ]
                } else if clear {
                    dst.neutral
                } else {
                    continue;
                };
                let count = if hold { 3 } else { 4 };
                output.rgba[target..target + count].copy_from_slice(&pixel[..count]);
            }
        }
        let dst = self.layers.get_mut(&id).unwrap();
        dst.image = Some(Arc::new(output));
        dst.image_modified = true;
        Ok(Value::Void)
    }
}
