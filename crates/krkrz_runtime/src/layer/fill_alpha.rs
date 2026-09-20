//! PackinOne fillAlpha sets clipped main-image alpha to 255; arguments are ignored.
use super::*;

impl Services {
    pub(super) fn layer_fill_alpha(&mut self, id: usize, budget: &mut u64) -> Result<Value> {
        let layer = &self.layers[&id];
        let image = layer.bitmap()?;
        let left = layer.clip[0].max(0) as usize;
        let top = layer.clip[1].max(0) as usize;
        let right = layer.clip[2].max(0).min(image.width as i32) as usize;
        let bottom = layer.clip[3].max(0).min(image.height as i32) as usize;
        let count = right.saturating_sub(left) * bottom.saturating_sub(top);
        *budget = budget
            .checked_sub(count as u64)
            .ok_or_else(|| unsupported("Layer.fillAlpha execution budget exceeded"))?;
        let layer = self.layers.get_mut(&id).unwrap();
        if right > left && bottom > top {
            let image = layer.image.as_mut().unwrap();
            for y in top..bottom {
                let start = (y * image.width as usize + left) * 4;
                for pixel in image.rgba[start..start + (right - left) * 4]
                    .as_chunks_mut::<4>()
                    .0
                {
                    pixel[3] = 255;
                }
            }
        }
        layer.image_modified = true;
        Ok(Value::Void)
    }
}
