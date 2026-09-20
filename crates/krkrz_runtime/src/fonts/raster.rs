//! Hinted, session-private glyphs. Layout and 65-level coverage follow Kirikiri's
//! GDI rasterizer; FreeType supplies portable outline loading and rasterization.
use super::*;
use crate::font::Font as FontStyle;
use freetype::{Library, Matrix, RenderMode, face::LoadFlag};

#[derive(Clone)]
pub(crate) struct TextGlyph {
    pub left: i32,
    pub top: i32,
    pub width: usize,
    pub height: usize,
    pub advance: [i32; 2],
    /// Coverage in 0..=64, including monochrome glyphs.
    pub coverage: Vec<u8>,
}

pub(crate) struct TextRasterizer {
    face: freetype::Face<Arc<[u8]>>,
    style: FontStyle,
    ascent: i32,
    lines: Vec<(i32, i32)>,
}

impl FontBook {
    pub(crate) fn text_rasterizer(&self, style: &FontStyle) -> Result<TextRasterizer> {
        let height = style.height.unsigned_abs();
        ensure!(
            (1..=4096).contains(&height),
            "font size is outside supported range"
        );
        let index = style
            .face
            .split(',')
            .find_map(|name| self.names.get(name.trim()))
            .with_context(|| format!("private font face not found: {}", style.face))?;
        let source = &self.faces[*index];
        let metadata = ttf_parser::Face::parse(&source.data, source.index)?;
        let scale = height as f64 / metadata.units_per_em() as f64;
        let (ascender, descender) = metadata
            .tables()
            .os2
            .map_or((metadata.ascender(), metadata.descender()), |os2| {
                (os2.windows_ascender(), os2.windows_descender())
            });
        let ascent = (ascender as f64 * scale).round() as i32;
        let cell_height = ascent + (descender.unsigned_abs() as f64 * scale).round() as i32;
        let mut lines = Vec::new();
        for (enabled, metrics, underline) in [
            (style.underline, metadata.underline_metrics(), true),
            (style.strikeout, metadata.strikeout_metrics(), false),
        ] {
            if enabled && let Some(metrics) = metrics {
                let mut y = ascent - (metrics.position as f64 * scale).round() as i32;
                if underline {
                    y = y.min(cell_height - 1);
                }
                let thickness = (metrics.thickness as f64 * scale).round() as i32;
                if y >= 0 && thickness > 0 {
                    lines.push((y, thickness));
                }
            }
        }
        let library = Library::init().context("initialize FreeType")?;
        // freetype-rs keeps the library and the Arc-backed font data alive with the face.
        let face = library.new_memory_face2(source.data.clone(), source.index as isize)?;
        face.set_pixel_sizes(0, height)?;
        Ok(TextRasterizer {
            face,
            style: style.clone(),
            ascent,
            lines,
        })
    }
}

