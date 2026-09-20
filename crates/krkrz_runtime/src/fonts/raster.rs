//! Hinted, session-private glyphs. Layout and 65-level coverage follow Kirikiri's
//! GDI rasterizer; Fontations supplies outline loading/hinting and Zeno rasterizes.
use super::*;
use crate::font::Font as FontStyle;
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, HintingInstance, OutlinePen, Target},
};
use zeno::{Command, Mask, PathBuilder, Transform};

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
    face: Face,
    smooth: HintingInstance,
    mono: HintingInstance,
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
        let source = self.resolve_face(&style.face)?;
        let font = skrifa::FontRef::from_index(&source.data, source.index)?;
        let metadata = font.metrics(Size::unscaled(), LocationRef::default());
        let scale = height as f64 / metadata.units_per_em as f64;
        let (ascender, descender) = font
            .os2()
            .map_or((metadata.ascent as f64, metadata.descent as f64), |os2| {
                (os2.us_win_ascent() as f64, os2.us_win_descent() as f64)
            });
        let ascent = (ascender * scale).round() as i32;
        let cell_height = ascent + (descender.abs() * scale).round() as i32;
        let mut lines = Vec::new();
        for (enabled, metrics, underline) in [
            (style.underline, metadata.underline, true),
            (style.strikeout, metadata.strikeout, false),
        ] {
            if enabled && let Some(metrics) = metrics {
                let mut y = ascent - (metrics.offset as f64 * scale).round() as i32;
                if underline {
                    y = y.min(cell_height - 1);
                }
                let thickness = (metrics.thickness as f64 * scale).round() as i32;
                if y >= 0 && thickness > 0 {
                    lines.push((y, thickness));
                }
            }
        }
        let outlines = font.outline_glyphs();
        let size = Size::new(height as f32);
        let smooth =
            HintingInstance::new(&outlines, size, LocationRef::default(), Target::default())?;
        let mono = HintingInstance::new(&outlines, size, LocationRef::default(), Target::Mono)?;
        Ok(TextRasterizer {
            face: source.clone(),
            smooth,
            mono,
            style: style.clone(),
            ascent,
            lines,
        })
    }
}

