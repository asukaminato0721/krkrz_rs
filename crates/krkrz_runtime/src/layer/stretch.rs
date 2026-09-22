//! Image resampling uses fast_image_resize; script geometry remains in the adapter.
use super::*;
use fast_image_resize::{
    FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
    images::{Image as ResizeImage, ImageRef},
};

pub(super) fn resize(
    source: &Image,
    crop: [f64; 4],
    width: u32,
    height: u32,
    filter: FilterType,
    budget: &mut u64,
) -> Result<Vec<u8>> {
    // Bound the destination, the separable intermediate and coefficient tables.
    let source_pixels = crop[2].ceil() as u64 * crop[3].ceil() as u64;
    let pixels = width as u64 * height as u64;
    let intermediate = source.width as u64 * height as u64;
    let tables = (source.width as u64 + source.height as u64 + width as u64 + height as u64) * 128;
    ensure!(
        (pixels + intermediate) * 4 + tables <= MAX_LAYER_IMAGE_BYTES as u64,
        "resize temporary memory limit exceeded"
    );
    *budget = budget
        .checked_sub((source_pixels + intermediate + pixels) * 8)
        .ok_or_else(|| unsupported("resize execution budget exceeded"))?;
    let input = ImageRef::new(source.width, source.height, &source.rgba, PixelType::U8x4)?;
    let mut output = ResizeImage::new(width, height, PixelType::U8x4);
    // copy operations sample all four stored channels independently. Premultiplying
    // here would erase RGB that scripts can read from fully transparent pixels.
    let options = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(filter))
        .crop(crop[0], crop[1], crop[2], crop[3])
        .use_alpha(false);
    Resizer::new().resize(&input, &mut output, &options)?;
    Ok(output.into_vec())
}

