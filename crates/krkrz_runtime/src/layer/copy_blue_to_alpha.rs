//! layerExSave copies source blue into destination alpha across the shared
//! image extent. ClipRect, DrawFace and layer opacity do not affect this copy.
use super::*;

impl Services {
    pub(super) fn layer_copy_blue_to_alpha(
        &mut self,
        id: usize,
        source: &Value,
        budget: &mut u64,
    ) -> Result<Value> {
        let source_id = object(source)?.context("source must be a Layer")?;
        let source = self
            .layers
            .get(&source_id)
            .context("source must be a Layer")?;
        source.bitmap()?;
        let source = source.image.as_ref().unwrap().clone();
        let destination = self.layers[&id].bitmap()?;
        let width = source.width.min(destination.width) as usize;
        let height = source.height.min(destination.height) as usize;
        *budget = budget
            .checked_sub((width * height) as u64)
            .ok_or_else(|| unsupported("Layer.copyBlueToAlpha execution budget exceeded"))?;
        let available = image_available(&self.layers, id);
        let layer = self.layers.get_mut(&id).unwrap();
        layer.prepare_image_write(available)?;
        let destination = Arc::make_mut(layer.image.as_mut().unwrap());
        for y in 0..height {
            for x in 0..width {
                destination.rgba[(y * destination.width as usize + x) * 4 + 3] =
                    source.rgba[(y * source.width as usize + x) * 4 + 2];
            }
        }
        layer.image_modified = true;
        Ok(Value::Void)
    }
}