impl TextRasterizer {
    pub(crate) fn glyph(&mut self, unit: u16, antialias: bool) -> Result<TextGlyph> {
        let flags = LoadFlag::NO_BITMAP
            | if antialias {
                LoadFlag::TARGET_NORMAL
            } else {
                LoadFlag::TARGET_MONO
            };
        let index = self.face.get_char_index(unit as usize).unwrap_or(0);
        self.face
            .load_glyph(index, flags)
            .context("load font glyph")?;
        let advance = ((self.face.glyph().linear_hori_advance() as f64 / 65536.0).round() as i32)
            + if self.style.bold {
                (self.face.glyph().advance().x / 64 / 50 + 1) as i32
            } else {
                0
            };
        if self.style.bold {
            // SAFETY: this rasterizer exclusively owns the live face and its loaded slot.
            // The outline is mutable until the next load; no outline references escape.
            let slot = self.face.raw_mut().glyph;
            let strength = ((self.style.height.unsigned_abs() as i64 * 64 + 12) / 24) as _;
            let error =
                unsafe { freetype::ffi::FT_Outline_Embolden(&mut (*slot).outline, strength) };
            ensure!(error == 0, "cannot embolden glyph: FreeType error {error}");
        }
        let glyph = self.face.glyph().get_glyph()?;
        let angle = self.style.angle as f64 * std::f64::consts::PI / 1800.0;
        let (sin, cos) = angle.sin_cos();
        let shear = if self.style.italic { 0.25 } else { 0.0 };
        if self.style.angle != 0 || self.style.italic {
            glyph.transform(
                Some(Matrix {
                    xx: (cos * 65536.0).round() as _,
                    xy: ((cos * shear - sin) * 65536.0).round() as _,
                    yx: (sin * 65536.0).round() as _,
                    yy: ((sin * shear + cos) * 65536.0).round() as _,
                }),
                None,
            )?;
        }
        let bounds = glyph.get_cbox(freetype::ffi::FT_GLYPH_BBOX_PIXELS);
        let w = bounds.xMax - bounds.xMin;
        let h = bounds.yMax - bounds.yMin;
        ensure!(
            w >= 0 && h >= 0 && w <= 8192 && h <= 8192 && w * h <= 16_000_000,
            "glyph bitmap exceeds size limit"
        );
        let bitmap = glyph.to_bitmap(
            if antialias {
                RenderMode::Normal
            } else {
                RenderMode::Mono
            },
            None,
        )?;
        let buffer = bitmap.bitmap();
        let width = buffer.width() as usize;
        let height = buffer.rows() as usize;
        let pitch = buffer.pitch().unsigned_abs() as usize;
        ensure!(
            width * height <= 16_000_000,
            "glyph bitmap exceeds size limit"
        );
        let mut coverage = vec![0; width * height];
        for y in 0..height {
            let row = if buffer.pitch() >= 0 {
                y
            } else {
                height - 1 - y
            } * pitch;
            for x in 0..width {
                coverage[y * width + x] = if antialias {
                    ((buffer.buffer()[row + x] as u16 * 65) >> 8) as u8
                } else if buffer.buffer()[row + x / 8] & (0x80 >> (x % 8)) != 0 {
                    64
                } else {
                    0
                };
            }
        }
        let angle90 = angle + std::f64::consts::FRAC_PI_2;
        let mut result = TextGlyph {
            left: bitmap.left() + (-angle90.cos() * self.ascent as f64) as i32,
            top: -bitmap.top() + (angle90.sin() * self.ascent as f64) as i32,
            width,
            height,
            coverage,
            advance: [
                (cos * advance as f64) as i32,
                (-sin * advance as f64) as i32,
            ],
        };
        for &(y, thickness) in &self.lines {
            result.add_line(y, thickness)?;
        }
        Ok(result)
    }
}

impl TextGlyph {
    fn add_line(&mut self, y: i32, thickness: i32) -> Result<()> {
        let line_top = (y - thickness / 2).max(0);
        let left = self.left.min(0);
        let top = self.top.min(line_top);
        let right = (self.left + self.width as i32).max(self.advance[0]);
        let bottom = (self.top + self.height as i32).max(line_top + thickness);
        let width = (right - left).max(0) as usize;
        let height = (bottom - top).max(0) as usize;
        ensure!(
            width * height <= 16_000_000,
            "decorated glyph exceeds size limit"
        );
        let mut data = vec![0; width * height];
        for y in 0..self.height {
            let dest = (y + (self.top - top) as usize) * width + (self.left - left) as usize;
            data[dest..dest + self.width]
                .copy_from_slice(&self.coverage[y * self.width..(y + 1) * self.width]);
        }
        for y in line_top..line_top + thickness {
            data[(y - top) as usize * width..(y - top + 1) as usize * width].fill(64);
        }
        self.left = left;
        self.top = top;
        self.width = width;
        self.height = height;
        self.coverage = data;
        Ok(())
    }
}
