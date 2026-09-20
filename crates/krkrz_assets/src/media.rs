//! Bounded image and Vorbis decoding. TLG5 is adapted from GARbro ImageTLG.cs.
use crate::binary::Reader;
use anyhow::{Result, ensure};
use serde::Serialize;
use std::io::Cursor;
const MAX_PIXELS: usize = 64 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl Image {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.starts_with(b"TLG") {
            return tlg(bytes);
        }
        let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some((MAX_PIXELS * 4) as u64);
        reader.limits(limits);
        let decoded = reader.decode()?.into_rgba8();
        let (width, height) = decoded.dimensions();
        ensure!(
            width as usize * height as usize <= MAX_PIXELS,
            "image exceeds pixel limit"
        );
        Ok(Self {
            width,
            height,
            rgba: decoded.into_raw(),
        })
    }
    pub fn write_png(&self, path: &std::path::Path) -> Result<()> {
        ensure!(
            self.rgba.len() == self.width as usize * self.height as usize * 4,
            "invalid RGBA image size"
        );
        // Caller chooses an output; refuse to overwrite any existing file.
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(file).write_image(
            &self.rgba,
            self.width,
            self.height,
            image::ExtendedColorType::Rgba8,
        )?;
        Ok(())
    }
}
fn tlg(bytes: &[u8]) -> Result<Image> {
    let bytes = if bytes.starts_with(b"TLG0.0\0sds\x1a") {
        let mut r = Reader::new(&bytes[11..]);
        let len = r.u32()? as usize;
        r.take(len)?
    } else {
        bytes
    };
    ensure!(
        bytes.starts_with(b"TLG5.0\0raw\x1a"),
        "unsupported TLG image version (TLG5 implemented)"
    );
    let mut r = Reader::new(&bytes[11..]);
    let channels = r.take(1)?[0] as usize;
    ensure!(channels == 3 || channels == 4, "invalid TLG5 channel count");
    let width = r.u32()?;
    let height = r.u32()?;
    let block_height = r.u32()? as usize;
    ensure!(
        width > 0
            && height > 0
            && width <= 16384
            && height <= 16384
            && width as usize * height as usize <= MAX_PIXELS,
        "invalid TLG5 dimensions"
    );
    ensure!(
        block_height > 0 && block_height <= 16384,
        "invalid TLG5 block height"
    );
    let blocks = (height as usize).div_ceil(block_height);
    r.take(blocks * 4)?;
    let mut rgba = vec![0; width as usize * height as usize * 4];
    let mut window = [0u8; 4096];
    let mut cursor = 0;
    for top in (0..height as usize).step_by(block_height) {
        let rows = block_height.min(height as usize - top);
        let size = rows * width as usize;
        let mut planes = Vec::new();
        for _ in 0..channels {
            let mode = r.take(1)?[0];
            let packed = r.u32()? as usize;
            let bytes = r.take(packed)?;
            let plane = match mode {
                0 => slide(bytes, size, &mut window, &mut cursor)?,
                1 => {
                    ensure!(bytes.len() == size, "TLG5 raw plane length mismatch");
                    bytes.to_vec()
                }
                _ => anyhow::bail!("unsupported TLG5 plane encoding {mode}"),
            };
            planes.push(plane);
        }
        for y in 0..rows {
            let mut previous = [0u8; 4];
            for x in 0..width as usize {
                let i = y * width as usize + x;
                let mut delta = [
                    planes[2][i].wrapping_add(planes[1][i]),
                    planes[1][i],
                    planes[0][i].wrapping_add(planes[1][i]),
                    if channels == 4 { planes[3][i] } else { 0 },
                ];
                let out = ((top + y) * width as usize + x) * 4;
                for c in 0..channels {
                    previous[c] = previous[c].wrapping_add(delta[c]);
                    delta[c] = previous[c];
                    if top + y > 0 {
                        delta[c] = delta[c].wrapping_add(rgba[out - width as usize * 4 + c]);
                    }
                }
                if channels == 3 {
                    delta[3] = 255;
                }
                rgba[out..out + 4].copy_from_slice(&delta);
            }
        }
    }
    ensure!(r.done(), "trailing TLG5 raw data");
    Ok(Image {
        width,
        height,
        rgba,
    })
}
fn slide(
    bytes: &[u8],
    size: usize,
    window: &mut [u8; 4096],
    cursor: &mut usize,
) -> Result<Vec<u8>> {
    let mut r = Reader::new(bytes);
    let mut flags = 0u16;
    let mut out = Vec::with_capacity(size);
    while !r.done() {
        flags >>= 1;
        if flags & 256 == 0 {
            flags = u16::from(r.take(1)?[0]) | 0xff00;
        }
        if flags & 1 != 0 {
            let pair = r.take(2)?;
            let mut pos = pair[0] as usize | ((pair[1] as usize & 15) << 8);
            let mut len = (pair[1] as usize >> 4) + 3;
            if len == 18 {
                len += r.take(1)?[0] as usize;
            }
            ensure!(out.len() + len <= size, "TLG5 LZSS output exceeds plane");
            for _ in 0..len {
                let b = window[pos];
                out.push(b);
                window[*cursor] = b;
                *cursor = (*cursor + 1) & 4095;
                pos = (pos + 1) & 4095;
            }
        } else {
            let b = r.take(1)?[0];
            ensure!(out.len() < size, "TLG5 literal exceeds plane");
            out.push(b);
            window[*cursor] = b;
            *cursor = (*cursor + 1) & 4095;
        }
    }
    ensure!(out.len() == size, "incomplete TLG5 plane");
    Ok(out)
}
#[derive(Clone, Debug, Serialize)]
pub struct Audio {
    pub channels: u8,
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}
impl Audio {
    /// Decode integer PCM WAVE data into the mixer's signed 16-bit format.
    pub fn decode_wave(bytes: &[u8], max_frames: usize) -> Result<Self> {
        let mut header = Reader::new(bytes);
        ensure!(header.take(4)? == b"RIFF", "not a RIFF wave");
        let length = header.u32()? as usize;
        ensure!(length >= 4, "invalid RIFF size");
        let mut chunks = Reader::new(header.take(length)?);
        ensure!(chunks.take(4)? == b"WAVE", "not a WAVE container");
        let mut format = None;
        let mut data = None;
        while !chunks.done() {
            let tag = chunks.take(4)?;
            let length = chunks.u32()? as usize;
            let body = chunks.take(length)?;
            match tag {
                b"fmt " => {
                    ensure!(format.is_none(), "duplicate wave format chunk");
                    let mut f = Reader::new(body);
                    let codec = f.u16()?;
                    let channels = f.u16()?;
                    let rate = f.u32()?;
                    let byte_rate = f.u32()?;
                    let align = f.u16()?;
                    let bits = f.u16()?;
                    ensure!(codec == 1 && [8, 16, 24, 32].contains(&bits), "unsupported wave PCM format");
                    ensure!((1..=8).contains(&channels) && (1..=768_000).contains(&rate), "invalid wave channel count or rate");
                    ensure!(align == channels * (bits / 8) && byte_rate == rate * align as u32, "invalid wave block alignment");
                    format = Some((channels as u8, rate, align as usize, bits));
                }
                b"data" => { ensure!(data.is_none(), "duplicate wave data chunk"); data = Some(body); }
                _ => {}
            }
            if length % 2 != 0 { chunks.take(1)?; }
        }
        let (channels, sample_rate, align, bits) = format.ok_or_else(|| anyhow::anyhow!("wave format chunk missing"))?;
        let data = data.ok_or_else(|| anyhow::anyhow!("wave data chunk missing"))?;
        ensure!(data.len().is_multiple_of(align) && data.len() / align <= max_frames, "wave frame count is invalid or exceeds limit");
        let samples = data.chunks_exact((bits / 8) as usize).map(|s| match bits {
            8 => (s[0] as i16 - 128) << 8,
            16 => i16::from_le_bytes([s[0], s[1]]),
            24 => i16::from_le_bytes([s[1], s[2]]),
            32 => i16::from_le_bytes([s[2], s[3]]),
            _ => unreachable!(),
        }).collect();
        Ok(Self { channels, sample_rate, samples })
    }
    pub fn decode_vorbis(bytes: &[u8], max_frames: usize) -> Result<Self> {
        let mut stream = lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes))?;
        let channels = stream.ident_hdr.audio_channels;
        let sample_rate = stream.ident_hdr.audio_sample_rate;
        ensure!(
            channels > 0 && sample_rate > 0,
            "invalid Vorbis stream format"
        );
        let limit = max_frames
            .checked_mul(channels as usize)
            .ok_or_else(|| anyhow::anyhow!("audio limit overflow"))?;
        let mut samples = Vec::new();
        while let Some(packet) = stream.read_dec_packet_itl()? {
            ensure!(
                samples.len() + packet.len() <= limit,
                "decoded audio exceeds frame limit"
            );
            samples.extend(packet);
        }
        ensure!(
            samples.len().is_multiple_of(channels as usize),
            "incomplete Vorbis frame"
        );
        Ok(Self {
            channels,
            sample_rate,
            samples,
        })
    }
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tlg5_pixels_and_truncation() {
        let mut bytes = b"TLG5.0\0raw\x1a\x03".to_vec();
        for n in [2u32, 2, 2, 0] {
            bytes.extend(n.to_le_bytes());
        }
        for plane in [[10u8, 1, 2, 0], [20, 2, 3, 0], [30, 3, 4, 0]] {
            bytes.push(1);
            bytes.extend(4u32.to_le_bytes());
            bytes.extend(plane);
        }
        let image = Image::decode(&bytes).unwrap();
        assert_eq!(
            image.rgba,
            [
                50, 20, 30, 255, 55, 22, 33, 255, 57, 23, 35, 255, 62, 25, 38, 255
            ]
        );
        for n in 0..bytes.len() {
            assert!(Image::decode(&bytes[..n]).is_err());
        }
    }
    #[test]
    fn slide_overlap_and_bounds() {
        let mut window = [0; 4096];
        let mut cursor = 0;
        assert_eq!(
            slide(&[2, b'a', 0, 0], 4, &mut window, &mut cursor).unwrap(),
            b"aaaa"
        );
        assert!(slide(&[1, 0, 0xf0, 255], 10, &mut window, &mut cursor).is_err());
    }
}
