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
    pub(crate) fn touch_images(
        &mut self,
        vm: &mut Vm,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let array = args
            .first()
            .context("System.touchImages requires an object")?;
        object(array)?.context("System.touchImages requires an object")?;
        let mut names = Vec::new();
        loop {
            *budget = budget
                .checked_sub(1)
                .ok_or_else(|| unsupported("touchImages execution budget exceeded"))?;
            let value = vm.get_property(
                array,
                &Value::Integer(names.len() as i64),
                true,
                false,
                self,
                budget,
            )?;
            if matches!(value, Value::Void) {
                break;
            }
            ensure!(
                names.len() < 1_000_000,
                "touchImages storage count limit exceeded"
            );
            names.push(value.text());
        }
        let maximum = self.image_cache.limit() as i64;
        let limit = args.get(1).map(Value::integer).transpose()?.unwrap_or(0);
        let limit = if limit < 0 {
            maximum.saturating_add(limit).max(0)
        } else if limit == 0 {
            maximum
        } else {
            limit.min(maximum)
        };
        let timeout = args.get(2).map(Value::integer).transpose()?.unwrap_or(0) as u64;
        let start = std::time::Instant::now();
        let mut bytes = 0;
        let mut touched = Vec::new();
        for name in names {
            if bytes >= limit || (timeout != 0 && start.elapsed().as_millis() >= timeout as u128) {
                break;
            }
            let result = (|| -> Result<(String, Arc<Image>)> {
                let name = if extension(&name).is_empty() {
                    self.suggest_graphic(&name)
                        .context("cannot suggest graphic extension")?
                } else {
                    self.resolve_graphic(&name)?
                };
                let image = self.read_graphic(&name, budget)?.0;
                let key = self.graphic_cache_key(&name)?;
                Ok((key, image))
            })();
            match result {
                Ok((key, image)) => {
                    bytes += image.rgba.len() as i64;
                    touched.push(key);
                }
                // Native prefetch ignores individual load failures. Resource
                // and interpreter budgets still terminate the Rust session.
                Err(error) if error.downcast_ref::<krkrz_tjs::VmAbort>().is_some() => {
                    return Err(error);
                }
                Err(_) => {}
            }
        }
        for key in touched.iter().rev() {
            self.image_cache.get(key);
        }
        Ok(Value::Void)
    }

    fn graphic_cache_key(&self, name: &str) -> Result<String> {
        match crate::save_storage::path(&self.storage.project, &self.save_dir, name) {
            Ok(path) if path.is_file() => Ok(path.to_string_lossy().into_owned()),
            _ => self.storage.placed_path(name),
        }
    }

    pub(super) fn layer_assign_images(
        &mut self,
        id: usize,
        source: &Value,
        budget: &mut u64,
    ) -> Result<Value> {
        let source = object(source)?.context("image source is null")?;
        let src = self
            .layers
            .get(&source)
            .context("image source is not a Layer")?;
        if source == id {
            let layer = self.layers.get_mut(&id).unwrap();
            layer.image_modified = true;
            if layer.image.is_some() {
                layer.reset_clip()?;
            }
            return Ok(Value::Void);
        }
        let bytes = src.image.as_ref().map_or(0, |i| i.rgba.len())
            + src.province.as_ref().map_or(0, |p| p.pixels.len());
        let used = image_bytes(&self.layers, Some(id));
        ensure!(
            used + src.province.as_ref().map_or(0, |p| p.pixels.len()) <= MAX_LAYER_IMAGE_BYTES,
            "session layer image memory limit exceeded"
        );
        *budget = budget
            .checked_sub(bytes as u64 / 4)
            .ok_or_else(|| unsupported("image assignment execution budget exceeded"))?;
        let (image, province) = (src.image.clone(), src.province.clone());
        let dest = self.layers.get_mut(&id).unwrap();
        dest.image = image;
        dest.province = province;
        if let Some(image) = &dest.image {
            let (w, h) = (image.width as i32, image.height as i32);
            dest.width = dest.width.min(w);
            dest.height = dest.height.min(h);
            dest.image_left = dest.image_left.max(dest.width - w);
            dest.image_top = dest.image_top.max(dest.height - h);
            dest.reset_clip()?;
        }
        dest.image_modified = true;
        Ok(Value::Void)
    }

    pub(super) fn suggest_graphic(&self, name: &str) -> Option<String> {
        EXTENSIONS
            .iter()
            .find_map(|ext| self.resolve_graphic(&format!("{name}{ext}")).ok())
    }

    pub(super) fn resolve_graphic(&self, name: &str) -> Result<String> {
        if let Ok(path) = crate::save_storage::path(&self.storage.project, &self.save_dir, name)
            && path.is_file()
        {
            return Ok(path.to_string_lossy().into_owned());
        }
        self.storage.resolve(name)
    }

    pub(super) fn read_graphic(
        &mut self,
        name: &str,
        budget: &mut u64,
    ) -> Result<(Arc<Image>, BTreeMap<String, String>)> {
        let bytes = self.read_storage(name)?;
        let tags = Image::metadata(&bytes)?;
        let key = self.graphic_cache_key(name)?;
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
        Ok((image, tags))
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
            self.resolve_graphic(&name)?
        };
        let (mut image, tags) = self.read_graphic(&resolved, budget)?;
        if key & 0xff000000 == 0x03000000 {
            let bytes = self.read_storage(&resolved)?;
            // Palette keys have no effect on true-color formats.
            if bytes.starts_with(b"BM")
                || (bytes.starts_with(b"\x89PNG") && bytes.get(25) == Some(&3))
            {
                let indices = IndexedImage::decode(&bytes)?;
                for (p, index) in Arc::make_mut(&mut image)
                    .rgba
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(indices.pixels)
                {
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
            self.resolve_graphic(&format!("{mask}{ext}")).ok()
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
            for (p, m) in Arc::make_mut(&mut image)
                .rgba
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
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
            for p in Arc::make_mut(&mut image)
                .rgba
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
            {
                for c in 0..3 {
                    p[c] = (mat[c] as i32 + (((p[c] as i32 - mat[c] as i32) * p[3] as i32) >> 8))
                        as u8;
                }
                p[3] = 255;
            }
        }
        let province = self
            .suggest_graphic(&format!("{stem}_p"))
            .map(|name| self.read_province(&name, image.width, image.height, budget))
            .transpose()?;
        let used = image_bytes(&self.layers, Some(id));
        let already_live = self.layers.iter().any(|(other, layer)| {
            *other != id
                && layer
                    .image
                    .as_ref()
                    .is_some_and(|other| Arc::ptr_eq(other, &image))
        });
        let image_bytes = if already_live { 0 } else { image.rgba.len() };
        ensure!(
            used + image_bytes + province.as_ref().map_or(0, |p| p.pixels.len())
                <= MAX_LAYER_IMAGE_BYTES,
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
            self.resolve_graphic(&name)?
        };
        let image = self.layers[&id].bitmap()?;
        let (width, height) = (image.width, image.height);
        let used = image_bytes(&self.layers, None)
            - self.layers[&id]
                .province
                .as_ref()
                .map_or(0, |p| p.pixels.len());
        ensure!(
            used + width as usize * height as usize <= MAX_LAYER_IMAGE_BYTES,
            "session layer image memory limit exceeded"
        );
        let province = self.read_province(&name, width, height, budget)?;
        let layer = self.layers.get_mut(&id).unwrap();
        layer.province = Some(province);
        layer.image_modified = true;
        Ok(Value::Void)
    }
}

