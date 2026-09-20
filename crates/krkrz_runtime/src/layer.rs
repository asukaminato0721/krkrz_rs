//! Native layer surfaces and hierarchy, following krkrz visual/LayerIntf.cpp.
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_assets::media::Image;
use krkrz_tjs::{ObjectRef, Value, Vm, unsupported};

pub(crate) struct Layer {
    constructed: bool,
    primary: bool,
    window: usize,
    root: usize,
    parent: Option<usize>,
    children: Vec<usize>,
    children_array: Option<Value>,
    children_dirty: bool,
    font: Option<Value>,
    name: String,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    image_left: i32,
    image_top: i32,
    pub(crate) image: Option<Image>,
    clip: [i32; 4],
    kind: i32,
    face: i32,
    hold_alpha: bool,
    visible: bool,
    enabled: bool,
    opacity: i32,
    hit_type: i32,
    hit_threshold: i32,
    neutral: [u8; 4],
    absolute_mode: bool,
    absolute: i32,
}
impl Default for Layer {
    fn default() -> Self {
        Self {
            constructed: false,
            primary: false,
            window: 0,
            root: 0,
            parent: None,
            children: Vec::new(),
            children_array: None,
            children_dirty: true,
            font: None,
            name: String::new(),
            left: 0,
            top: 0,
            width: 32,
            height: 32,
            image_left: 0,
            image_top: 0,
            image: Some(Image {
                width: 32,
                height: 32,
                rgba: [255, 255, 255, 0].repeat(32 * 32),
            }),
            clip: [0, 0, 32, 32],
            kind: 2,
            face: 128,
            hold_alpha: false,
            visible: false,
            enabled: true,
            opacity: 255,
            hit_type: 0,
            hit_threshold: 16,
            neutral: [255, 255, 255, 0],
            absolute_mode: false,
            absolute: 0,
        }
    }
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    crate::plugins::declare_class(vm, "Layer")?;
    let class = vm.globals["Layer"].clone();
    for key in ["children", "window", "isPrimary", "font"] {
        vm.register_native_property(&class, key, Some(&format!("Layer.get:{key}")), None)?;
    }
    Ok(())
}
pub(crate) fn object(value: &Value) -> Result<Option<usize>> {
    let Value::Object(reference) = value else {
        anyhow::bail!("expected an Object");
    };
    Ok(reference.object)
}
fn bound(id: usize) -> Value {
    Value::Object(ObjectRef {
        object: Some(id),
        context: Some(id),
    })
}
fn int(value: &Value) -> Result<i32> {
    Ok(value.integer()? as i32)
}
fn pixel_count(w: i32, h: i32) -> Result<usize> {
    ensure!(w > 0 && h > 0, "cannot create an empty layer image");
    let n = w as u64 * h as u64;
    if n > 16 * 1024 * 1024 {
        return Err(unsupported("layer image exceeds 16 megapixel limit"));
    }
    Ok(n as usize)
}
// High-byte color identifiers use the Win32 COLORREF byte order.
fn rgb(color: u32) -> Result<[u8; 3]> {
    if color & 0x80000000 != 0 {
        // Invalid GetSysColor indices return black. Valid platform colors require a host palette.
        if color & 0xff <= 30 {
            return Err(unsupported("system color palette is not configured"));
        }
        return Ok([0, 0, 0]);
    }
    if color & 0xff000000 != 0 {
        Ok([color as u8, (color >> 8) as u8, (color >> 16) as u8])
    } else {
        Ok([(color >> 16) as u8, (color >> 8) as u8, color as u8])
    }
}
impl Layer {
    fn bitmap(&self) -> Result<&Image> {
        self.image.as_ref().context("layer has no image")
    }
    fn reset_clip(&mut self) -> Result<()> {
        let image = self.bitmap()?;
        self.clip = [0, 0, image.width as i32, image.height as i32];
        Ok(())
    }
    fn allocate(&mut self, available: usize) -> Result<()> {
        if self.image.is_none() {
            let n = pixel_count(self.width, self.height)?;
            ensure!(
                n * 4 <= available,
                "session layer image memory limit exceeded"
            );
            self.image = Some(Image {
                width: self.width as u32,
                height: self.height as u32,
                rgba: self.neutral.repeat(n),
            });
            self.image_left = 0;
            self.image_top = 0;
        }
        self.reset_clip()
    }
    fn resize_image(&mut self, w: i32, h: i32, available: usize) -> Result<()> {
        let n = pixel_count(w, h)?;
        ensure!(
            n * 4 <= available,
            "session layer image memory limit exceeded"
        );
        let old = self.bitmap()?;
        let mut rgba = self.neutral.repeat(n);
        for y in 0..h.min(old.height as i32) as usize {
            let len = w.min(old.width as i32) as usize * 4;
            rgba[y * w as usize * 4..y * w as usize * 4 + len].copy_from_slice(
                &old.rgba[y * old.width as usize * 4..y * old.width as usize * 4 + len],
            );
        }
        self.image = Some(Image {
            width: w as u32,
            height: h as u32,
            rgba,
        });
        self.reset_clip()
    }
    fn size(&mut self, w: i32, h: i32, available: usize) -> Result<()> {
        ensure!(w >= 0 && h >= 0, "negative layer dimension");
        if w > 32768 || h > 32768 {
            return Err(unsupported("layer dimension exceeds limit"));
        }
        if let Some(image) = &self.image {
            let iw = image.width as i32;
            let ih = image.height as i32;
            if w > iw || h > ih {
                self.resize_image(w.max(iw), h.max(ih), available)?;
            }
            self.image_left = self.image_left.max(w - iw.max(w));
            self.image_top = self.image_top.max(h - ih.max(h));
        }
        self.width = w;
        self.height = h;
        Ok(())
    }
    fn image_size(&mut self, w: i32, h: i32, available: usize) -> Result<()> {
        let image = self.bitmap()?;
        if (w, h) == (image.width as i32, image.height as i32) {
            return Ok(());
        }
        self.resize_image(w, h, available)?;
        self.width = self.width.min(w);
        self.height = self.height.min(h);
        self.image_left = self.image_left.max(self.width - w);
        self.image_top = self.image_top.max(self.height - h);
        Ok(())
    }
    fn set_clip(&mut self, x: i32, y: i32, w: i32, h: i32) -> Result<()> {
        let image = self.bitmap()?;
        self.clip = [
            x.max(0),
            y.max(0),
            x.wrapping_add(w).min(image.width as i32).max(x.max(0)),
            y.wrapping_add(h).min(image.height as i32).max(y.max(0)),
        ];
        Ok(())
    }
    fn draw_face(&self) -> i32 {
        if self.face != 128 {
            self.face
        } else {
            match self.kind {
                2 | 13..=28 => 0,
                12 => 4,
                _ => 1,
            }
        }
    }
    fn inside_clip(&self, x: i32, y: i32) -> bool {
        x >= self.clip[0] && y >= self.clip[1] && x < self.clip[2] && y < self.clip[3]
    }
    fn point(&self, x: i32, y: i32) -> Result<usize> {
        let image = self.bitmap()?;
        ensure!(
            x >= 0 && y >= 0 && x < image.width as i32 && y < image.height as i32,
            "layer pixel is outside the image"
        );
        Ok((y as usize * image.width as usize + x as usize) * 4)
    }
    fn call(&mut self, op: &str, args: &[Value], available: usize) -> Result<Value> {
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("Layer.{op}: missing argument {i}"))
        };
        let n = |i: usize| -> Result<i32> { int(arg(i)?) };
        if let Some(key) = op.strip_prefix("get:") {
            let v = match key {
                "name" => return Ok(Value::string(&self.name)),
                "left" => self.left,
                "top" => self.top,
                "width" => self.width,
                "height" => self.height,
                "imageLeft" => {
                    self.bitmap()?;
                    self.image_left
                }
                "imageTop" => {
                    self.bitmap()?;
                    self.image_top
                }
                "imageWidth" => self.bitmap()?.width as i32,
                "imageHeight" => self.bitmap()?.height as i32,
                "clipLeft" => self.clip[0],
                "clipTop" => self.clip[1],
                "clipWidth" => self.clip[2] - self.clip[0],
                "clipHeight" => self.clip[3] - self.clip[1],
                "hasImage" => self.image.is_some().into(),
                "type" => self.kind,
                "face" => self.face,
                "holdAlpha" => self.hold_alpha.into(),
                "visible" => self.visible.into(),
                "enabled" => self.enabled.into(),
                "opacity" => self.opacity,
                "hitType" => self.hit_type,
                "hitThreshold" => self.hit_threshold,
                _ => return Err(unsupported(format!("unsupported Layer operation: {op}"))),
            };
            return Ok(Value::Integer(v.into()));
        }
        match op {
            "finalize" => {}
            "set:name" => self.name = arg(0)?.text(),
            "set:visible" => self.visible = arg(0)?.truth()?,
            "set:enabled" => self.enabled = arg(0)?.truth()?,
            "set:opacity" => self.opacity = n(0)?.clamp(0, 255),
            "set:holdAlpha" => self.hold_alpha = arg(0)?.truth()?,
            "set:face" => self.face = n(0)?,
            "set:hitType" => self.hit_type = n(0)?,
            "set:hitThreshold" => self.hit_threshold = n(0)?,
            "set:hasImage" => {
                if arg(0)?.truth()? {
                    ensure!(
                        !matches!(self.kind, 0 | 6 | 7),
                        "layer type cannot have an image"
                    );
                    self.allocate(available)?;
                } else {
                    self.image = None;
                }
            }
            "set:type" => {
                let kind = n(0)?;
                ensure!((0..=28).contains(&kind), "invalid layer type");
                if kind != self.kind {
                    self.kind = kind;
                    self.neutral = match kind {
                        3 | 8 | 10..=14 | 17 | 21 | 22 | 24 | 26..=28 => [0, 0, 0, 0],
                        18..=20 => [128, 128, 128, 0],
                        _ => [255, 255, 255, 0],
                    };
                    if matches!(kind, 0 | 6 | 7) {
                        self.image = None;
                    } else {
                        self.allocate(available)?;
                    }
                }
            }
            "set:left" | "set:top" | "setPos" => {
                let (x, y) = match op {
                    "set:left" => (n(0)?, self.top),
                    "set:top" => (self.left, n(0)?),
                    _ => (n(0)?, n(1)?),
                };
                ensure!(
                    !self.primary || (x == 0 && y == 0),
                    "cannot move primary layer"
                );
                self.left = x;
                self.top = y;
                if op == "setPos" && args.len() >= 4 {
                    self.size(n(2)?, n(3)?, available)?;
                }
            }
            "set:width" => self.size(n(0)?, self.height, available)?,
            "set:height" => self.size(self.width, n(0)?, available)?,
            "setSize" => self.size(n(0)?, n(1)?, available)?,
            "setImageSize" => self.image_size(n(0)?, n(1)?, available)?,
            "set:imageWidth" => self.image_size(n(0)?, self.bitmap()?.height as i32, available)?,
            "set:imageHeight" => self.image_size(self.bitmap()?.width as i32, n(0)?, available)?,
            "setSizeToImageSize" => {
                let i = self.bitmap()?;
                self.size(i.width as i32, i.height as i32, available)?;
            }
            "setImagePos" | "set:imageLeft" | "set:imageTop" => {
                let (x, y) = match op {
                    "set:imageLeft" => (n(0)?, self.image_top),
                    "set:imageTop" => (self.image_left, n(0)?),
                    _ => (n(0)?, n(1)?),
                };
                let i = self.bitmap()?;
                ensure!(
                    x <= 0
                        && y <= 0
                        && i64::from(i.width) + i64::from(x) >= i64::from(self.width)
                        && i64::from(i.height) + i64::from(y) >= i64::from(self.height),
                    "image position leaves layer bounds uncovered"
                );
                self.image_left = x;
                self.image_top = y;
            }
            "setClip" => {
                if args.is_empty() {
                    self.reset_clip()?;
                } else {
                    self.set_clip(n(0)?, n(1)?, n(2)?, n(3)?)?;
                }
            }
            "set:clipLeft" | "set:clipTop" | "set:clipWidth" | "set:clipHeight" => {
                let mut c = [
                    self.clip[0],
                    self.clip[1],
                    self.clip[2] - self.clip[0],
                    self.clip[3] - self.clip[1],
                ];
                c[match op {
                    "set:clipLeft" => 0,
                    "set:clipTop" => 1,
                    "set:clipWidth" => 2,
                    _ => 3,
                }] = n(0)?;
                self.set_clip(c[0], c[1], c[2], c[3])?;
            }
            "getMainPixel" | "getMaskPixel" => {
                let i = self.point(n(0)?, n(1)?)?;
                let p = &self.bitmap()?.rgba[i..i + 4];
                return Ok(Value::Integer(if op == "getMaskPixel" {
                    p[3] as i64
                } else {
                    ((p[0] as i64) << 16) | ((p[1] as i64) << 8) | p[2] as i64
                }));
            }
            "setMainPixel" | "setMaskPixel" => {
                let (x, y) = (n(0)?, n(1)?);
                let color = n(2)? as u32;
                self.bitmap()?;
                if self.inside_clip(x, y) {
                    let index = self.point(x, y)?;
                    let p = &mut self.image.as_mut().unwrap().rgba[index..index + 4];
                    if op == "setMaskPixel" {
                        p[3] = color as u8;
                    } else {
                        p[..3].copy_from_slice(&rgb(color)?);
                    }
                }
            }
            "fillRect" => {
                let (x, y, w, h, color) = (n(0)?, n(1)?, n(2)?, n(3)?, arg(4)?.integer()? as u32);
                let left = x.max(self.clip[0]);
                let top = y.max(self.clip[1]);
                let right = x.wrapping_add(w).min(self.clip[2]);
                let bottom = y.wrapping_add(h).min(self.clip[3]);
                if right <= left || bottom <= top {
                    return Ok(Value::Void);
                }
                let face = self.draw_face();
                if !matches!(face, 0 | 1 | 2 | 4) {
                    return Err(unsupported(format!("Layer.fillRect draw face {face}")));
                }
                let hold = face == 1 && self.hold_alpha;
                let c = if hold {
                    rgb(color)?
                } else {
                    rgb(color & 0xffffff)?
                };
                let image = self.image.as_mut().context("layer has no image")?;
                for y in top.max(0)..bottom.min(image.height as i32) {
                    for x in left.max(0)..right.min(image.width as i32) {
                        let i = (y as usize * image.width as usize + x as usize) * 4;
                        if face == 2 {
                            image.rgba[i + 3] = color as u8;
                        } else {
                            image.rgba[i..i + 3].copy_from_slice(&c);
                            if !hold {
                                image.rgba[i + 3] = (color >> 24) as u8;
                            }
                        }
                    }
                }
            }
            _ => return Err(unsupported(format!("unsupported Layer operation: {op}"))),
        }
        Ok(Value::Void)
    }
}
impl Services {
    fn layer_reparent(&mut self, id: usize, parent: Option<usize>) -> Result<()> {
        let root = self.layers[&id].root;
        if let Some(p) = parent {
            let layer = self.layers.get(&p).context("parent is not a Layer")?;
            ensure!(
                layer.constructed && layer.root == root,
                "cannot move layer under a different primary layer"
            );
            let mut ancestor = Some(p);
            while let Some(a) = ancestor {
                ensure!(a != id, "layer hierarchy cycle");
                ancestor = self.layers[&a].parent;
            }
        }
        if let Some(old) = self.layers[&id].parent {
            let layer = self.layers.get_mut(&old).unwrap();
            layer.children.retain(|child| *child != id);
            layer.children_dirty = true;
        }
        if let Some(p) = parent {
            let last = self.layers[&p]
                .children
                .last()
                .map(|c| self.layers[c].absolute.wrapping_add(1))
                .unwrap_or(0);
            if self.layers[&p].absolute_mode {
                self.layers.get_mut(&id).unwrap().absolute = last;
            }
            let layer = self.layers.get_mut(&p).unwrap();
            layer.children.push(id);
            layer.children_dirty = true;
        }
        self.layers.get_mut(&id).unwrap().parent = parent;
        Ok(())
    }
    fn layer_order_mode(&mut self, id: usize, mode: bool) {
        if self.layers[&id].absolute_mode == mode {
            return;
        }
        if mode {
            let children = self.layers[&id].children.clone();
            for (i, child) in children.iter().enumerate() {
                self.layers.get_mut(child).unwrap().absolute = i as i32;
            }
        }
        self.layers.get_mut(&id).unwrap().absolute_mode = mode;
    }
    fn layer_order(&mut self, id: usize, index: i32, absolute: bool) -> Result<()> {
        let parent = self.layers[&id]
            .parent
            .context("cannot reorder primary or siblingless layer")?;
        self.layer_order_mode(parent, absolute);
        let children = &self.layers[&parent].children;
        let from = children.iter().position(|c| *c == id).unwrap();
        let to = if absolute {
            let to = children
                .iter()
                .position(|c| self.layers[c].absolute >= index)
                .unwrap_or(children.len());
            if from < to { to - 1 } else { to }
        } else {
            index.clamp(0, children.len() as i32 - 1) as usize
        };
        let layer = self.layers.get_mut(&parent).unwrap();
        layer.children.remove(from);
        layer.children.insert(to, id);
        layer.children_dirty = true;
        if absolute {
            self.layers.get_mut(&id).unwrap().absolute = index;
        }
        Ok(())
    }
    pub(crate) fn layer_call(
        &mut self,
        vm: &mut Vm,
        op: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let id = object(context)?.context("Layer requires a non-null context")?;
        if op == "@initialize" {
            ensure!(self.layers.len() < 10000, "layer count limit exceeded");
            self.layers.entry(id).or_default();
            return Ok(Value::Void);
        }
        if op == "@invalidate" {
            if let Some(layer) = self.layers.remove(&id) {
                if let Some(parent) = layer.parent.and_then(|p| self.layers.get_mut(&p)) {
                    parent.children.retain(|c| *c != id);
                    parent.children_dirty = true;
                }
                for child in layer.children {
                    if let Some(child) = self.layers.get_mut(&child) {
                        child.parent = None;
                    }
                }
                if let Some(window) = self.windows.get_mut(&layer.window)
                    && object(&window.primary_layer)? == Some(id)
                {
                    window.primary_layer = Value::NULL;
                }
                if let Some(font) = layer.font {
                    vm.invalidate(&font, self, budget)?;
                }
            }
            return Ok(Value::Void);
        }
        if !self.layers.contains_key(&id) {
            return Err(unsupported(format!(
                "Layer.{op}: context has no Layer native instance"
            )));
        }
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("Layer.{op}: missing argument {i}"))
        };
        if op == "Layer" {
            if self.layers[&id].constructed {
                return Ok(Value::Void);
            }
            let window = object(arg(0)?)?.context("Layer requires a Window")?;
            ensure!(
                self.windows.get(&window).is_some_and(|w| w.constructed),
                "Layer requires a constructed Window"
            );
            let parent = object(arg(1)?)?;
            let (root, tree_window) = if let Some(p) = parent {
                let p = self.layers.get(&p).context("parent is not a Layer")?;
                ensure!(p.constructed, "parent Layer is not constructed");
                (p.root, p.window)
            } else {
                (id, window)
            };
            let layer = self.layers.get_mut(&id).unwrap();
            layer.constructed = true;
            layer.root = root;
            layer.window = tree_window;
            if parent.is_none() {
                layer.primary = true;
                layer.kind = 1;
                layer.visible = true;
                layer.neutral = [255; 4];
                layer.hit_threshold = 0;
                let w = self.windows.get_mut(&window).unwrap();
                if w.primary_layer == Value::NULL {
                    w.primary_layer = bound(id);
                }
            }
            self.layer_reparent(id, parent)?;
            return Ok(Value::Void);
        }
        ensure!(
            self.layers[&id].constructed,
            "Layer constructor has not run"
        );
        match op {
            "get:absoluteOrderMode" => {
                return Ok(Value::Integer(self.layers[&id].absolute_mode.into()));
            }
            "set:absoluteOrderMode" => {
                self.layer_order_mode(id, arg(0)?.truth()?);
                return Ok(Value::Void);
            }
            "set:order" | "set:absolute" => {
                self.layer_order(id, int(arg(0)?)?, op == "set:absolute")?;
                return Ok(Value::Void);
            }
            "bringToBack" | "bringToFront" => {
                self.layer_order(id, if op == "bringToBack" { 0 } else { i32::MAX }, false)?;
                return Ok(Value::Void);
            }
            "get:parent" => return Ok(self.layers[&id].parent.map_or(Value::NULL, bound)),
            "set:parent" => {
                self.layer_reparent(id, object(arg(0)?)?)?;
                return Ok(Value::Void);
            }
            "get:window" => return Ok(bound(self.layers[&id].window)),
            "get:isPrimary" => return Ok(Value::Integer((self.layers[&id].root == id).into())),
            "get:font" => {
                if let Some(font) = &self.layers[&id].font {
                    return Ok(font.clone());
                }
                let class = vm.globals["Font"].clone();
                let font = vm.construct(&class, &[], self, budget)?;
                self.layers
                    .get_mut(&id)
                    .context("Layer invalidated during Font creation")?
                    .font = Some(font.clone());
                return Ok(font);
            }
            "get:children" => {
                let layer = self.layers.get_mut(&id).unwrap();
                if layer.children_array.is_none() {
                    layer.children_array = Some(vm.new_native_array(Vec::new())?);
                }
                let array = layer.children_array.clone().unwrap();
                if layer.children_dirty {
                    vm.set_member(&array, &Value::string("count"), Value::Integer(0))?;
                    for (i, child) in layer.children.iter().enumerate() {
                        vm.set_member(&array, &Value::Integer(i as i64), bound(*child))?;
                    }
                    layer.children_dirty = false;
                }
                return Ok(array);
            }
            "get:order" | "get:absolute" => {
                let layer = &self.layers[&id];
                let index = layer
                    .parent
                    .map(|p| {
                        self.layers[&p]
                            .children
                            .iter()
                            .position(|c| *c == id)
                            .unwrap_or(0)
                    })
                    .unwrap_or(0);
                return Ok(Value::Integer(
                    if op == "get:absolute"
                        && layer.parent.is_some_and(|p| self.layers[&p].absolute_mode)
                    {
                        layer.absolute as i64
                    } else {
                        index as i64
                    },
                ));
            }
            _ => {}
        }
        let bytes: usize = self
            .layers
            .iter()
            .filter(|(key, _)| **key != id)
            .filter_map(|(_, l)| l.image.as_ref())
            .map(|i| i.rgba.len())
            .sum();
        self.layers
            .get_mut(&id)
            .unwrap()
            .call(op, args, (256usize << 20).saturating_sub(bytes))
    }
}
