//! Native layer surfaces and hierarchy, following krkrz visual/LayerIntf.cpp.
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_assets::media::Image;
use krkrz_tjs::{ObjectRef, Value, Vm, unsupported};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

// Bound the unique buffers retained by all live layers. Portrait caches and
// transitions can overlap; shared images count once until a write detaches them.
// Each individual image is also limited to 16 megapixels.
const MAX_LAYER_IMAGE_BYTES: usize = 512 << 20;

/// Live layer buffers excluding one destination. Shared sources remain charged
/// when the destination detaches or replaces its image.
pub(crate) fn image_available(layers: &BTreeMap<usize, Layer>, destination: usize) -> usize {
    MAX_LAYER_IMAGE_BYTES.saturating_sub(image_bytes(layers, Some(destination)))
}
fn image_bytes(layers: &BTreeMap<usize, Layer>, excluded: Option<usize>) -> usize {
    let mut seen = BTreeSet::new();
    layers
        .iter()
        .filter(|(id, _)| Some(**id) != excluded)
        .map(|(_, layer)| {
            let main = layer
                .image
                .as_ref()
                .filter(|image| seen.insert(Arc::as_ptr(image) as usize))
                .map_or(0, |image| image.rgba.len());
            main + layer.province.as_ref().map_or(0, |p| p.pixels.len())
        })
        .sum()
}

