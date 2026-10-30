//! LayerIntf.cpp flips the complete main and province images, ignoring ClipRect and DrawFace.
use super::*;

pub(super) fn flip_rows(bytes: &mut [u8], stride: usize, height: usize) {
    for y in 0..height / 2 {
        let (first, last) = bytes.split_at_mut((height - y - 1) * stride);
        first[y * stride..(y + 1) * stride].swap_with_slice(&mut last[..stride]);
    }
}

impl Services {
    pub(super) fn layer_flip(
        &mut self,
        id: usize,
        vertical: bool,
        budget: &mut u64,
    ) -> Result<Value> {
        let layer = &self.layers[&id];
        let image = layer.bitmap()?;
        let count = image.rgba.len() / 4 + layer.province.as_ref().map_or(0, |p| p.pixels.len());
        *budget = budget
            .checked_sub(count as u64)
            .ok_or_else(|| unsupported("Layer flip execution budget exceeded"))?;
        let available = image_available(&self.layers, id);
        let layer = self.layers.get_mut(&id).unwrap();
        layer.prepare_image_write(available)?;
        let image = Arc::make_mut(layer.image.as_mut().unwrap());
        if vertical {
            flip_rows(
                &mut image.rgba,
                image.width as usize * 4,
                image.height as usize,
            );
        } else {
            for row in image
                .rgba
                .as_chunks_mut::<4>()
                .0
                .chunks_mut(image.width as usize)
            {
                row.reverse();
            }
        }
        if let Some(province) = &mut layer.province {
            if vertical {
                flip_rows(&mut province.pixels, province.width, province.height);
            } else {
                for row in province.pixels.chunks_mut(province.width) {
                    row.reverse();
                }
            }
        }
        layer.image_modified = true;
        Ok(Value::Void)
    }
}
