//! Bounded image, integer WAVE PCM, and Vorbis decoding.
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
mod indexed;
mod metadata;
pub use indexed::IndexedImage;

impl Image {
    pub fn metadata(bytes: &[u8]) -> Result<std::collections::BTreeMap<String, String>> {
        metadata::metadata(bytes)
    }
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
    if bytes.starts_with(b"TLG6.0\0raw\x1a") {
        return tlg6(bytes);
    }
    ensure!(
        bytes.starts_with(b"TLG5.0\0raw\x1a"),
        "unsupported TLG image version"
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
    // libtlg allocates its plane buffers using the declared block height.
    ensure!(
        block_height * width as usize * (channels + 1) <= MAX_PIXELS * 4,
        "TLG5 block buffers exceed memory limit"
    );
    for top in (0..height as usize).step_by(block_height) {
        let size = block_height.min(height as usize - top) * width as usize;
        for _ in 0..channels {
            let mode = r.take(1)?[0];
            let packed = r.u32()? as usize;
            let plane = r.take(packed)?;
            match mode {
                0 => {
                    ensure!(
                        packed <= block_height * width as usize + 10,
                        "TLG5 compressed plane exceeds decoder buffer"
                    );
                    validate_slide(plane, size)?;
                }
                1 => ensure!(packed == size, "TLG5 raw plane length mismatch"),
                _ => anyhow::bail!("unsupported TLG5 plane encoding {mode}"),
            }
        }
    }
    ensure!(r.done(), "trailing TLG5 raw data");
    decode_tlg(bytes, width, height, channels)
}

fn tlg6(bytes: &[u8]) -> Result<Image> {
    // Validate allocation sizes and all outer compressed spans before entering
    // the community decoder. Its safe indexing can panic on malformed entropy
    // data, so translate that into the same recoverable error as other formats.
    let mut r = Reader::new(&bytes[11..]);
    let colors = r.take(1)?[0];
    ensure!(matches!(colors, 1 | 3 | 4), "invalid TLG6 channel count");
    ensure!(r.take(3)? == [0, 0, 0], "unsupported TLG6 flags");
    let width = r.u32()?;
    let height = r.u32()?;
    ensure!(
        width > 0
            && height > 0
            && width <= 16384
            && height <= 16384
            && width as usize * height as usize <= MAX_PIXELS,
        "invalid TLG6 dimensions"
    );
    let max_bits = r.u32()?;
    ensure!(
        max_bits as usize / 8 <= bytes.len() && max_bits <= 64 * 1024 * 1024,
        "TLG6 bit pool exceeds limit"
    );
    let filters = r.u32()? as usize;
    r.take(filters)?;
    for _ in 0..height.div_ceil(8) {
        for _ in 0..colors {
            let bits = r.u32()?;
            ensure!(
                bits >> 30 == 0 && bits <= max_bits,
                "invalid TLG6 entropy length"
            );
            r.take((bits as usize).div_ceil(8))?;
        }
    }
    ensure!(r.done(), "trailing TLG6 raw data");
    decode_tlg(bytes, width, height, colors as usize)
}

fn decode_tlg(bytes: &[u8], width: u32, height: u32, colors: usize) -> Result<Image> {
    let decoded = std::panic::catch_unwind(|| libtlg_rs::load_tlg(Cursor::new(bytes)))
        .map_err(|_| anyhow::anyhow!("malformed TLG compressed data"))??;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for p in decoded.data.chunks_exact(colors) {
        if colors == 1 {
            rgba.extend_from_slice(&[p[0], p[0], p[0], 255]);
        } else {
            rgba.extend_from_slice(&[p[2], p[1], p[0], if colors == 4 { p[3] } else { 255 }]);
        }
    }
    ensure!(
        rgba.len() == width as usize * height as usize * 4,
        "invalid TLG output size"
    );
    Ok(Image {
        width,
        height,
        rgba,
    })
}
// Validate token lengths without maintaining a dictionary or decoding pixels.
// The upstream decoder does not reject incomplete or overlong planes itself.
fn validate_slide(bytes: &[u8], size: usize) -> Result<()> {
    let mut r = Reader::new(bytes);
    let mut flags = 0u16;
    let mut produced = 0usize;
    while !r.done() {
        flags >>= 1;
        if flags & 256 == 0 {
            flags = u16::from(r.take(1)?[0]) | 0xff00;
        }
        let count = if flags & 1 != 0 {
            let pair = r.take(2)?;
            let length = (pair[1] as usize >> 4) + 3;
            if length == 18 {
                length + r.take(1)?[0] as usize
            } else {
                length
            }
        } else {
            r.take(1)?;
            1
        };
        produced += count;
        ensure!(produced <= size, "TLG5 LZSS output exceeds plane");
    }
    ensure!(produced == size, "incomplete TLG5 plane");
    Ok(())
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
        use std::io::Read;

        let length = hound::read_wave_header(&mut Cursor::new(bytes))?;
        ensure!(
            length >= 12 && length <= bytes.len() as u64,
            "invalid RIFF size"
        );
        // Hound expects fmt before data and stops at the first data chunk.
        // Keep our bounded chunk-order/duplicate/padding validation, then expose
        // a canonical header plus the original data slice without copying PCM.
        let mut chunks = Reader::new(&bytes[12..length as usize]);
        let mut format = None;
        let mut data = None;
        while !chunks.done() {
            let tag = chunks.take(4)?;
            let length = chunks.u32()? as usize;
            let body = chunks.take(length)?;
            match tag {
                b"fmt " => {
                    ensure!(format.is_none(), "duplicate wave format chunk");
                    ensure!(body.len() >= 16, "invalid wave format chunk");
                    ensure!(body[..2] == [1, 0], "unsupported wave PCM format");
                    format = Some(&body[..16]);
                }
                b"data" => {
                    ensure!(data.is_none(), "duplicate wave data chunk");
                    data = Some(body);
                }
                _ => {}
            }
            if !length.is_multiple_of(2) {
                chunks.take(1)?;
            }
        }
        let format = format.ok_or_else(|| anyhow::anyhow!("wave format chunk missing"))?;
        let data = data.ok_or_else(|| anyhow::anyhow!("wave data chunk missing"))?;
        let mut header = [0u8; 44];
        header[..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&(36 + data.len() as u32).to_le_bytes());
        header[8..20].copy_from_slice(b"WAVEfmt \x10\0\0\0");
        header[20..36].copy_from_slice(format);
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&(data.len() as u32).to_le_bytes());
        let mut reader = hound::WavReader::new(Cursor::new(header).chain(Cursor::new(data)))?;
        let spec = reader.spec();
        ensure!(
            [8, 16, 24, 32].contains(&spec.bits_per_sample),
            "unsupported wave PCM format"
        );
        ensure!(
            (1..=8).contains(&spec.channels) && (1..=768_000).contains(&spec.sample_rate),
            "invalid wave channel count or rate"
        );
        ensure!(
            u16::from_le_bytes([format[12], format[13]])
                == spec.channels * (spec.bits_per_sample / 8),
            "invalid wave block alignment"
        );
        ensure!(
            reader.duration() as usize <= max_frames,
            "decoded audio exceeds frame limit"
        );
        let samples = reader
            .samples::<i32>()
            .map(|sample| {
                let sample = sample?;
                Ok(if spec.bits_per_sample == 8 {
                    (sample << 8) as i16
                } else {
                    (sample >> (spec.bits_per_sample - 16)) as i16
                })
            })
            .collect::<Result<Vec<_>, hound::Error>>()?;
        Ok(Self {
            channels: spec.channels as u8,
            sample_rate: spec.sample_rate,
            samples,
        })
    }

    pub fn decode_vorbis(bytes: &[u8], max_frames: usize) -> Result<Self> {
        let mut stream = lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes))?;
        let channels = stream.ident_hdr.audio_channels;
        let sample_rate = stream.ident_hdr.audio_sample_rate;
        ensure!(
            (1..=8).contains(&channels) && (1..=768_000).contains(&sample_rate),
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
    fn wave(bits: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36u32 + data.len() as u32 + data.len() as u32 % 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(44100u32.to_le_bytes());
        bytes.extend((44100u32 * (bits as u32 / 8)).to_le_bytes());
        bytes.extend((bits / 8).to_le_bytes());
        bytes.extend(bits.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(data);
        if !data.len().is_multiple_of(2) {
            bytes.push(0);
        }
        bytes
    }
    #[test]
    fn wave_pcm_depths_and_bounds() {
        for (bits, data, expected) in [
            (8, vec![0, 128, 255], vec![-32768, 0, 32512]),
            (16, vec![0, 128, 0, 0, 255, 127], vec![-32768, 0, 32767]),
            (
                24,
                vec![0, 0, 128, 255, 255, 255, 255, 255, 127],
                vec![-32768, -1, 32767],
            ),
            (
                32,
                vec![0, 0, 0, 128, 255, 255, 255, 255, 255, 255, 255, 127],
                vec![-32768, -1, 32767],
            ),
        ] {
            let bytes = wave(bits, &data);
            let decoded = Audio::decode_wave(&bytes, 3).unwrap();
            assert_eq!(decoded.samples, expected);
            assert_eq!((decoded.channels, decoded.sample_rate), (1, 44100));
            assert!(Audio::decode_wave(&bytes, 2).is_err());
            for end in 0..bytes.len() {
                assert!(Audio::decode_wave(&bytes[..end], 3).is_err());
            }
        }
        assert!(Audio::decode_wave(&wave(16, &[0]), 3).is_err());
        let mut bytes = wave(16, &[0, 0]);
        bytes[32] = 1; // invalid block alignment
        assert!(Audio::decode_wave(&bytes, 3).is_err());
        bytes[32] = 2;
        bytes[20] = 3; // IEEE float is not integer PCM
        assert!(Audio::decode_wave(&bytes, 3).is_err());
    }
    #[test]
    fn wave_chunk_order_padding_and_duplicates() {
        let bytes = wave(8, &[128]);
        let mut reordered = bytes[..12].to_vec();
        reordered.extend(&bytes[36..]);
        reordered.extend(b"JUNK");
        reordered.extend(1u32.to_le_bytes());
        reordered.extend([23, 0]);
        reordered.extend(&bytes[12..36]);
        let size = (reordered.len() as u32 - 8).to_le_bytes();
        reordered[4..8].copy_from_slice(&size);
        assert_eq!(Audio::decode_wave(&reordered, 1).unwrap().samples, [0]);
        reordered.extend(&bytes[36..]);
        let size = (reordered.len() as u32 - 8).to_le_bytes();
        reordered[4..8].copy_from_slice(&size);
        assert!(Audio::decode_wave(&reordered, 2).is_err());
        let mut no_data = bytes[..36].to_vec();
        no_data[4..8].copy_from_slice(&28u32.to_le_bytes());
        assert!(Audio::decode_wave(&no_data, 3).is_err());
    }
    #[test]
    fn wave_stereo_scaling_extended_fmt_and_format_rejection() {
        for bits in [8u16, 16, 24, 32] {
            let values = [i16::MIN, 0, i16::MAX - 255, -256];
            let mut data = Vec::new();
            for value in values {
                if bits == 8 {
                    data.push(((value as i32 >> 8) + 128) as u8);
                } else {
                    let sample = (value as i32) << (bits - 16);
                    data.extend_from_slice(&sample.to_le_bytes()[..(bits / 8) as usize]);
                }
            }
            let mut bytes = wave(bits, &data);
            bytes[22..24].copy_from_slice(&2u16.to_le_bytes());
            bytes[28..32].copy_from_slice(&(44100 * 2 * u32::from(bits / 8)).to_le_bytes());
            bytes[32..34].copy_from_slice(&(2 * (bits / 8)).to_le_bytes());
            // A PCM fmt extension is ignored by the compatibility adapter.
            bytes.splice(36..36, [0u8; 6]);
            bytes[16..20].copy_from_slice(&22u32.to_le_bytes());
            let length = bytes.len() as u32 - 8;
            bytes[4..8].copy_from_slice(&length.to_le_bytes());
            let decoded = Audio::decode_wave(&bytes, 2).unwrap();
            assert_eq!(decoded.channels, 2);
            assert_eq!(decoded.samples, values);
            assert!(Audio::decode_wave(&bytes, 1).is_err());

            let mut duplicate = bytes.clone();
            duplicate.extend_from_slice(&bytes[12..42]);
            let length = duplicate.len() as u32 - 8;
            duplicate[4..8].copy_from_slice(&length.to_le_bytes());
            assert!(Audio::decode_wave(&duplicate, 2).is_err());
        }
        for channels in [0u16, 9, u16::MAX] {
            let mut bytes = wave(16, &[]);
            bytes[22..24].copy_from_slice(&channels.to_le_bytes());
            assert!(Audio::decode_wave(&bytes, 0).is_err());
        }
        for rate in [0u32, 768_001] {
            let mut bytes = wave(16, &[]);
            bytes[24..28].copy_from_slice(&rate.to_le_bytes());
            bytes[28..32].copy_from_slice(&(rate * 2).to_le_bytes());
            assert!(Audio::decode_wave(&bytes, 0).is_err());
        }
        // Matching average byte rate does not excuse overpadded samples.
        let mut bytes = wave(16, &[0; 4]);
        bytes[32..34].copy_from_slice(&4u16.to_le_bytes());
        bytes[28..32].copy_from_slice(&(44100u32 * 4).to_le_bytes());
        assert!(Audio::decode_wave(&bytes, 2).is_err());
    }

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
    fn compressed_tlg5_overlap_and_bounds() {
        let mut bytes = b"TLG5.0\0raw\x1a\x03".to_vec();
        for n in [4u32, 1, 1, 0] {
            bytes.extend(n.to_le_bytes());
        }
        for _ in 0..3 {
            bytes.push(0);
            bytes.extend(4u32.to_le_bytes());
            bytes.extend([2, 1, 0, 0]); // literal 1, then overlapping three-byte copy
        }
        assert_eq!(
            Image::decode(&bytes).unwrap().rgba,
            [2, 1, 2, 255, 4, 2, 4, 255, 6, 3, 6, 255, 8, 4, 8, 255]
        );
        assert!(validate_slide(&[1, 0, 0xf0, 255], 10).is_err());
        assert!(validate_slide(&[0, 1], 4).is_err());
        for end in 0..bytes.len() {
            assert!(Image::decode(&bytes[..end]).is_err());
        }
    }
}