fn apply_key(image: &mut Arc<Image>, key: u32) -> Result<()> {
    let transparent = if key == 0x01ffffff {
        // Preserve the original adaptive algorithm, including its unusual
        // accumulated run count when a run does not beat the current maximum.
        let mut colors: Vec<_> = image.rgba[..image.width as usize * 4]
            .as_chunks::<4>()
            .0
            .iter()
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
        for p in Arc::make_mut(image).rgba.as_chunks_mut::<4>().0.iter_mut() {
            p[3] = if color(p) == key { 0 } else { 255 };
        }
    }
    Ok(())
}
fn color(p: &[u8]) -> u32 {
    ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_images_share_until_color_key_changes_pixels() {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        Image {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        }
        .write_png(&project.path().join("shared.png"))
        .unwrap();
        let mut session =
            crate::Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
        session.services.image_cache.set_limit(4096);
        let (first, _) = session
            .services
            .read_graphic("shared.png", &mut 100_000)
            .unwrap();
        let (mut second, _) = session
            .services
            .read_graphic("shared.png", &mut 100_000)
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        apply_key(&mut second, 0x1fffffff).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        apply_key(&mut second, 0x010203).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first.rgba[3], 255);
        assert_eq!(second.rgba[3], 0);
        let (cached, _) = session
            .services
            .read_graphic("shared.png", &mut 100_000)
            .unwrap();
        assert!(Arc::ptr_eq(&first, &cached));
    }
}
