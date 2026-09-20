//! CPU layer completion. Layer face/clip/holdAlpha affect drawing into a
//! surface; display type/opacity/parent bounds affect tree composition.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Rect {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}
impl Rect {
    fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let w = (self.x + self.w).min(other.x + other.w) - x;
        let h = (self.y + self.h).min(other.y + other.h) - y;
        (w > 0 && h > 0).then_some(Self { x, y, w, h })
    }
}
fn charge(budget: &mut u64, count: u64) -> Result<()> {
    *budget = budget
        .checked_sub(count)
        .ok_or_else(|| unsupported("layer composition execution budget exceeded"))?;
    Ok(())
}
impl Services {
    fn layer_self_image(
        &self,
        id: usize,
        rect: Rect,
        budget: &mut u64,
        depth: usize,
        live_bytes: usize,
    ) -> Result<Image> {
        ensure!(depth < 128, "layer composition nesting limit exceeded");
        let n = pixel_count(rect.w as i32, rect.h as i32)?;
        ensure!(
            live_bytes + n * 4 <= 256 << 20,
            "layer composition memory limit exceeded"
        );
        charge(budget, n as u64)?;
        let layer = self
            .layers
            .get(&id)
            .context("composition source is not a Layer")?;
        let mut result = Image {
            width: rect.w as u32,
            height: rect.h as u32,
            rgba: layer.neutral.repeat(n),
        };
        if let Some(image) = &layer.image {
            for y in 0..rect.h {
                let sy = rect.y + y - layer.image_top as i64;
                if sy < 0 || sy >= image.height as i64 {
                    continue;
                }
                let left = 0.max(layer.image_left as i64 - rect.x);
                let right = rect
                    .w
                    .min(layer.image_left as i64 + image.width as i64 - rect.x);
                if right <= left {
                    continue;
                }
                let src = (sy * image.width as i64 + rect.x + left - layer.image_left as i64)
                    as usize
                    * 4;
                let dst = (y * rect.w + left) as usize * 4;
                let len = (right - left) as usize * 4;
                result.rgba[dst..dst + len].copy_from_slice(&image.rgba[src..src + len]);
            }
        }
        Ok(result)
    }

