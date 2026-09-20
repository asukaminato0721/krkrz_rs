//! Graphic loading semantics from Kirikiri GraphicsLoaderIntf/LayerIntf.
use super::*;
use krkrz_assets::media::IndexedImage;
use std::{collections::BTreeMap, sync::Arc};

// Graphic handlers are searched in reverse registration order.
const EXTENSIONS: &[&str] = &[
    ".jxr", ".tlg6", ".tlg5", ".tlg", ".png", ".jif", ".jpg", ".jpeg", ".dib", ".bmp",
];

fn extension(name: &str) -> &str {
    let base = name.rsplit(['/', '\\', '>']).next().unwrap_or(name);
    base.rfind('.').map_or("", |i| &base[i..])
}

impl Services {
    fn suggest_graphic(&self, name: &str) -> Option<String> {
        EXTENSIONS
            .iter()
            .find_map(|ext| self.storage.resolve(&format!("{name}{ext}")).ok())
    }

    fn read_graphic(
        &mut self,
        name: &str,
        budget: &mut u64,
    ) -> Result<(Image, BTreeMap<String, String>)> {
        let bytes = self.read_storage(name)?;
        let tags = Image::metadata(&bytes)?;
        let key = self.storage.placed_path(name)?;
        let image = match self.image_cache.get(&key) {
            Some(image) => image,
            None => {
                let image = Arc::new(
                    Image::decode(&bytes).with_context(|| format!("load graphic {name}"))?,
                );
                pixel_count(image.width as i32, image.height as i32)?;
                self.image_cache.insert(key, image.clone());
                image
            }
        };
        *budget = budget
            .checked_sub(image.width as u64 * image.height as u64)
            .ok_or_else(|| unsupported("image loading execution budget exceeded"))?;
        Ok(((*image).clone(), tags))
    }

    pub(super) fn layer_load_images(
        &mut self,
        vm: &mut Vm,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        self.layers[&id].bitmap()?;
        let name = args
            .first()
            .context("Layer.loadImages requires a storage")?
            .text();
        let key = match args.get(1) {
            None | Some(Value::Void) => 0x1fffffff,
            Some(v) => v.integer()? as u32,
        };
        let ext = extension(&name);
        let stem = &name[..name.len() - ext.len()];
        let resolved = if ext.is_empty() {
            self.suggest_graphic(&name)
                .with_context(|| format!("cannot suggest graphic extension: {name}"))?
        } else {
            self.storage.resolve(&name)?
        };
        let (mut image, tags) = self.read_graphic(&resolved, budget)?;
        if key & 0xff000000 == 0x03000000 {
            let bytes = self.read_storage(&resolved)?;
            // Palette keys have no effect on true-color formats.
            if bytes.starts_with(b"BM")
                || (bytes.starts_with(b"\x89PNG") && bytes.get(25) == Some(&3))
            {
                let indices = IndexedImage::decode(&bytes)?;
                for (p, index) in image.rgba.as_chunks_mut::<4>().0.iter_mut().zip(indices.pixels) {
                    p[3] = if index == key as u8 { 0 } else { 255 };
                }
            }
        } else {
            apply_key(&mut image, key)?;
        }
        let mask = format!("{stem}_m");
        let mask = if ext.is_empty() {
            None
        } else {
            self.storage.resolve(&format!("{mask}{ext}")).ok()
        }
        .or_else(|| self.suggest_graphic(&mask));
        if let Some(mask) = mask {
            let bytes = self.read_storage(&mask)?;
            let palette = bytes.starts_with(b"BM")
                || (bytes.starts_with(b"\x89PNG") && bytes.get(25) == Some(&3));
            let (mask, _) = self.read_graphic(&mask, budget)?;
            ensure!(
                image.width == mask.width && image.height == mask.height,
                "mask image size mismatch"
            );
            for (p, m) in image
                .rgba
                .as_chunks_mut::<4>().0.iter_mut()
                .zip(mask.rgba.as_chunks::<4>().0.iter())
            {
                // libpng's RGB-to-gray call in Kirikiri uses zero red/green
                // coefficients. Gray images already have equal RGB channels.
                p[3] = if palette {
                    ((m[0] as u32 * 77 + m[1] as u32 * 150 + m[2] as u32 * 29) >> 8) as u8
                } else {
                    m[2]
                };
            }
        }
        if key & 0xff000000 == 0x04000000 {
            let mat = [(key >> 16) as u8, (key >> 8) as u8, key as u8];
            for p in image.rgba.as_chunks_mut::<4>().0.iter_mut() {
                for c in 0..3 {
                    p[c] =
                        (mat[c] as i32 + (((p[c] as i32 - mat[c] as i32) * p[3] as i32) >> 8)) as u8;
                }
                p[3] = 255;
            }
        }
        let province = self
            .suggest_graphic(&format!("{stem}_p"))
            .map(|name| self.read_province(&name, image.width, image.height, budget))
            .transpose()?;
        let used: usize = self
            .layers
            .iter()
            .filter(|(key, _)| **key != id)
            .map(|(_, l)| {
                l.image.as_ref().map_or(0, |i| i.rgba.len())
                    + l.province.as_ref().map_or(0, |p| p.pixels.len())
            })
            .sum();
        ensure!(
            used + image.rgba.len() + province.as_ref().map_or(0, |p| p.pixels.len()) <= 256 << 20,
            "session layer image memory limit exceeded"
        );
        let layer = self.layers.get_mut(&id).unwrap();
        let (w, h) = (image.width as i32, image.height as i32);
        if w < layer.width {
            layer.width = w;
            layer.image_left = 0;
        }
        if h < layer.height {
            layer.height = h;
            layer.image_top = 0;
        }
        layer.image_left = layer.image_left.max(layer.width - w);
        layer.image_top = layer.image_top.max(layer.height - h);
        layer.image = Some(image);
        layer.province = province;
        layer.image_modified = true;
        layer.reset_clip()?;
        if tags.is_empty() {
            return Ok(Value::NULL);
        }
        let dict = vm.new_dictionary()?;
        for (name, value) in tags {
            vm.set_member(&dict, &Value::string(&name), Value::string(&value))?;
        }
        Ok(dict)
    }