#[derive(Clone)]
struct Province {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

/// A native manager outlives its primary Layer when a detached child survives.
/// It owns focused/modal targets, but stores the primary and tree edges weakly.
#[derive(Default)]
pub(crate) struct Manager {
    focused_layer: Option<usize>,
    modal_layers: Vec<usize>,
    modal_removing: Vec<usize>,
    enabled_notify_depth: usize,
    enabled_snapshot: Vec<(usize, bool)>,
    focus_lock: bool,
}

pub(crate) struct Layer {
    constructed: bool,
    transition: Option<transition::Transition>,
    focusable: bool,
    join_focus_chain: bool,
    focus_work: Option<usize>,
    action_owner: Value,
    primary: bool,
    window: usize,
    root: usize,
    parent: Option<usize>,
    children: Vec<usize>,
    children_array: Option<Value>,
    children_dirty: bool,
    font: Option<Value>,
    name: String,
    hint: Value,
    show_parent_hint: bool,
    ignore_hint_sensing: bool,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    image_left: i32,
    image_top: i32,
    pub(crate) image: Option<Arc<Image>>,
    province: Option<Province>,
    clip: [i32; 4],
    kind: i32,
    face: i32,
    hold_alpha: bool,
    visible: bool,
    enabled: bool,
    opacity: i32,
    cursor: i32,
    cursor_x_work: i32,
    image_modified: bool,
    call_on_paint: bool,
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
            transition: None,
            focusable: false,
            join_focus_chain: true,
            focus_work: None,
            action_owner: Value::NULL,
            primary: false,
            window: 0,
            root: 0,
            parent: None,
            children: Vec::new(),
            children_array: None,
            children_dirty: true,
            font: None,
            name: String::new(),
            hint: Value::string(""),
            show_parent_hint: true,
            ignore_hint_sensing: false,
            left: 0,
            top: 0,
            width: 32,
            height: 32,
            image_left: 0,
            image_top: 0,
            image: Some(Arc::new(Image {
                width: 32,
                height: 32,
                rgba: [255, 255, 255, 0].repeat(32 * 32),
            })),
            province: None,
            clip: [0, 0, 32, 32],
            kind: 2,
            face: 128,
            hold_alpha: false,
            visible: false,
            enabled: true,
            opacity: 255,
            cursor: 0,
            cursor_x_work: 0,
            image_modified: true,
            call_on_paint: false,
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
    for key in [
        "children",
        "window",
        "isPrimary",
        "font",
        "focused",
        "nodeFocusable",
        "nodeEnabled",
        "nextFocusable",
        "prevFocusable",
    ] {
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
fn rgb(color: u32) -> [u8; 3] {
    if color & 0x80000000 != 0 {
        // Portable light palette for Win32 GetSysColor indices. Scripts use
        // these identifiers for dialog text, button faces and beveled borders.
        // Store RGB values here, not COLORREF; invalid/reserved indices are black.
        const SYSTEM: [u32; 31] = [
            0xc8c8c8, // ScrollBar
            0x000000, // Background
            0x000080, // ActiveCaption
            0x808080, // InactiveCaption
            0xf0f0f0, // Menu
            0xffffff, // Window
            0x646464, // WindowFrame
            0x000000, // MenuText
            0x000000, // WindowText
            0xffffff, // CaptionText
            0xb4b4b4, // ActiveBorder
            0xf4f7fc, // InactiveBorder
            0x808080, // AppWorkSpace
            0x0078d7, // Highlight
            0xffffff, // HighlightText
            0xf0f0f0, // BtnFace / 3DFace
            0xa0a0a0, // BtnShadow / 3DShadow
            0x6d6d6d, // GrayText
            0x000000, // BtnText
            0xffffff, // InactiveCaptionText
            0xffffff, // BtnHighlight / 3DHighlight
            0x696969, // 3DDkShadow
            0xe3e3e3, // 3DLight
            0x000000, // InfoText
            0xffffe1, // InfoBk
            0x000000, // Reserved
            0x0066cc, // HotLight
            0x1084d0, // GradientActiveCaption
            0xb5b5b5, // GradientInactiveCaption
            0x0078d7, // MenuHighlight
            0xf0f0f0, // MenuBar
        ];
        let color = SYSTEM.get((color & 0xff) as usize).copied().unwrap_or(0);
        return [(color >> 16) as u8, (color >> 8) as u8, color as u8];
    }
    if color & 0xff000000 != 0 {
        [color as u8, (color >> 8) as u8, (color >> 16) as u8]
    } else {
        [(color >> 16) as u8, (color >> 8) as u8, color as u8]
    }
}
impl Layer {
    fn bitmap(&self) -> Result<&Image> {
        self.image.as_deref().context("layer has no image")
    }
    // Check before detaching; a failed write leaves the shared pixels intact.
    fn prepare_image_write(&mut self, available: usize) -> Result<()> {
        let bytes =
            self.bitmap()?.rgba.len() + self.province.as_ref().map_or(0, |p| p.pixels.len());
        ensure!(
            bytes <= available,
            "session layer image memory limit exceeded"
        );
        Arc::make_mut(self.image.as_mut().unwrap());
        Ok(())
    }
    fn allocate_province(&mut self, available: usize) -> Result<()> {
        if self.province.is_none() {
            let (w, h) = self
                .image
                .as_ref()
                .map(|i| (i.width as i32, i.height as i32))
                .unwrap_or((self.width, self.height));
            let n = pixel_count(w, h)?;
            ensure!(
                n + self.image.as_ref().map_or(0, |i| i.rgba.len()) <= available,
                "session layer image memory limit exceeded"
            );
            self.province = Some(Province {
                width: w as usize,
                height: h as usize,
                pixels: vec![0; n],
            });
            self.image_modified = true;
        }
        Ok(())
    }
    fn resize_province(&mut self, w: usize, h: usize) {
        if let Some(old) = self.province.take() {
            let mut pixels = vec![0; w * h];
            for y in 0..h.min(old.height) {
                let len = w.min(old.width);
                pixels[y * w..y * w + len]
                    .copy_from_slice(&old.pixels[y * old.width..y * old.width + len]);
            }
            self.province = Some(Province {
                width: w,
                height: h,
                pixels,
            });
        }
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
                n * if self.province.is_some() { 5 } else { 4 } <= available,
                "session layer image memory limit exceeded"
            );
            self.image = Some(Arc::new(Image {
                width: self.width as u32,
                height: self.height as u32,
                rgba: self.neutral.repeat(n),
            }));
            self.image_left = 0;
            self.image_top = 0;
            self.resize_province(self.width as usize, self.height as usize);
        }
        self.image_modified = true;
        self.reset_clip()
    }
    fn resize_image(&mut self, w: i32, h: i32, available: usize) -> Result<()> {
        let n = pixel_count(w, h)?;
        ensure!(
            n * if self.province.is_some() { 5 } else { 4 } <= available,
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
        self.image = Some(Arc::new(Image {
            width: w as u32,
            height: h as u32,
            rgba,
        }));
        self.resize_province(w as usize, h as usize);
        self.image_modified = true;
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
    fn color_rect(&mut self, args: &[Value], available: usize) -> Result<Value> {
        ensure!(args.len() >= 5, "Layer.colorRect: missing arguments");
        let n = |i| int(&args[i]);
        let (x, y, w, h) = (n(0)?, n(1)?, n(2)?, n(3)?);
        let left = x.max(self.clip[0]);
        let top = y.max(self.clip[1]);
        let right = x.wrapping_add(w).min(self.clip[2]);
        let bottom = y.wrapping_add(h).min(self.clip[3]);
        if right <= left || bottom <= top {
            return Ok(Value::Void);
        }
        let face = self.draw_face();
        if matches!(face, 2 | 3) {
            return self.call("fillRect", &args[..5], available);
        }
        ensure!(matches!(face, 0 | 1 | 4), "invalid colorRect draw face");
        let opacity = args
            .get(5)
            .filter(|v| !matches!(v, Value::Void))
            .map(int)
            .transpose()?
            .unwrap_or(255);
        self.bitmap()?;
        ensure!(
            face != 4 || opacity >= 0,
            "negative opacity is not supported on additive alpha face"
        );
        if opacity == 0 {
            return Ok(Value::Void);
        }
        let removing = face == 0 && opacity < 0;
        let strength = if removing {
            (-(opacity as i64)).min(255) as i32
        } else {
            opacity.clamp(0, 255)
        };
        let color = if removing {
            [0; 3]
        } else {
            rgb(args[4].integer()? as u32)
        };
        self.prepare_image_write(available)?;
        let image = Arc::make_mut(self.image.as_mut().unwrap());
        for y in top.max(0)..bottom.min(image.height as i32) {
            for x in left.max(0)..right.min(image.width as i32) {
                let i = (y as usize * image.width as usize + x as usize) * 4;
                let pixel = &mut image.rgba[i..i + 4];
                if removing {
                    pixel[3] = ((pixel[3] as i32 * (255 - strength)) >> 8) as u8;
                } else if strength == 255 {
                    pixel[..3].copy_from_slice(&color);
                    if face != 1 {
                        pixel[3] = 255;
                    }
                } else if face == 0 {
                    let weight = straight_alpha_weight(pixel[3], strength as u8);
                    for k in 0..3 {
                        pixel[k] = (pixel[k] as i32
                            + (((color[k] as i32 - pixel[k] as i32) * weight) >> 8))
                            as u8;
                    }
                    pixel[3] = (255 - ((255 - pixel[3] as i32) * (255 - strength) / 255)) as u8;
                } else if face == 1 {
                    for k in 0..3 {
                        pixel[k] = ((pixel[k] as i32 * (255 - strength)
                            + color[k] as i32 * strength)
                            >> 8) as u8;
                    }
                } else {
                    // Kirikiri's SSE2 path adjusts 128..254 upward by one.
                    let alpha = strength + (strength >> 7);
                    for k in 0..3 {
                        pixel[k] = (pixel[k] as i32 - ((pixel[k] as i32 * alpha) >> 8)
                            + ((color[k] as i32 * alpha) >> 8))
                            .min(255) as u8;
                    }
                    pixel[3] =
                        (pixel[3] as i32 - ((pixel[3] as i32 * alpha) >> 8) + alpha).min(255) as u8;
                }
            }
        }
        self.image_modified = true;
        Ok(Value::Void)
    }
    fn call(&mut self, op: &str, args: &[Value], available: usize) -> Result<Value> {
        if op == "colorRect" {
            return self.color_rect(args, available);
        }
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("Layer.{op}: missing argument {i}"))
        };
        let n = |i: usize| -> Result<i32> { int(arg(i)?) };
        if let Some(key) = op.strip_prefix("get:") {
            let v = match key {
                "name" => return Ok(Value::string(&self.name)),
                "hint" => return Ok(self.hint.clone()),
                "neutralColor" => {
                    let [r, g, b, a] = self.neutral;
                    return Ok(Value::Integer(u32::from_be_bytes([a, r, g, b]).into()));
                }
                "showParentHint" => self.show_parent_hint.into(),
                "ignoreHintSensing" => self.ignore_hint_sensing.into(),
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
                "cursor" => self.cursor,
                "imageModified" => self.image_modified.into(),
                "callOnPaint" => self.call_on_paint.into(),
                "hitType" => self.hit_type,
                "hitThreshold" => self.hit_threshold,
                _ => return Err(unsupported(format!("unsupported Layer operation: {op}"))),
            };
            return Ok(Value::Integer(v.into()));
        }
        match op {
            "finalize" => {}
            "set:name" => self.name = arg(0)?.text(),
            "set:neutralColor" => {
                let [a, r, g, b] = (arg(0)?.integer()? as u32).to_be_bytes();
                self.neutral = [r, g, b, a];
            }
            "set:showParentHint" => self.show_parent_hint = arg(0)?.truth()?,
            "set:ignoreHintSensing" => self.ignore_hint_sensing = arg(0)?.truth()?,
            "set:hint" => {
                self.hint = arg(0)?.unary("string")?;
                self.show_parent_hint = false;
                self.ignore_hint_sensing = false;
            }
            "set:visible" => self.visible = arg(0)?.truth()?,
            "set:enabled" => self.enabled = arg(0)?.truth()?,
            "set:opacity" => self.opacity = n(0)?.clamp(0, 255),
            "set:imageModified" => self.image_modified = arg(0)?.truth()?,
            "set:callOnPaint" => self.call_on_paint = arg(0)?.truth()?,
            "update" => {
                if !args.is_empty() {
                    ensure!(
                        args.len() >= 4,
                        "Layer.update requires 0 or at least 4 arguments"
                    );
                    for value in &args[..4] {
                        int(value)?;
                    }
                }
                self.call_on_paint = true;
            }
            "set:cursor" => {
                if matches!(arg(0)?, Value::String(_)) {
                    return Err(unsupported("Layer cursor image loading is not implemented"));
                }
                self.cursor = n(0)?;
            }
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
                    self.province = None;
                    self.image_modified = true;
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
                        self.province = None;
                        self.image_modified = true;
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
            "getProvincePixel" => {
                let (x, y) = (n(0)?, n(1)?);
                return Ok(Value::Integer(
                    self.province
                        .as_ref()
                        .filter(|p| {
                            x >= 0 && y >= 0 && (x as usize) < p.width && (y as usize) < p.height
                        })
                        .map_or(0, |p| p.pixels[y as usize * p.width + x as usize] as i64),
                ));
            }
            "setProvincePixel" => {
                let (x, y, color) = (n(0)?, n(1)?, n(2)? as u8);
                self.allocate_province(available)?;
                if self.inside_clip(x, y) {
                    let p = self.province.as_mut().unwrap();
                    ensure!(
                        x >= 0 && y >= 0 && (x as usize) < p.width && (y as usize) < p.height,
                        "province pixel is outside image"
                    );
                    p.pixels[y as usize * p.width + x as usize] = color;
                    self.image_modified = true;
                }
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
                    self.prepare_image_write(available)?;
                    let p = &mut Arc::make_mut(self.image.as_mut().unwrap()).rgba[index..index + 4];
                    if op == "setMaskPixel" {
                        p[3] = color as u8;
                    } else {
                        p[..3].copy_from_slice(&rgb(color));
                    }
                    self.image_modified = true;
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
                if face == 3 {
                    let color = color as u8;
                    if color != 0 {
                        self.allocate_province(available)?;
                    }
                    if let Some(p) = &mut self.province {
                        if color == 0
                            && left == 0
                            && top == 0
                            && right == p.width as i32
                            && bottom == p.height as i32
                        {
                            self.province = None;
                            self.image_modified = true;
                        } else {
                            for y in top.max(0)..bottom.min(p.height as i32) {
                                for x in left.max(0)..right.min(p.width as i32) {
                                    p.pixels[y as usize * p.width + x as usize] = color;
                                    self.image_modified = true;
                                    self.image_modified = true;
                                }
                            }
                        }
                    }
                    return Ok(Value::Void);
                }
                if !matches!(face, 0 | 1 | 2 | 4) {
                    return Err(unsupported(format!("Layer.fillRect draw face {face}")));
                }
                let hold = face == 1 && self.hold_alpha;
                let c = if hold {
                    rgb(color)
                } else {
                    rgb(color & 0xffffff)
                };
                self.prepare_image_write(available)?;
                let image = Arc::make_mut(self.image.as_mut().unwrap());
                self.image_modified = true;
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
    fn layer_validate_parent(&self, id: usize, parent: Option<usize>) -> Result<()> {
        let root = self
            .layers
            .get(&id)
            .context("Layer invalidated before reparenting")?
            .root;
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
        Ok(())
    }
    fn layer_reparent(&mut self, id: usize, parent: Option<usize>) -> Result<()> {
        self.layer_validate_parent(id, parent)?;
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
            self.layer_invalidate_transition(vm, id, budget)?;
            self.layer_remove_modes(vm, id, true, budget)?;
            self.layer_forget_focus(id);
            if let Some(layer) = self.layers.get(&id)
                && let Some(window) = self.windows.get(&layer.window)
            {
                let window_id = layer.window;
                let capture = window
                    .input
                    .capture
                    .is_some_and(|c| self.layer_descendant(c, id));
                let hover = window
                    .input
                    .hover
                    .is_some_and(|c| self.layer_descendant(c, id));
                let window = self.windows.get_mut(&window_id).unwrap();
                if capture {
                    window.input.capture = None;
                }
                if hover {
                    window.input.hover = None;
                }
            }
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
                if !self
                    .layers
                    .values()
                    .any(|l| l.constructed && l.root == layer.root)
                {
                    self.layer_managers.remove(&layer.root);
                }
            }
            return Ok(Value::Void);
        }
        if !self.layers.contains_key(&id) {
            return Err(unsupported(format!(
                "Layer.{op}: context has no Layer native instance"
            )));
        }
        if op == "get:nodeVisible" {
            // Visibility follows the layer ancestors, independently of opacity,
            // enabled state, clipping, or the native window's visibility.
            let mut current = Some(id);
            while let Some(id) = current {
                *budget = budget
                    .checked_sub(1)
                    .ok_or_else(|| unsupported("Layer visibility execution budget exceeded"))?;
                let layer = &self.layers[&id];
                if !layer.visible {
                    return Ok(Value::Integer(0));
                }
                current = layer.parent;
            }
            return Ok(Value::Integer(1));
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
            layer.action_owner = arg(0)?.clone();
            layer.constructed = true;
            layer.root = root;
            layer.window = tree_window;
            if parent.is_none() {
                self.layer_managers.insert(id, Manager::default());
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
        if let Some(value) = self.layer_focus_call(vm, id, op, args, budget)? {
            return Ok(value);
        }
        match op {
            "getLayerAt" => return self.layer_get_at(vm, id, args, budget),
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
                let parent = object(arg(0)?)?;
                self.layer_validate_parent(id, parent)?;
                self.layer_blur_tree(vm, id, budget)?;
                self.layer_reparent(id, parent)?;
                return Ok(Value::Void);
            }
            "get:window" => {
                let layer = &self.layers[&id];
                return Ok(if layer.constructed {
                    bound(layer.window)
                } else {
                    Value::NULL
                });
            }
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
                    vm.clear_array(&array)?;
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
        if matches!(op, "beginTransition" | "stopTransition") {
            return self.layer_transition_call(vm, id, op, args, budget);
        }
        if op == "piledCopy" {
            return self.layer_piled_copy(vm, id, args, budget);
        }
        if op == "shrinkCopy" {
            return self.layer_shrink_copy(id, args, budget);
        }
        if op == "doBoxBlur" {
            return self.layer_box_blur(id, args, budget);
        }
        if op == "affineCopy" {
            return self.layer_affine_copy(id, args, budget);
        }
        if op == "stretchCopy" {
            return self.layer_stretch_copy(id, args, budget);
        }
        if op == "saveLayerImage" {
            return self.layer_save_image(id, args, budget);
        }
        if op == "loadImages" {
            return self.layer_load_images(vm, id, args, budget);
        }
        if op == "assignImages" {
            return self.layer_assign_images(id, arg(0)?, budget);
        }
        if op == "adjustGamma" {
            return self.layer_gamma(id, args, budget);
        }
        if op == "doGrayScale" {
            return self.layer_grayscale(id, budget);
        }
        if op == "drawText" {
            return self.layer_draw_text(id, args, budget);
        }
        if op == "clipAlphaRect" {
            return self.layer_clip_alpha(id, args, budget);
        }
        if op == "fillAlpha" {
            return self.layer_fill_alpha(id, budget);
        }
        if op == "copyAlphaToProvince" {
            return self.layer_copy_alpha_to_province(id, args, budget);
        }
        if matches!(op, "copyRect" | "operateRect") {
            return self.layer_blit(id, op, args, budget);
        }
        if op == "loadProvinceImage" {
            return self.layer_load_province(id, args, budget);
        }
        // Getters and geometry-only setters do not allocate pixels. Avoid
        // walking every shared buffer for each property read during rendering.
        let available = if matches!(
            op,
            "colorRect"
                | "fillRect"
                | "setMainPixel"
                | "setMaskPixel"
                | "setProvincePixel"
                | "set:hasImage"
                | "set:type"
                | "set:width"
                | "set:height"
                | "setSize"
                | "setImageSize"
                | "set:imageWidth"
                | "set:imageHeight"
                | "setSizeToImageSize"
        ) || op == "setPos" && args.len() >= 4
        {
            image_available(&self.layers, id)
        } else {
            0
        };
        self.layers.get_mut(&id).unwrap().call(op, args, available)
    }
}

mod affine;
mod blit;
mod blur;
mod clip_alpha;
mod copy_alpha_to_province;
mod draw;
pub use draw::WindowFrameState;
mod fill_alpha;
mod focus;
mod gamma;
mod grayscale;
mod images;
pub(crate) mod input;
mod save;
mod shrink;
mod stretch;
mod text;
mod transition;

// Match TVPOpacityOnOpacityTable's single-precision construction rather than
// replacing its 8-bit interpolation with a different compositing formula.
fn straight_alpha_weight(destination: u8, source: u8) -> i32 {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Box<[u8]>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = vec![0; 65536];
        for b in 0..256 {
            for a in 0..256 {
                table[b * 256 + a] = if a == 0 {
                    255
                } else {
                    let at = (a as f64 / 255.0) as f32;
                    let bt = (b as f64 / 255.0) as f32;
                    let c = bt / at;
                    let c = c / ((1.0 - bt as f64 + c as f64) as f32);
                    ((c * 255.0) as i32).min(255) as u8
                };
            }
        }
        table.into_boxed_slice()
    });
    table[source as usize * 256 + destination as usize] as i32
}

impl crate::Session {
    /// Run the layer tree's pending onPaint events before a host reads pixels.
    /// Callbacks clear their flag before running; calling update from onPaint
    /// requests the next completion. Invisible children also receive onPaint.
    pub fn prepare_window_paint(&mut self, window: &Value) -> Result<()> {
        let window = object(window)?.context("paint requires a Window")?;
        let window = self
            .services
            .windows
            .get(&window)
            .filter(|w| w.constructed)
            .context("paint requires a constructed Window")?;
        if let Some(root) = object(&window.primary_layer)? {
            self.services
                .layer_before_completion(&mut self.vm, root, &mut self.budget)?;
        }
        Ok(())
    }
}

impl Layer {
    pub(crate) fn gc_trace(&self, out: &mut Vec<Value>) {
        out.extend([self.action_owner.clone(), self.hint.clone()]);
        // LayerIntf keeps the tree and FocusWork as native pointers, without
        // AddRef. Only reading children creates an owning script Array.
        out.extend(self.children_array.iter().cloned());
        out.extend(self.font.iter().cloned());
        if let Some(transition) = &self.transition {
            transition.gc_trace(out);
        }
    }
}

impl Manager {
    fn gc_trace(&self, out: &mut Vec<Value>) {
        out.extend(self.focused_layer.map(Value::object));
        out.extend(self.modal_layers.iter().copied().map(Value::object));
    }
}

impl Services {
    pub(crate) fn layer_manager_gc_trace(&self, window: usize, out: &mut Vec<Value>) {
        // Windows retain managers, whose focus/modal references own their
        // targets. The manager's Primary pointer does not own the root Layer.
        for layer in self
            .layers
            .values()
            .filter(|l| l.primary && l.window == window)
        {
            if let Some(manager) = self.layer_managers.get(&layer.root) {
                manager.gc_trace(out);
            }
        }
    }