impl Services {
    pub(super) fn layer_stretch(
        &mut self,
        id: usize,
        op: &str,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(args.len() >= 9, "Layer.{op}: missing arguments");
        let (dx, dy, dw, dh) = (
            int(&args[0])? as i64,
            int(&args[1])? as i64,
            int(&args[2])? as i64,
            int(&args[3])? as i64,
        );
        let (sx, sy, sw, sh) = (
            int(&args[5])? as i64,
            int(&args[6])? as i64,
            int(&args[7])? as i64,
            int(&args[8])? as i64,
        );
        let source = object(&args[4])?.context("stretchCopy source is null")?;
        let source_layer = self
            .layers
            .get(&source)
            .context("stretch source is not a Layer")?;
        let src = source_layer.bitmap()?;
        let optional_int = |index, default| {
            args.get(index)
                .filter(|v| !matches!(v, Value::Void))
                .map(int)
                .transpose()
                .map(|v| v.unwrap_or(default))
        };
        let operation = if op == "operateStretch" {
            let mode = match optional_int(9, 128)? {
                128 => match source_layer.kind {
                    2..=5 | 8..=28 => source_layer.kind,
                    _ => 1,
                },
                mode => mode,
            };
            Some((mode, optional_int(10, 255)?.clamp(0, 255)))
        } else {
            None
        };
        let dst = &self.layers[&id];
        let image = dst.bitmap()?;
        let clip = [
            dst.clip[0].max(0) as i64,
            dst.clip[1].max(0) as i64,
            (dst.clip[2] as i64).min(image.width as i64),
            (dst.clip[3] as i64).min(image.height as i64),
        ];
        if dw == 0
            || dh == 0
            || sw == 0
            || sh == 0
            || dx.min(dx + dw) >= clip[2]
            || dx.max(dx + dw) <= clip[0]
            || dy.min(dy + dh) >= clip[3]
            || dy.max(dy + dh) <= clip[1]
        {
            return Ok(Value::Void);
        }
        let face = dst.draw_face();
        ensure!(matches!(face, 0 | 1 | 4), "Layer.{op}: invalid draw face");
        if let Some((mode, opacity)) = operation {
            if !matches!(mode, 1 | 2 | 12) {
                return Err(unsupported(format!(
                    "Layer.operateStretch blend mode {mode}"
                )));
            }
            if opacity == 0 {
                return Ok(Value::Void);
            }
        }
        let mode = optional_int(if operation.is_some() { 11 } else { 9 }, 0)?;
        let filter = mode & 15;
        // Unscaled blits keep the native clipping and overlap semantics.
        if dw == sw
            && dh == sh
            && dx >= clip[0]
            && dy >= clip[1]
            && dx + dw <= clip[2]
            && dy + dh <= clip[3]
        {
            let mut blit_args = vec![
                args[0].clone(),
                args[1].clone(),
                args[4].clone(),
                args[5].clone(),
                args[6].clone(),
                args[7].clone(),
                args[8].clone(),
            ];
            if let Some((mode, opacity)) = operation {
                blit_args.extend([Value::Integer(mode.into()), Value::Integer(opacity.into())]);
            }
            return self.layer_blit(
                id,
                if operation.is_some() {
                    "operateRect"
                } else {
                    "copyRect"
                },
                &blit_args,
                budget,
            );
        }
        let hold = face == 1 && dst.hold_alpha;
        let resample = matches!(filter, 2 | 3)
            && !hold
            && dw > 0
            && dh > 0
            && sw > 0
            && sh > 0
            && dx >= clip[0]
            && dy >= clip[1]
            && dx + dw <= clip[2]
            && dy + dh <= clip[3];
        if !resample {
            let mut affine = vec![
                args[4].clone(),
                args[5].clone(),
                args[6].clone(),
                args[7].clone(),
                args[8].clone(),
                Value::Integer(0),
            ];
            affine.extend(
                [
                    dx as f64 - 0.5,
                    dy as f64 - 0.5,
                    (dx + dw) as f64 - 0.5,
                    dy as f64 - 0.5,
                    dx as f64 - 0.5,
                    (dy + dh) as f64 - 0.5,
                ]
                .map(Value::Real),
            );
            affine.push(Value::Integer(mode as i64));
            return self.layer_affine(id, &affine, operation, budget);
        }
        ensure!(
            sw > 0
                && sh > 0
                && sx >= 0
                && sy >= 0
                && sx + sw <= src.width as i64
                && sy + sh <= src.height as i64,
            "stretchCopy source rectangle outside image"
        );
        let (left, top, right, bottom) = (dx, dy, dx + dw, dy + dh);
        let output = resize(
            src,
            [sx as f64, sy as f64, sw as f64, sh as f64],
            dw as u32,
            dh as u32,
            if filter == 3 {
                FilterType::CatmullRom
            } else {
                FilterType::Bilinear
            },
            budget,
        )?;
        let available = image_available(&self.layers, id);
        let dst = self.layers.get_mut(&id).unwrap();
        let hold_alpha = dst.hold_alpha;
        dst.prepare_image_write(available)?;
        let image = Arc::make_mut(dst.image.as_mut().unwrap());
        if let Some((mode, opacity)) = operation {
            for (y, row) in output.chunks_exact(dw as usize * 4).enumerate() {
                let start = ((top as usize + y) * image.width as usize + left as usize) * 4;
                blit::blend_row(
                    &mut image.rgba[start..start + row.len()],
                    row,
                    left as usize,
                    face,
                    mode,
                    opacity,
                    hold_alpha,
                );
            }
            dst.image_modified = true;
            return Ok(Value::Void);
        }
        let mut pixels = output.as_chunks::<4>().0.iter();
        for y in top..bottom {
            for x in left..right {
                let i = (y as usize * image.width as usize + x as usize) * 4;
                let count = if hold { 3 } else { 4 };
                image.rgba[i..i + count].copy_from_slice(&pixels.next().unwrap()[..count]);
            }
        }
        dst.image_modified = true;
        Ok(Value::Void)
    }
}