    fn read_province(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        budget: &mut u64,
    ) -> Result<Province> {
        *budget = budget
            .checked_sub(width as u64 * height as u64)
            .ok_or_else(|| unsupported("province loading execution budget exceeded"))?;
        let decoded = IndexedImage::decode(&self.read_storage(name)?)
            .with_context(|| format!("load province {name}"))?;
        ensure!(
            decoded.width <= width && decoded.height <= height,
            "province image size mismatch"
        );
        let mut pixels = vec![0; width as usize * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                pixels[y * width as usize + x] = decoded.pixels[(y % decoded.height as usize)
                    * decoded.width as usize
                    + x % decoded.width as usize];
            }
        }
        Ok(Province {
            width: width as usize,
            height: height as usize,
            pixels,
        })
    }

    pub(super) fn layer_load_province(
        &mut self,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let name = args
            .first()
            .context("Layer.loadProvinceImage requires a storage")?
            .text();
        let name = if extension(&name).is_empty() {
            self.suggest_graphic(&name)
                .context("cannot suggest province extension")?
        } else {
            self.storage.resolve(&name)?
        };
        let image = self.layers[&id].bitmap()?;
        let (width, height) = (image.width, image.height);
        let used: usize = self
            .layers
            .iter()
            .map(|(other, l)| {
                l.image.as_ref().map_or(0, |i| i.rgba.len())
                    + if *other == id {
                        0
                    } else {
                        l.province.as_ref().map_or(0, |p| p.pixels.len())
                    }
            })
            .sum();
        ensure!(
            used + width as usize * height as usize <= 256 << 20,
            "session layer image memory limit exceeded"
        );
        let province = self.read_province(&name, width, height, budget)?;
        let layer = self.layers.get_mut(&id).unwrap();
        layer.province = Some(province);
        layer.image_modified = true;
        Ok(Value::Void)
    }
}

fn apply_key(image: &mut Image, key: u32) -> Result<()> {
    let transparent = if key == 0x01ffffff {
        // Preserve the original adaptive algorithm, including its unusual
        // accumulated run count when a run does not beat the current maximum.
        let mut colors: Vec<_> = image.rgba[..image.width as usize * 4]
            .as_chunks::<4>().0.iter()
            .map(|p| color(p))
            .collect();
        colors.sort_unstable();
        colors.push(u32::MAX);
        let (mut previous, mut count, mut maximum, mut selected) = (u32::MAX, 0, 0, 0);
        for c in colors {
            if c != previous {
                if maximum < count {
                    maximum = count;
                    selected = previous;
                    count = 0;
                }
            } else {
                count += 1;
            }
            previous = c;
        }
        Some(selected)
    } else if key & 0xff000000 == 0 {
        Some(key)
    } else {
        None
    };
    if let Some(key) = transparent {
        for p in image.rgba.as_chunks_mut::<4>().0.iter_mut() {
            p[3] = if color(p) == key { 0 } else { 255 };
        }
    }
    Ok(())
}
fn color(p: &[u8]) -> u32 {
    ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32
}