    pub(super) fn layer_complete(
        &self,
        id: usize,
        rect: Rect,
        budget: &mut u64,
        depth: usize,
        live_bytes: usize,
    ) -> Result<Image> {
        let mut result = self.layer_self_image(id, rect, budget, depth, live_bytes)?;
        let layer = &self.layers[&id];
        let bytes = live_bytes + result.rgba.len();
        if let Some(t) = &layer.transition
            && !t.with_children
        {
            let src_layer = self
                .layers
                .get(&t.source)
                .context("transition source was invalidated")?;
            let source = self.layer_self_image(
                t.source,
                Rect {
                    x: rect.x - layer.image_left as i64 + src_layer.image_left as i64,
                    y: rect.y - layer.image_top as i64 + src_layer.image_top as i64,
                    ..rect
                },
                budget,
                depth + 1,
                bytes,
            )?;
            super::transition::crossfade(&mut result, &source, layer.kind, t.phase);
        }
        self.layer_draw_children(id, &mut result, rect, layer.kind, budget, depth, bytes)?;
        if let Some(t) = &layer.transition
            && t.with_children
        {
            let source = if t.source == id {
                result.clone()
            } else {
                self.layer_complete(t.source, rect, budget, depth + 1, bytes)?
            };
            super::transition::crossfade(&mut result, &source, layer.kind, t.phase);
        }
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn layer_draw_children(
        &self,
        id: usize,
        target: &mut Image,
        rect: Rect,
        target_kind: i32,
        budget: &mut u64,
        depth: usize,
        live_bytes: usize,
    ) -> Result<()> {
        ensure!(depth < 128, "layer composition nesting limit exceeded");
        for child in &self.layers[&id].children {
            charge(budget, 1)?;
            let layer = &self.layers[child];
            if !layer.visible || layer.opacity == 0 {
                continue;
            }
            let Some(clipped) = rect.intersection(Rect {
                x: layer.left as i64,
                y: layer.top as i64,
                w: layer.width as i64,
                h: layer.height as i64,
            }) else {
                continue;
            };
            let local = Rect {
                x: clipped.x - layer.left as i64,
                y: clipped.y - layer.top as i64,
                ..clipped
            };
            if matches!(layer.kind, 0 | 6 | 7) {
                // Binders forward child pixels straight to their own target.
                // Their bounds clip children, but nonzero opacity is ignored.
                let bytes = piece_bytes(clipped);
                ensure!(
                    live_bytes + bytes <= 256 << 20,
                    "layer composition memory limit exceeded"
                );
                charge(budget, bytes as u64 / 4)?;
                let mut piece = Image {
                    width: clipped.w as u32,
                    height: clipped.h as u32,
                    rgba: vec![0; (clipped.w * clipped.h * 4) as usize],
                };
                copy_region(
                    target,
                    &mut piece,
                    clipped.x - rect.x,
                    clipped.y - rect.y,
                    false,
                );
                self.layer_draw_children(
                    *child,
                    &mut piece,
                    local,
                    target_kind,
                    budget,
                    depth + 1,
                    live_bytes + piece_bytes(clipped),
                )?;
                copy_region(
                    target,
                    &mut piece,
                    clipped.x - rect.x,
                    clipped.y - rect.y,
                    true,
                );
                continue;
            }
            if !matches!(layer.kind, 1 | 2 | 12) {
                return Err(unsupported(format!(
                    "layer composition blend type {}",
                    layer.kind
                )));
            }
            let source = self.layer_complete(*child, local, budget, depth + 1, live_bytes)?;
            let face = match target_kind {
                2 | 13 => 0,
                12 => 4,
                _ => 1,
            };
            let x = (clipped.x - rect.x) as usize;
            let y = (clipped.y - rect.y) as usize;
            for row in 0..source.height as usize {
                let start = ((y + row) * target.width as usize + x) * 4;
                let len = source.width as usize * 4;
                super::blit::blend_row(
                    &mut target.rgba[start..start + len],
                    &source.rgba[row * len..(row + 1) * len],
                    x,
                    face,
                    layer.kind,
                    layer.opacity,
                    false,
                );
            }
        }
        Ok(())
    }

    pub(super) fn layer_piled_copy(
        &mut self,
        vm: &mut Vm,
        id: usize,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(args.len() >= 7, "Layer.piledCopy: missing arguments");
        let source = object(&args[2])?.context("piledCopy source is null")?;
        let src = self
            .layers
            .get(&source)
            .context("piledCopy source is not a Layer")?;
        src.bitmap()?;
        let dst = &self.layers[&id];
        let image = dst.bitmap()?;
        let (dx, dy, sx, sy, w, h) = (
            int(&args[0])? as i64,
            int(&args[1])? as i64,
            int(&args[3])? as i64,
            int(&args[4])? as i64,
            int(&args[5])? as i64,
            int(&args[6])? as i64,
        );
        let left = 0.max(-sx).max(-dx).max(dst.clip[0] as i64 - dx);
        let top = 0.max(-sy).max(-dy).max(dst.clip[1] as i64 - dy);
        let right = w
            .min(src.width as i64 - sx)
            .min(image.width as i64 - dx)
            .min(dst.clip[2] as i64 - dx);
        let bottom = h
            .min(src.height as i64 - sy)
            .min(image.height as i64 - dy)
            .min(dst.clip[3] as i64 - dy);
        if right <= left || bottom <= top {
            return Ok(Value::Void);
        }
        let rect = Rect {
            x: sx + left,
            y: sy + top,
            w: right - left,
            h: bottom - top,
        };
        // Complete returns the MainImage directly for an unoffset leaf. It
        // does not run paint events or advance a transition on that path.
        let direct = src.image_left == 0
            && src.image_top == 0
            && src.bitmap()?.width == src.width as u32
            && src.bitmap()?.height == src.height as u32
            && !src
                .children
                .iter()
                .any(|c| self.layers[c].visible && self.layers[c].opacity != 0);
        let mut pixels = if direct {
            self.layer_self_image(source, rect, budget, 0, 0)?
        } else {
            self.layer_before_completion(vm, source, budget)?;
            let pixels = self.layer_complete(source, rect, budget, 0, 0)?;
            self.layer_after_completion(vm, source, budget)?;
            pixels
        };
        let dst = self
            .layers
            .get_mut(&id)
            .context("piledCopy destination was invalidated")?;
        copy_region(
            dst.image
                .as_mut()
                .context("piledCopy destination has no image")?,
            &mut pixels,
            dx + left,
            dy + top,
            true,
        );
        dst.image_modified = true;
        Ok(Value::Void)
    }
}
fn piece_bytes(rect: Rect) -> usize {
    (rect.w * rect.h * 4) as usize
}
fn copy_region(target: &mut Image, piece: &mut Image, x: i64, y: i64, write: bool) {
    let left = 0.max(-x);
    let top = 0.max(-y);
    let right = (piece.width as i64).min(target.width as i64 - x);
    let bottom = (piece.height as i64).min(target.height as i64 - y);
    if right <= left || bottom <= top {
        return;
    }
    let len = (right - left) as usize * 4;
    for row in top..bottom {
        let start = ((y + row) * target.width as i64 + x + left) as usize * 4;
        let piece_start = (row * piece.width as i64 + left) as usize * 4;
        let large = &mut target.rgba[start..start + len];
        let small = &mut piece.rgba[piece_start..piece_start + len];
        if write {
            large.copy_from_slice(small);
        } else {
            small.copy_from_slice(large);
        }
    }
}

impl crate::Session {
    /// Complete the native Layer tree into an RGBA surface shared by replay and
    /// native presentation. The root's own opacity and visibility are ignored.
    pub fn capture_window(&mut self, window: &Value) -> Result<Image> {
        self.prepare_window_paint(window)?;
        self.capture_window_prepared(window)
    }
    pub(super) fn capture_window_prepared(&mut self, window: &Value) -> Result<Image> {
        let window = object(window)?.context("capture requires a Window")?;
        let window = self
            .services
            .windows
            .get(&window)
            .context("capture requires a Window")?;
        let root = object(&window.primary_layer)?.context("window has no primary Layer")?;
        let layer = &self.services.layers[&root];
        let image = self.services.layer_complete(
            root,
            Rect {
                x: 0,
                y: 0,
                w: layer.width as i64,
                h: layer.height as i64,
            },
            &mut self.budget,
            0,
            0,
        )?;
        self.services
            .layer_after_completion(&mut self.vm, root, &mut self.budget)?;
        Ok(image)
    }
}
