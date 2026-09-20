//! Native BMP headers and ordered palette dithering from GraphicsLoaderIntf.cpp
//! and tvpgl.c. File writes use the session's isolated storage overlay.
use super::*;

fn dither(pixel: &[u8], x: usize, y: usize) -> u8 {
    const MATRIX: [[u8; 4]; 4] = [[0, 12, 2, 14], [8, 4, 10, 6], [3, 15, 1, 13], [11, 7, 9, 5]];
    // Upstream stores the dither table with its x/y indices transposed.
    let quantize = |v: u8, levels: u8, threshold: u8| {
        let value = v as f64 / 255.0 * levels as f64;
        value as u8 + u8::from((threshold as f64 / 16.0) < value.fract())
    };
    quantize(pixel[0], 5, MATRIX[(x + 1) % 2][(y + 1) % 2])
        + quantize(pixel[1], 6, MATRIX[x % 4][(y + 1) % 2]) * 6
        + quantize(pixel[2], 5, MATRIX[x % 4][y % 4]) * 42
}
fn bmp(image: &Image, bytes: usize) -> Vec<u8> {
    let stride = (image.width as usize * bytes + 3) & !3;
    let offset = 54 + if bytes == 1 { 1024 } else { 0 };
    let size = offset + stride * image.height as usize;
    let mut output = vec![0u8; size];
    output[..2].copy_from_slice(b"BM");
    for (at, value) in [
        (2, size as u32),
        (10, offset as u32),
        (14, 40),
        (18, image.width),
        (22, image.height),
    ] {
        output[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    output[26..28].copy_from_slice(&1u16.to_le_bytes());
    output[28..30].copy_from_slice(&(bytes as u16 * 8).to_le_bytes());
    if bytes == 1 {
        for i in 0..252 {
            output[54 + i * 4..58 + i * 4].copy_from_slice(&[
                (i / 42 * 255 / 5) as u8,
                (i / 6 % 7 * 255 / 6) as u8,
                (i % 6 * 255 / 5) as u8,
                0,
            ]);
        }
    }
    for y in 0..image.height as usize {
        for x in 0..image.width as usize {
            let p = &image.rgba[(y * image.width as usize + x) * 4..][..4];
            let at = offset + (image.height as usize - 1 - y) * stride + x * bytes;
            if bytes == 1 {
                output[at] = dither(p, x, y);
            } else {
                output[at..at + 3].copy_from_slice(&[p[2], p[1], p[0]]);
                if bytes == 4 {
                    output[at + 3] = p[3];
                }
            }
        }
    }
    output
}
impl Services {
    pub(super) fn layer_save_image(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let name = args
            .first()
            .context("Layer.saveLayerImage: missing filename")?
            .text();
        let mode = args
            .get(1)
            .filter(|v| !matches!(v, Value::Void))
            .map(Value::text)
            .unwrap_or_else(|| "bmp".into());
        if !mode.starts_with("bmp") && mode != ".bmp" && mode != ".dib" {
            return Err(unsupported(format!("Layer.saveLayerImage format {mode}")));
        }
        let bytes = match mode.as_str() {
            "bmp8" => 1,
            "bmp24" => 3,
            _ => 4,
        };
        let image = self.layers[&id].bitmap()?;
        *budget = budget
            .checked_sub(image.width as u64 * image.height as u64)
            .ok_or_else(|| unsupported("saveLayerImage execution budget exceeded"))?;
        let path = crate::save_storage::path(&self.storage.project, &self.save_dir, &name)?;
        crate::save_storage::write(&path, &bmp(image, bytes), None)?;
        self.image_cache.clear();
        Ok(Value::Void)
    }
}
