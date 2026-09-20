use super::*;
use image::ImageDecoder;

pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// None means grayscale; otherwise pixels are palette indices.
    pub palette: Option<Vec<[u8; 3]>>,
}

impl IndexedImage {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.starts_with(b"BM") {
            let mut decoder = image::codecs::bmp::BmpDecoder::new(Cursor::new(bytes))?;
            let (width, height) = decoder.dimensions();
            check_dimensions(width, height)?;
            let palette = decoder.get_palette().map(|p| p.to_vec());
            ensure!(palette.is_some(), "province BMP must be palettized");
            decoder.set_indexed_color(true);
            let mut pixels = vec![0; width as usize * height as usize];
            decoder.read_image(&mut pixels)?;
            return Ok(Self {
                width,
                height,
                pixels,
                palette,
            });
        }
        let mut decoder = png::Decoder::new_with_limits(
            Cursor::new(bytes),
            png::Limits {
                bytes: MAX_PIXELS * 4,
            },
        );
        decoder.set_transformations(png::Transformations::IDENTITY);
        let mut reader = decoder.read_info()?;
        let info = reader.info();
        let (width, height) = (info.width, info.height);
        check_dimensions(width, height)?;
        ensure!(
            matches!(
                info.color_type,
                png::ColorType::Indexed | png::ColorType::Grayscale
            ),
            "province PNG must be palettized or grayscale"
        );
        let depth = info.bit_depth as usize;
        ensure!(depth <= 8, "province PNG depth exceeds 8 bits");
        let palette = info.palette.as_ref().map(|p| p.as_chunks::<3>().0.to_vec());
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| anyhow::anyhow!("PNG buffer size overflow"))?;
        ensure!(size <= MAX_PIXELS, "province PNG exceeds limit");
        let mut raw = vec![0; size];
        let frame = reader.next_frame(&mut raw)?;
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        let max = (1usize << depth) - 1;
        for row in raw[..frame.buffer_size()].chunks_exact(frame.line_size) {
            for x in 0..width as usize {
                let value = (row[x * depth / 8] as usize >> (8 - depth - x * depth % 8)) & max;
                pixels.push(if palette.is_some() {
                    value as u8
                } else {
                    (value * 255 / max) as u8
                });
            }
        }
        Ok(Self {
            width,
            height,
            pixels,
            palette,
        })
    }
}

fn check_dimensions(w: u32, h: u32) -> Result<()> {
    ensure!(
        w > 0 && h > 0 && w <= 16384 && h <= 16384 && w as usize * h as usize <= MAX_PIXELS,
        "invalid indexed image dimensions"
    );
    Ok(())
}
