use crate::binary::Reader;
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
pub(super) fn metadata(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut tags = BTreeMap::new();
    if bytes.starts_with(b"TLG0.0\0sds\x1a") {
        let mut r = Reader::new(&bytes[11..]);
        let raw = r.u32()? as usize;
        r.take(raw)?;
        while !r.done() {
            let kind = r.take(4)?;
            let size = r.u32()? as usize;
            let mut data = r.take(size)?;
            if kind != b"tags" {
                continue;
            }
            while !data.is_empty() {
                let key = tag_string(&mut data)?;
                ensure!(data.first() == Some(&b'='), "invalid TLG tag separator");
                data = &data[1..];
                let value = tag_string(&mut data)?;
                ensure!(data.first() == Some(&b','), "invalid TLG tag terminator");
                data = &data[1..];
                ensure!(tags.len() < 4096, "image metadata limit exceeded");
                tags.insert(key, value);
            }
        }
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut r = Reader::new(&bytes[8..]);
        while !r.done() {
            let size = u32::from_be_bytes(r.take(4)?.try_into()?) as usize;
            let kind = r.take(4)?;
            let data = r.take(size)?;
            r.take(4)?; // CRC is validated by the PNG decoder.
            if matches!(kind, b"oFFs" | b"pHYs") || kind.eq_ignore_ascii_case(b"vpAg") {
                ensure!(data.len() >= 9, "truncated PNG metadata");
                let (a, b, unit) = if kind == b"oFFs" {
                    ("offs_x", "offs_y", "offs_unit")
                } else if kind == b"pHYs" {
                    ("reso_x", "reso_y", "reso_unit")
                } else {
                    ("vpag_w", "vpag_h", "vpag_unit")
                };
                tags.insert(
                    a.into(),
                    i32::from_be_bytes(data[..4].try_into()?).to_string(),
                );
                tags.insert(
                    b.into(),
                    i32::from_be_bytes(data[4..8].try_into()?).to_string(),
                );
                tags.insert(
                    unit.into(),
                    if kind == b"pHYs" {
                        if data[8] == 1 { "meter" } else { "unknown" }
                    } else {
                        match data[8] {
                            0 => "pixel",
                            1 => "micrometer",
                            _ => "unknown",
                        }
                    }
                    .into(),
                );
            }
            if kind == b"IEND" {
                break;
            }
        }
    }
    Ok(tags)
}
fn tag_string(data: &mut &[u8]) -> Result<String> {
    let end = data
        .iter()
        .position(|b| *b == b':')
        .context("invalid TLG tag length")?;
    let len: usize = std::str::from_utf8(&data[..end])?.parse()?;
    *data = &data[end + 1..];
    ensure!(
        len <= data.len() && len <= 1024 * 1024,
        "TLG tag length exceeds limit"
    );
    let value = std::str::from_utf8(&data[..len])?.to_owned();
    *data = &data[len..];
    Ok(value)
}