impl TextRasterizer {
    pub(crate) fn glyph(&mut self, unit: u16, antialias: bool) -> Result<TextGlyph> {
        let font = skrifa::FontRef::from_index(&self.face.data, self.face.index)?;
        let id = font.charmap().map(unit as u32).unwrap_or_default();
        let linear_advance =
            advance_width(&font, id, self.style.height.unsigned_abs() as f32).round() as i32;
        let mut advance = linear_advance;
        let mut pen = Pen::default();
        if let Some(glyph) = font.outline_glyphs().get(id) {
            let hinting = if antialias { &self.smooth } else { &self.mono };
            let adjusted = glyph.draw(DrawSettings::hinted(hinting, false), &mut pen)?;
            if self.style.bold {
                advance += adjusted
                    .advance_width
                    .unwrap_or(linear_advance as f32)
                    .round() as i32
                    / 50
                    + 1;
            }
        } else if self.style.bold {
            advance += linear_advance / 50 + 1;
        }
        let angle = self.style.angle as f64 * std::f64::consts::PI / 1800.0;
        let (sin, cos) = angle.sin_cos();
        let shear = if self.style.italic { 0.25 } else { 0.0 };
        // Outlines use a Y-up baseline; the bitmap and layer use Y-down.
        let transform = Transform::new(
            cos as f32,
            -sin as f32,
            (cos * shear - sin) as f32,
            -(sin * shear + cos) as f32,
            0.0,
            0.0,
        );
        let (mut coverage, left, top, width, height) = render_outline(
            &pen.0,
            transform,
            self.style
                .bold
                .then_some(((self.style.height.unsigned_abs() * 64 + 12) / 24) as f32 / 64.0),
        )?;
        for value in &mut coverage {
            *value = if antialias {
                ((*value as u16 * 65) >> 8) as u8
            } else if *value >= 128 {
                64
            } else {
                0
            };
        }
        let angle90 = angle + std::f64::consts::FRAC_PI_2;
        let mut result = TextGlyph {
            left: left + (-angle90.cos() * self.ascent as f64) as i32,
            top: top + (angle90.sin() * self.ascent as f64) as i32,
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

// Scale unrounded design metrics in f64, as getTextWidth does. Skrifa's
// pixel-scaled metrics deliberately use 16.16 precision for outline compatibility.
fn advance_width(font: &skrifa::FontRef<'_>, id: skrifa::GlyphId, pixels: f32) -> f64 {
    let size = Size::unscaled();
    let location = LocationRef::default();
    font.glyph_metrics(size, location)
        .advance_width(id)
        .unwrap_or(0.0) as f64
        * pixels as f64
        / font.metrics(size, location).units_per_em as f64
}

pub(super) fn rasterize(face: &Face, character: char, em_pixels: f32) -> Result<GlyphBitmap> {
    let font = skrifa::FontRef::from_index(&face.data, face.index)?;
    let size = Size::new(em_pixels);
    let location = LocationRef::default();
    let id = font.charmap().map(character).unwrap_or_default();
    let advance = advance_width(&font, id, em_pixels) as f32;
    let mut pen = Pen::default();
    if let Some(glyph) = font.outline_glyphs().get(id) {
        glyph.draw(DrawSettings::unhinted(size, location), &mut pen)?;
    }
    let (coverage, left, top, width, height) =
        render_outline(&pen.0, Transform::scale(1.0, -1.0), None)?;
    Ok(GlyphBitmap {
        coverage,
        left,
        top,
        width: width as u32,
        height: height as u32,
        advance,
    })
}

/// Adapter only: curves, hinting, stroking and scan conversion remain in libraries.
#[derive(Default)]
struct Pen(Vec<Command>);
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to([x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to([x, y]);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.quad_to([cx, cy], [x, y]);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to([cx0, cy0], [cx1, cy1], [x, y]);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

fn render_outline(
    path: &[Command],
    transform: Transform,
    bold: Option<f32>,
) -> Result<(Vec<u8>, i32, i32, usize, usize)> {
    if path.is_empty() {
        return Ok((Vec::new(), 0, 0, 0, 0));
    }
    // A centered stroke plus translation grows the synthetic bold outline toward
    // the right and up, preserving the original baseline and left bearing.
    let mut transform = transform;
    if let Some(strength) = bold {
        transform.x += (transform.xx + transform.yx) * strength * 0.5;
        transform.y += (transform.xy + transform.yy) * strength * 0.5;
    }
    let stroke = bold.map(zeno::Stroke::new);
    let bounds = zeno::bounds(
        path,
        stroke
            .as_ref()
            .map_or(zeno::Style::Fill(zeno::Fill::NonZero), |s| s.into()),
        Some(transform),
    );
    let left = bounds.min.x.floor() as i32;
    let top = bounds.min.y.floor() as i32;
    let width = (bounds.max.x.ceil() - left as f32) as usize;
    let height = (bounds.max.y.ceil() - top as f32) as usize;
    ensure!(
        width <= 8192 && height <= 8192 && width * height <= 16_000_000,
        "glyph bitmap exceeds size limit"
    );
    if width == 0 || height == 0 {
        return Ok((Vec::new(), left, top, 0, 0));
    }
    let mut mask = Mask::new(path);
    mask.transform(Some(transform))
        .size(width as u32, height as u32)
        .offset([-left as f32, -top as f32]);
    let mut coverage = vec![0; width * height];
    mask.render_into(&mut coverage, None);
    if let Some(stroke) = stroke {
        let mut edge = vec![0; coverage.len()];
        mask.style(stroke).render_into(&mut edge, None);
        for (fill, edge) in coverage.iter_mut().zip(edge) {
            *fill = (*fill).max(edge);
        }
    }
    Ok((coverage, left, top, width, height))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> FontBook {
        let mut book = FontBook::default();
        book.add(include_bytes!("../../tests/fixtures/synthetic.ttf").to_vec())
            .unwrap();
        book
    }

    #[test]
    fn hinted_glyphs_preserve_advance_rotation_and_coverage_contract() {
        let book = book();
        for height in [10, 20, 51] {
            for angle in [0, 450, 900, -900] {
                for bold in [false, true] {
                    let style = FontStyle {
                        height,
                        angle,
                        bold,
                        italic: true,
                        ..FontStyle::default()
                    };
                    let mut raster = book.text_rasterizer(&style).unwrap();
                    let smooth = raster.glyph('A' as u16, true).unwrap();
                    let mono = raster.glyph('A' as u16, false).unwrap();
                    let linear = (height as f64 * 0.7).round() as i32;
                    let advance = linear + if bold { linear / 50 + 1 } else { 0 };
                    let theta = angle as f64 * std::f64::consts::PI / 1800.0;
                    assert_eq!(
                        smooth.advance,
                        [
                            (theta.cos() * advance as f64) as i32,
                            (-theta.sin() * advance as f64) as i32
                        ]
                    );
                    assert_eq!(smooth.advance, mono.advance);
                    assert!(smooth.coverage.iter().all(|v| *v <= 64));
                    assert!(smooth.coverage.iter().any(|v| *v > 0 && *v < 64));
                    assert!(mono.coverage.iter().all(|v| matches!(*v, 0 | 64)));
                    assert!(mono.coverage.contains(&64));
                    assert_eq!(smooth.coverage.len(), smooth.width * smooth.height);
                }
            }
        }
    }

    #[test]
    fn whitespace_decorations_and_size_limits() {
        let book = book();
        let mut style = FontStyle {
            height: 20,
            ..FontStyle::default()
        };
        let space = book
            .text_rasterizer(&style)
            .unwrap()
            .glyph(32, true)
            .unwrap();
        assert_eq!(space.advance, [14, 0]);
        assert!(space.coverage.is_empty());
        // This fixture has zero-width decoration metrics: enabling decorations
        // must not manufacture a bitmap or change the whitespace advance.
        style.underline = true;
        style.strikeout = true;
        let decorated = book
            .text_rasterizer(&style)
            .unwrap()
            .glyph(32, true)
            .unwrap();
        assert_eq!(decorated.advance, space.advance);
        assert!(decorated.coverage.is_empty());
        for height in [0, 4097, i32::MIN] {
            style.height = height;
            assert!(book.text_rasterizer(&style).is_err());
        }
    }
    #[test]
    fn underline_and_strikeout_use_font_metrics_on_whitespace() {
        let mut data = include_bytes!("../../tests/fixtures/synthetic.ttf").to_vec();
        let count = u16::from_be_bytes(data[4..6].try_into().unwrap()) as usize;
        for table in 0..count {
            let record = 12 + table * 16;
            let offset =
                u32::from_be_bytes(data[record + 8..record + 12].try_into().unwrap()) as usize;
            if &data[record..record + 4] == b"post" {
                data[offset + 8..offset + 10].copy_from_slice(&(-100i16).to_be_bytes());
                data[offset + 10..offset + 12].copy_from_slice(&50i16.to_be_bytes());
            } else if &data[record..record + 4] == b"OS/2" {
                data[offset + 26..offset + 28].copy_from_slice(&50i16.to_be_bytes());
                data[offset + 28..offset + 30].copy_from_slice(&300i16.to_be_bytes());
            }
        }
        let mut book = FontBook::default();
        book.add(data).unwrap();
        let style = FontStyle {
            height: 20,
            underline: true,
            strikeout: true,
            ..FontStyle::default()
        };
        let glyph = book
            .text_rasterizer(&style)
            .unwrap()
            .glyph(32, true)
            .unwrap();
        assert_eq!(
            (glyph.left, glyph.top, glyph.width, glyph.height),
            (0, 10, 14, 9)
        );
        assert_eq!(glyph.advance, [14, 0]);
        for (row, coverage) in glyph.coverage.as_chunks::<14>().0.iter().enumerate() {
            assert!(
                coverage
                    .iter()
                    .all(|v| *v == if row == 0 || row == 8 { 64 } else { 0 })
            );
        }
    }
}
