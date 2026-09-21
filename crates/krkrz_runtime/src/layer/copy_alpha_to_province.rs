//! PackinOne alpha-to-province conversion, checked against the original DLL.
use super::*;

impl Services {
    pub(super) fn layer_copy_alpha_to_province(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let optional = |index: usize, default: i32| -> Result<i32> {
            args.get(index)
                .filter(|value| !matches!(value, Value::Void))
                .map_or(Ok(default), int)
        };
        let threshold = optional(0, -1)?;
        let above = optional(1, 1)?;
        let below = optional(2, 0)?;
        let layer = &self.layers[&id];
        let image = layer.image.as_deref().context("src must be Layer.")?;
        let left = layer.clip[0].max(0) as usize;
        let top = layer.clip[1].max(0) as usize;
        let right = layer.clip[2].max(0).min(image.width as i32) as usize;
        let bottom = layer.clip[3].max(0).min(image.height as i32) as usize;
        let count = right.saturating_sub(left) * bottom.saturating_sub(top);
        ensure!(count > 0, "src must be Layer.");
        *budget = budget
            .checked_sub(count as u64)
            .ok_or_else(|| unsupported("Layer.copyAlphaToProvince execution budget exceeded"))?;
        let available = image_available(&self.layers, id);
        let layer = self.layers.get_mut(&id).unwrap();
        layer.allocate_province(available)?;
        // Only the province plane changes. Do not detach a shared main image.
        let image = layer.image.as_ref().unwrap();
        let province = layer.province.as_mut().unwrap();
        for y in top..bottom {
            for x in left..right {
                let alpha = image.rgba[(y * image.width as usize + x) * 4 + 3];
                let value = if threshold < 0 {
                    i32::from(alpha)
                } else if i32::from(alpha) >= threshold {
                    above
                } else {
                    below
                };
                // Out-of-range labels preserve the existing province pixel.
                if let Ok(value) = u8::try_from(value) {
                    province.pixels[y * province.width + x] = value;
                }
            }
        }
        layer.image_modified = true;
        Ok(Value::Void)
    }
}
