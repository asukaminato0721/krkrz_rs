//! Kirikiri TFT v0/v1 indices and 65-level run-length glyph coverage.
//! Format reference: visual/PrerenderedFont.{h,cpp} at the pinned krkrz revision.
use super::TextGlyph;
use anyhow::{Context, Result, ensure};

pub(super) struct Item {
    offset: usize,
    width: usize,
    height: usize,
    origin: [i32; 2],
    increments: [i32; 2],
    pub advance: i32,
}

pub(super) struct PrerenderedFont {
    data: Vec<u8>,
    version: u8,
    characters: Vec<u16>,
    items: Vec<Item>,
    pub hash: [u8; 32],
}

impl PrerenderedFont {
    pub fn parse(data: Vec<u8>, hash: [u8; 32]) -> Result<Self> {
        ensure!(data.len() >= 36, "truncated prerendered font header");
        ensure!(
            &data[..22] == b"TVP pre-rendered font\x1a",
            "invalid prerendered font signature"
        );
        let version = data[22];
        ensure!(version <= 1, "unsupported prerendered font version");
        ensure!(data[23] == 2, "prerendered font must use UTF-16");
        let word =
            |offset| u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        let count = word(24);
        ensure!(
            count <= 65536,
            "prerendered font character count exceeds limit"
        );
        let table = |offset: usize, stride: usize| -> Result<&[u8]> {
            ensure!(offset >= 36, "invalid prerendered font table offset");
            data.get(
                offset
                    ..offset
                        .checked_add(count * stride)
                        .context("prerendered font table overflow")?,
            )
            .context("truncated prerendered font table")
        };
        let characters: Vec<u16> = table(word(28), 2)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| u16::from_le_bytes(*bytes))
            .collect();
        ensure!(
            characters.windows(2).all(|pair| pair[0] < pair[1]),
            "prerendered font characters must be sorted and unique"
        );
        let items = table(word(32), 20)?
            .as_chunks::<20>()
            .0
            .iter()
            .map(|bytes| {
                let unsigned =
                    |offset| u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
                let signed = |offset| unsigned(offset) as i16 as i32;
                let item = Item {
                    offset: u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
                    width: unsigned(4) as usize,
                    height: unsigned(6) as usize,
                    origin: [signed(8), signed(10)],
                    increments: [signed(12), signed(14)],
                    advance: signed(16),
                };
                ensure!(
                    item.width * item.height <= 16_000_000,
                    "prerendered glyph exceeds size limit"
                );
                ensure!(
                    item.width == 0 || item.height == 0 || (36..data.len()).contains(&item.offset),
                    "invalid prerendered glyph offset"
                );
                Ok(item)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            data,
            version,
            characters,
            items,
            hash,
        })
    }

    pub fn byte_len(&self) -> usize {
        self.data.len()
    }

    pub fn find(&self, unit: u16) -> Option<&Item> {
        self.characters
            .binary_search(&unit)
            .ok()
            .map(|index| &self.items[index])
    }

    pub fn glyph(&self, item: &Item) -> Result<TextGlyph> {
        let area = item.width * item.height;
        let mut coverage = Vec::with_capacity(area);
        let mut cursor = item.offset;
        while coverage.len() < area {
            let value = *self
                .data
                .get(cursor)
                .context("truncated prerendered glyph data")?;
            cursor += 1;
            if value <= 64 {
                coverage.push(value);
            } else {
                let count = if self.version == 0 {
                    ensure!(value == 65, "invalid prerendered glyph coverage");
                    let count = *self
                        .data
                        .get(cursor)
                        .context("truncated prerendered glyph run")?
                        as usize;
                    cursor += 1;
                    count
                } else {
                    (value - 64) as usize
                };
                let previous = *coverage
                    .last()
                    .context("prerendered glyph run has no preceding pixel")?;
                ensure!(
                    count <= area - coverage.len(),
                    "prerendered glyph run exceeds bitmap"
                );
                coverage.resize(coverage.len() + count, previous);
            }
        }
        Ok(TextGlyph {
            left: item.origin[0],
            top: -item.origin[1],
            width: item.width,
            height: item.height,
            advance: item.increments,
            coverage,
        })
    }
}
