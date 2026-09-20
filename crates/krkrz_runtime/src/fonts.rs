//! Session-private OpenType fonts and CPU glyph coverage via ab_glyph/ttf-parser.
use ab_glyph::{Font, FontRef, point};
use anyhow::{Context, Result, ensure};
use krkrz_tjs::unsupported;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};

mod raster;
pub(crate) use raster::TextGlyph;

#[derive(Clone)]
struct Face {
    data: Arc<[u8]>,
    index: u32,
}
#[derive(Default)]
pub struct FontBook {
    faces: Vec<Face>,
    names: BTreeMap<String, usize>,
    sources: BTreeMap<[u8; 32], u32>,
    bytes: usize,
}
#[derive(Debug, PartialEq)]
pub struct GlyphBitmap {
    /// Bitmap origin relative to the glyph baseline.
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub advance: f32,
    pub coverage: Vec<u8>,
}
impl FontBook {
    /// FontSystem::GetBeingFont chooses the first available named candidate,
    /// then the default font. Use a bundled Japanese face as the portable
    /// default so Windows-only saved preferences remain readable on Linux.
    fn resolve_face(&self, name: &str) -> Result<&Face> {
        let named = name
            .split(',')
            .find_map(|candidate| self.names.get(candidate.trim()).copied());
        let fallback = || {
            [
                "ＭＳ ゴシック",
                "MS Gothic",
                "源ノ角ゴシック JP Regular",
                "Noto Sans CJK JP",
                "Noto Sans JP",
            ]
            .iter()
            .find_map(|candidate| self.names.get(*candidate).copied())
            .or_else(|| {
                self.faces.iter().position(|face| {
                    ttf_parser::Face::parse(&face.data, face.index).is_ok_and(|font| {
                        font.glyph_index('あ').is_some() && font.glyph_index('A').is_some()
                    })
                })
            })
            .or_else(|| (!self.faces.is_empty()).then_some(0))
        };
        named
            .or_else(fallback)
            .map(|index| &self.faces[index])
            .with_context(|| format!("no registered font is available for: {name}"))
    }
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(String::as_str)
    }
    /// Kirikiri measures UTF-16 code units separately, with integral advances
    /// and no kerning. Bold adds the GDI rasterizer's synthetic advance.
    pub fn text_width(&self, name: &str, text: &[u16], height: u32, bold: bool) -> Result<i32> {
        ensure!(height <= 4096, "font size is outside supported range");
        if text.is_empty() || text[0] == 0 {
            return Ok(0);
        }
        let face = self.resolve_face(name)?;
        let font = ttf_parser::Face::parse(&face.data, face.index)
            .context("registered font is invalid")?;
        let scale = height as f64 / font.units_per_em() as f64;
        let cell_height =
            ((font.ascender() as i32 - font.descender() as i32) as f64 * scale).round() as i32;
        let extra = if bold { cell_height / 50 + 1 } else { 0 };
        let mut width = 0i32;
        for unit in text.iter().take_while(|c| **c != 0) {
            let glyph = char::from_u32((*unit).into())
                .and_then(|c| font.glyph_index(c))
                .unwrap_or(ttf_parser::GlyphId(0));
            let advance =
                (font.glyph_hor_advance(glyph).unwrap_or(0) as f64 * scale).round() as i32;
            width = width
                .checked_add(advance + extra)
                .context("text width overflow")?;
        }
        Ok(width)
    }
    /// Register complete TTF/OTF/TTC data and return its face count, or zero for invalid data.
    pub fn add(&mut self, data: Vec<u8>) -> Result<u32> {
        let hash: [u8; 32] = Sha256::digest(&data).into();
        if let Some(count) = self.sources.get(&hash) {
            return Ok(*count);
        }
        if data.len() > 64 << 20 || self.bytes.saturating_add(data.len()) > 256 << 20 {
            return Err(unsupported("private font storage limit exceeded"));
        }
        let count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        if count > 64 || self.faces.len() + count as usize > 256 {
            return Err(unsupported("private font face limit exceeded"));
        }
        let mut names = Vec::new();
        for index in 0..count {
            let Ok(face) = ttf_parser::Face::parse(&data, index) else {
                return Ok(0);
            };
            if FontRef::try_from_slice_and_index(&data, index).is_err() {
                return Ok(0);
            }
            for name in face.names() {
                if matches!(
                    name.name_id,
                    ttf_parser::name_id::FAMILY
                        | ttf_parser::name_id::FULL_NAME
                        | ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                ) && let Some(name) = name.to_string()
                    && !name.is_empty()
                {
                    names.push((name, self.faces.len() + index as usize));
                }
            }
        }
        self.bytes += data.len();
        let data: Arc<[u8]> = data.into();
        for index in 0..count {
            self.faces.push(Face {
                data: data.clone(),
                index,
            });
        }
        for (name, index) in names {
            self.names.insert(name, index);
        }
        self.sources.insert(hash, count);
        Ok(count)
    }
    /// Rasterize a glyph at an em size in pixels. The caller supplies text layout.
    pub fn rasterize(&self, name: &str, character: char, em_pixels: f32) -> Result<GlyphBitmap> {
        ensure!(
            em_pixels.is_finite() && em_pixels > 0.0 && em_pixels <= 4096.0,
            "font size is outside supported range"
        );
        let face = self.resolve_face(name)?;
        let font = FontRef::try_from_slice_and_index(&face.data, face.index)
            .context("registered font is invalid")?;
        let units = font.units_per_em().context("font has no em size")?;
        let scale = em_pixels * font.height_unscaled() / units;
        let id = font.glyph_id(character);
        let advance = font.h_advance_unscaled(id) * em_pixels / units;
        let Some(outline) = font.outline_glyph(id.with_scale_and_position(scale, point(0.0, 0.0)))
        else {
            return Ok(GlyphBitmap {
                left: 0,
                top: 0,
                width: 0,
                height: 0,
                advance,
                coverage: Vec::new(),
            });
        };
        let bounds = outline.px_bounds();
        let width = bounds.width() as u32;
        let height = bounds.height() as u32;
        let length = (width as usize)
            .checked_mul(height as usize)
            .filter(|n| *n <= 16_000_000)
            .ok_or_else(|| unsupported("glyph bitmap exceeds size limit"))?;
        let mut coverage = vec![0; length];
        outline.draw(|x, y, value| {
            coverage[y as usize * width as usize + x as usize] =
                (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        });
        Ok(GlyphBitmap {
            left: bounds.min.x as i32,
            top: bounds.min.y as i32,
            width,
            height,
            advance,
            coverage,
        })
    }
}