    pub(crate) fn layer_shared_manager_gc_trace(&self, id: usize, out: &mut Vec<Value>) {
        if let Some(manager) = self.layer_managers.get(&self.layers[&id].root) {
            manager.gc_trace(out);
        }
    }
}

impl Layer {
    pub(crate) fn set_movie_image(&mut self, image: &Image, available: usize) -> Result<()> {
        ensure!(
            image.rgba.len() + self.province.as_ref().map_or(0, |p| p.pixels.len()) <= available,
            "session layer image memory limit exceeded"
        );
        self.image_size(image.width as i32, image.height as i32, available)?;
        // VideoOverlay updates both the bitmap and the visible layer bounds.
        // KAG can attach a small placeholder layer before opening the movie.
        self.size(image.width as i32, image.height as i32, available)?;
        self.image = Some(Arc::new(image.clone()));
        self.image_modified = true;
        Ok(())
    }
}

impl Layer {
    pub(crate) fn present_alpha_movie(
        &mut self,
        image: &Image,
        left: i32,
        top: i32,
        available: usize,
    ) -> Result<()> {
        ensure!(
            !self.primary || (left == 0 && top == 0),
            "cannot move primary layer"
        );
        self.set_movie_image(image, available)?;
        self.left = left;
        self.top = top;
        Ok(())
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn assigned_images_share_then_detach_with_atomic_quota_checks() {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        let mut session =
            crate::Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
        session.services.layers.insert(1, Layer::default());
        session.services.layers.insert(2, Layer::default());
        session
            .services
            .layer_assign_images(2, &Value::object(1), &mut 100_000)
            .unwrap();
        let source = session.services.layers[&1].image.clone().unwrap();
        assert!(Arc::ptr_eq(
            &source,
            session.services.layers[&2].image.as_ref().unwrap()
        ));
        assert_eq!(image_bytes(&session.services.layers, None), 4096);
        assert_eq!(
            image_available(&session.services.layers, 2),
            MAX_LAYER_IMAGE_BYTES - 4096
        );
        let destination = session.services.layers.get_mut(&2).unwrap();
        assert!(
            destination
                .call(
                    "setMaskPixel",
                    &[Value::Integer(0), Value::Integer(0), Value::Integer(255)],
                    4095
                )
                .is_err()
        );
        assert!(Arc::ptr_eq(&source, destination.image.as_ref().unwrap()));
        destination
            .call(
                "setMaskPixel",
                &[Value::Integer(0), Value::Integer(0), Value::Integer(255)],
                4096,
            )
            .unwrap();
        assert_eq!(source.rgba[3], 0);
        assert_eq!(destination.bitmap().unwrap().rgba[3], 255);
        assert!(!Arc::ptr_eq(&source, destination.image.as_ref().unwrap()));
        assert_eq!(image_bytes(&session.services.layers, None), 8192);
    }

    #[test]
    fn shared_clip_alpha_keeps_forward_self_overlap() {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        let mut session =
            crate::Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
        let mut layer = Layer::default();
        layer.resize_image(4, 1, 4096).unwrap();
        layer
            .call(
                "fillRect",
                &[
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(4),
                    Value::Integer(1),
                    Value::Integer(0x80000000),
                ],
                4096,
            )
            .unwrap();
        session.services.layers.insert(1, layer);
        session.services.layers.insert(2, Layer::default());
        session
            .services
            .layer_assign_images(2, &Value::object(1), &mut 100_000)
            .unwrap();
        session
            .services
            .layer_clip_alpha(
                1,
                &[
                    Value::Integer(1),
                    Value::Integer(0),
                    Value::object(1),
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(3),
                    Value::Integer(1),
                ],
                &mut 100_000,
            )
            .unwrap();
        let alpha = |id| {
            session.services.layers[&id]
                .bitmap()
                .unwrap()
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[3])
                .collect::<Vec<_>>()
        };
        assert_eq!(alpha(1), [128, 64, 32, 16]);
        assert_eq!(alpha(2), [128; 4]);
    }

    #[test]
    fn rejected_image_growth_keeps_pixels_dimensions_and_clip() {
        let mut layer = Layer {
            clip: [1, 2, 3, 4],
            ..Layer::default()
        };
        let before = layer.image.clone().unwrap();
        assert!(layer.resize_image(64, 64, 4096).is_err());
        let after = layer.image.as_ref().unwrap();
        assert_eq!((after.width, after.height), (before.width, before.height));
        assert_eq!(after.rgba, before.rgba);
        assert_eq!(layer.clip, [1, 2, 3, 4]);
        assert!(
            layer
                .resize_image(8192, 8192, MAX_LAYER_IMAGE_BYTES)
                .is_err()
        );
        assert_eq!(layer.image.as_ref().unwrap().rgba, before.rgba);
        assert!(layer.allocate_province(4096).is_err());
        assert!(layer.province.is_none());
    }
}
