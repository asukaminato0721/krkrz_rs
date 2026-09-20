use anyhow::{Result, ensure};
use krkrz_assets::media::Image;
#[derive(Clone, Debug)]
pub struct Layer {
    pub image: Image,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub opacity: u8,
    pub visible: bool,
}
/// Straight-alpha source-over compositing with deterministic integer rounding.
/// Layer-specific Kirikiri blend modes must be added before KAG rendering is supported.
pub fn compose(width: u32, height: u32, background: [u8; 4], layers: &[Layer]) -> Result<Image> {
    ensure!(
        width > 0 && height > 0 && (width as u64) * (height as u64) <= 64 * 1024 * 1024,
        "invalid canvas dimensions"
    );
    let mut rgba = background.repeat(width as usize * height as usize);
    let mut order = layers.iter().enumerate().collect::<Vec<_>>();
    order.sort_by_key(|(i, l)| (l.z, *i));
    for (_, layer) in order {
        let image = &layer.image;
        ensure!(
            u64::from(image.width) * u64::from(image.height) * 4 == image.rgba.len() as u64,
            "invalid layer image size"
        );
        if !layer.visible || layer.opacity == 0 {
            continue;
        }
        let left = i64::from(layer.x).max(0);
        let top = i64::from(layer.y).max(0);
        let right = (i64::from(layer.x) + i64::from(image.width)).min(i64::from(width));
        let bottom = (i64::from(layer.y) + i64::from(image.height)).min(i64::from(height));
        for y in top..bottom {
            for x in left..right {
                let source = (((y - i64::from(layer.y)) * i64::from(image.width)
                    + (x - i64::from(layer.x)))
                    * 4) as usize;
                let target = (y * i64::from(width) * 4 + x * 4) as usize;
                let a = (u32::from(image.rgba[source + 3]) * u32::from(layer.opacity) + 127) / 255;
                let dest_a = u32::from(rgba[target + 3]);
                let total = a * 255 + dest_a * (255 - a);
                for c in 0..3 {
                    rgba[target + c] = (u32::from(image.rgba[source + c]) * a * 255
                        + u32::from(rgba[target + c]) * dest_a * (255 - a)
                        + total / 2)
                        .checked_div(total)
                        .unwrap_or(0) as u8;
                }
                rgba[target + 3] = ((total + 127) / 255) as u8;
            }
        }
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipped_alpha_and_order() {
        let layers = vec![Layer {
            image: Image {
                width: 2,
                height: 1,
                rgba: vec![255, 0, 0, 128, 255, 0, 0, 128],
            },
            x: -1,
            y: 0,
            z: 1,
            opacity: 255,
            visible: true,
        }];
        let out = compose(2, 1, [0, 0, 255, 255], &layers).unwrap();
        assert_eq!(out.rgba, [128, 0, 127, 255, 0, 0, 255, 255]);
    }
}
