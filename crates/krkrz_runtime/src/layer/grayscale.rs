//! Kirikiri TVPDoGrayScale: integer luminance, clipped RGB, unchanged alpha.
use super::*;

impl Services {
    pub(super) fn layer_grayscale(&mut self, id: usize, budget: &mut u64) -> Result<Value> {
        let layer = &self.layers[&id];
        let image = layer.bitmap()?;
        let left = layer.clip[0].max(0) as usize;
        let top = layer.clip[1].max(0) as usize;
        let right = layer.clip[2].max(0).min(image.width as i32) as usize;
        let bottom = layer.clip[3].max(0).min(image.height as i32) as usize;
        let count = right.saturating_sub(left) * bottom.saturating_sub(top);
        *budget = budget
            .checked_sub(count as u64)
            .ok_or_else(|| unsupported("Layer.doGrayScale execution budget exceeded"))?;
        let available = image_available(&self.layers, id);
        let layer = self.layers.get_mut(&id).unwrap();
        if right > left && bottom > top {
            layer.prepare_image_write(available)?;
            let image = Arc::make_mut(layer.image.as_mut().unwrap());
            for y in top..bottom {
                let start = (y * image.width as usize + left) * 4;
                for pixel in image.rgba[start..start + (right - left) * 4]
                    .as_chunks_mut::<4>()
                    .0
                {
                    let gray = ((pixel[0] as u32 * 54
                        + pixel[1] as u32 * 183
                        + pixel[2] as u32 * 19)
                        >> 8) as u8;
                    pixel[..3].fill(gray);
                }
            }
        }
        // DrawFace and holdAlpha do not affect this whole-main-image operation.
        layer.image_modified = true;
        Ok(Value::Void)
    }
}
