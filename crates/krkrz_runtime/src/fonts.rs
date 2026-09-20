//! Session-private OpenType fonts and CPU glyph coverage via Fontations/Zeno.
use anyhow::{Context, Result, ensure};
use krkrz_tjs::unsupported;
use sha2::{Digest, Sha256};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, Size},
    raw::{FileRef, TableProvider},
    string::StringId,
};
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
                    FontRef::from_index(&face.data, face.index).is_ok_and(|font| {
                        font.charmap().map('あ').is_some() && font.charmap().map('A').is_some()
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
        let font =
            FontRef::from_index(&face.data, face.index).context("registered font is invalid")?;
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let glyphs = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        let scale = height as f64 / metrics.units_per_em as f64;
        let cell_height = ((metrics.ascent - metrics.descent) as f64 * scale).round() as i32;
        let extra = if bold { cell_height / 50 + 1 } else { 0 };
        let mut width = 0i32;
        for unit in text.iter().take_while(|c| **c != 0) {
            let glyph = char::from_u32((*unit).into())
                .and_then(|c| font.charmap().map(c))
                .unwrap_or_default();
            let advance =
                (glyphs.advance_width(glyph).unwrap_or(0.0) as f64 * scale).round() as i32;
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
        let Ok(file) = FileRef::new(&data) else {
            return Ok(0);
        };
        let count = match &file {
            FileRef::Font(_) => 1,
            FileRef::Collection(c) => c.len(),
        };
        if count > 64 || self.faces.len() + count as usize > 256 {
            return Err(unsupported("private font face limit exceeded"));
        }
        let mut names = Vec::new();
        for (index, face) in file.fonts().enumerate() {
            let Ok(face) = face else {
                return Ok(0);
            };
            // FontRef validates the directory lazily; require the tables used by
            // layout now so registration still rejects incomplete font data.
            if !face
                .head()
                .is_ok_and(|head| (16..=16384).contains(&head.units_per_em()))
                || face.hhea().is_err()
                || face.maxp().is_err()
                || face.hmtx().is_err()
            {
                return Ok(0);
            }
            for id in [
                StringId::FAMILY_NAME,
                StringId::FULL_NAME,
                StringId::TYPOGRAPHIC_FAMILY_NAME,
            ] {
                for name in face.localized_strings(id) {
                    let name = name.to_string();
                    if !name.is_empty() {
                        names.push((name, self.faces.len() + index));
                    }
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
        raster::rasterize(face, character, em_pixels)
    }
}
