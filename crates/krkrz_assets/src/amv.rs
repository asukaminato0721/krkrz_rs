//! Checked AJPM (AlphaMovie) container access.
//!
//! Format research: xmoezzz/amv_decoder (see docs/references.json). The stream
//! consists of rectangle packets, including empty packets and repeated sequence
//! numbers. A packet is not necessarily a displayed frame. Entropy decoding and
//! presentation remain separate from this index.
mod decode;
mod idct;
mod tables;
use crate::binary::Reader;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::ops::Range;

const MAX_PIXELS: usize = 64 * 1024 * 1024;
const MAX_PACKETS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum AlphaEncoding {
    Dct,
    Deflate,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Header {
    pub revision: u32,
    pub header_size: u32,
    pub frame_count: u32,
    pub fps_scale: u32,
    pub fps_rate: u32,
    pub width: u16,
    pub height: u16,
    pub attributes: u8,
    pub alpha_encoding: AlphaEncoding,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Packet {
    pub offset: usize,
    pub sequence: u32,
    pub left: i16,
    pub top: i16,
    pub width: u16,
    pub height: u16,
    /// Compressed alpha plane for the DEFLATE variant.
    pub alpha: Option<Range<usize>>,
    /// JPEG-like entropy data; this is not an independent JPEG file.
    pub entropy: Range<usize>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Movie {
    pub header: Header,
    /// Natural-order 8x8 quantization tables: luma, chroma, optional alpha.
    pub quantization: Vec<Vec<u8>>,
    pub packets: Vec<Packet>,
    #[serde(skip)]
    data: Vec<u8>,
}
impl Movie {
    /// Takes ownership of a complete resource. Packet offsets refer to these
    /// bytes, so subsequent seeks do not allocate or duplicate payloads.
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        let mut r = Reader::new(&data);
        ensure!(r.take(4)? == b"AJPM", "invalid AlphaMovie signature");
        let file_size = r.u32()? as usize;
        ensure!(file_size == data.len(), "AlphaMovie file size mismatch");
        let revision = r.u32()?;
        ensure!(revision == 0, "unsupported AlphaMovie revision {revision}");
        let header_size = r.u32()?;
        r.take(4)?; // Encoder metadata; not interpreted.
        let frame_count = r.u32()?;
        let fps_scale = r.u32()?;
        let fps_rate = r.u32()?;
        let width = r.u16()?;
        let height = r.u16()?;
        let attributes = r.take(1)?[0];
        r.take(3)?;
        ensure!(frame_count > 0, "AlphaMovie has no frames");
        ensure!(
            fps_scale > 0 && fps_rate > 0,
            "invalid AlphaMovie frame rate"
        );
        ensure!(
            width > 0 && height > 0 && width as usize * height as usize <= MAX_PIXELS,
            "invalid AlphaMovie canvas dimensions"
        );
        let alpha_encoding = if attributes & 1 != 0 {
            AlphaEncoding::Dct
        } else {
            ensure!(attributes & 2 != 0, "invalid AlphaMovie attributes");
            AlphaEncoding::Deflate
        };
        let tables = if alpha_encoding == AlphaEncoding::Dct {
            3
        } else {
            2
        };
        ensure!(
            header_size as usize == 40 + tables * 64,
            "invalid AlphaMovie quantization table size"
        );
        let quantization = (0..tables)
            .map(|_| Ok(r.take(64)?.to_vec()))
            .collect::<Result<Vec<_>>>()?;
        let mut packets = Vec::new();
        while !r.done() {
            ensure!(
                packets.len() < MAX_PACKETS,
                "AlphaMovie packet limit exceeded"
            );
            let offset = r.pos;
            ensure!(
                r.take(4)? == b"FRAM",
                "invalid AlphaMovie packet tag at {offset:#x}"
            );
            let length = r.u32()? as usize;
            let base = r.pos;
            let mut packet = Reader::new(
                r.take(length)
                    .with_context(|| format!("AlphaMovie packet at {offset:#x}"))?,
            );
            let sequence = packet.u32()?;
            let left = packet.u16()? as i16;
            let top = packet.u16()? as i16;
            let width = packet.u16()?;
            let height = packet.u16()?;
            ensure!(
                width.is_multiple_of(16) && height.is_multiple_of(16),
                "AlphaMovie rectangle is not 16-pixel aligned"
            );
            ensure!(
                (width == 0) == (height == 0),
                "invalid empty AlphaMovie rectangle"
            );
            ensure!(
                width as usize * height as usize <= MAX_PIXELS,
                "AlphaMovie rectangle exceeds pixel limit"
            );
            let alpha = if alpha_encoding == AlphaEncoding::Deflate {
                let len = packet.u32()? as usize;
                let start = base + packet.pos;
                packet.take(len)?;
                Some(start..base + packet.pos)
            } else {
                None
            };
            let entropy = base + packet.pos..base + length;
            packets.push(Packet {
                offset,
                sequence,
                left,
                top,
                width,
                height,
                alpha,
                entropy,
            });
        }
        ensure!(!packets.is_empty(), "AlphaMovie has no packets");
        Ok(Self {
            header: Header {
                revision,
                header_size,
                frame_count,
                fps_scale,
                fps_rate,
                width,
                height,
                attributes,
                alpha_encoding,
            },
            quantization,
            packets,
            data,
        })
    }
    pub fn entropy(&self, packet: usize) -> Result<&[u8]> {
        let packet = self
            .packets
            .get(packet)
            .context("AlphaMovie packet index out of range")?;
        self.data
            .get(packet.entropy.clone())
            .context("invalid AlphaMovie entropy range")
    }
    pub fn compressed_alpha(&self, packet: usize) -> Result<Option<&[u8]>> {
        let packet = self
            .packets
            .get(packet)
            .context("AlphaMovie packet index out of range")?;
        packet
            .alpha
            .as_ref()
            .map(|range| {
                self.data
                    .get(range.clone())
                    .context("invalid AlphaMovie alpha range")
            })
            .transpose()
    }
}
