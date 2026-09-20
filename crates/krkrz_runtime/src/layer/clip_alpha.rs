//! PackinOne clipAlphaRect behavior, checked against the installed plugin.
use super::*;

impl Services {
    pub(super) fn layer_clip_alpha(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(args.len() >= 7, "Layer.clipAlphaRect: missing arguments");
        let source_id = object(&args[2])?.context("clip alpha source is null")?;
        let source = self
            .layers
            .get(&source_id)
            .context("clip alpha source is not a Layer")?;
        let src = source.bitmap()?;
        let dst = &self.layers[&id];
        let dest = dst.bitmap()?;
        let (dx, dy, sx, sy, width, height) = (
            int(&args[0])? as i64,
            int(&args[1])? as i64,
            int(&args[3])? as i64,
            int(&args[4])? as i64,
            int(&args[5])? as i64,
            int(&args[6])? as i64,
        );
        let left = 0.max(-sx).max(-dx).max(dst.clip[0] as i64 - dx);
        let top = 0.max(-sy).max(-dy).max(dst.clip[1] as i64 - dy);
        let right = width
            .min(src.width as i64 - sx)
            .min(dest.width as i64 - dx)
            .min(dst.clip[2] as i64 - dx);
        let bottom = height
            .min(src.height as i64 - sy)
            .min(dest.height as i64 - dy)
            .min(dst.clip[3] as i64 - dy);
        if right <= left || bottom <= top {
            self.layers.get_mut(&id).unwrap().image_modified = true;
            return Ok(Value::Void);
        }
        let count = ((right - left) * (bottom - top)) as usize;
        *budget = budget
            .checked_sub(count as u64)
            .ok_or_else(|| unsupported("Layer clipAlphaRect execution budget exceeded"))?;
        // The original plugin walks self copies forward. Unlike copyRect, its
        // overlapping source pixels can include alpha values already modified.
        let alphas = if source_id != id {
            let mut values = Vec::with_capacity(count);
            for y in top..bottom {
                for x in left..right {
                    values.push(src.rgba[((sy + y) * src.width as i64 + sx + x) as usize * 4 + 3]);
                }
            }
            Some(values)
        } else {
            None
        };
        let available = image_available(&self.layers, id);
        let dst = self.layers.get_mut(&id).unwrap();
        dst.prepare_image_write(available)?;
        let image = Arc::make_mut(dst.image.as_mut().unwrap());
        let mut sample = 0;
        for y in top..bottom {
            for x in left..right {
                let source_alpha = alphas.as_ref().map_or_else(
                    || image.rgba[((sy + y) * image.width as i64 + sx + x) as usize * 4 + 3],
                    |values| values[sample],
                );
                let offset = ((dy + y) * image.width as i64 + dx + x) as usize * 4 + 3;
                // Original DLL exhaustive check: all 65,536 alpha pairs.
                let product = image.rgba[offset] as u16 * source_alpha as u16;
                image.rgba[offset] = ((product + (product >> 7)) >> 8) as u8;
                sample += 1;
            }
        }
        dst.image_modified = true;
        Ok(Value::Void)
    }
}
