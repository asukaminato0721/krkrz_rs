use crate::{binary::Reader, xp3::inflate};
use anyhow::{Result, ensure};
/// Decode BOM text, Kirikiri simple/compressed text, or unmarked UTF-8/Shift-JIS.
/// Unmarked encoding detection is a tooling convenience, not a locale implementation.
pub fn decode(bytes: &[u8]) -> Result<String> {
    if let Some(data) = bytes.strip_prefix(&[0xfe, 0xfe]) {
        let mut r = Reader::new(data);
        let mode = r.take(1)?[0];
        ensure!(mode <= 2, "unsupported Kirikiri text cipher mode {mode}");
        ensure!(
            r.take(2)? == [0xff, 0xfe],
            "missing UTF-16 BOM in Kirikiri text stream"
        );
        let bytes = if mode == 2 {
            let packed = usize::try_from(r.u64()?)?;
            let raw = r.u64()?;
            let decoded = inflate(r.take(packed)?, raw, 64 << 20)?;
            ensure!(r.done(), "trailing compressed text bytes");
            decoded
        } else {
            r.take(data.len() - r.pos)?.to_vec()
        };
        ensure!(bytes.len().is_multiple_of(2), "odd UTF-16 text length");
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                let n = u16::from_le_bytes(*b);
                match mode {
                    0 if n >= 0x20 => n ^ (((n & 0xfe) << 8) ^ 1),
                    1 => ((n & 0xaaaa) >> 1) | ((n & 0x5555) << 1),
                    _ => n,
                }
            })
            .collect::<Vec<_>>();
        return Ok(String::from_utf16(&units)?);
    }
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        ensure!(bytes.len().is_multiple_of(2), "odd UTF-16 text length");
        let le = bytes[0] == 0xff;
        let units = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                if le {
                    u16::from_le_bytes(*b)
                } else {
                    u16::from_be_bytes(*b)
                }
            })
            .collect::<Vec<_>>();
        return Ok(String::from_utf16(&units)?);
    }
    if let Some(bytes) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return Ok(std::str::from_utf8(bytes)?.to_owned());
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_owned());
    }
    let (text, errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(bytes);
    ensure!(!errors, "invalid Shift-JIS source text");
    Ok(text.into_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn all_kirikiri_text_modes() {
        let text = "日本語\nabc";
        let units = text.encode_utf16().collect::<Vec<_>>();
        for mode in 0..=2 {
            let mut bytes = vec![0xfe, 0xfe, mode, 0xff, 0xfe];
            let mut raw = vec![];
            for &n in &units {
                let encoded = match mode {
                    0 if n >= 0x20 => n ^ (((n & 0xfe) << 8) ^ 1),
                    1 => ((n & 0xaaaa) >> 1) | ((n & 0x5555) << 1),
                    _ => n,
                };
                raw.extend(encoded.to_le_bytes());
            }
            if mode == 2 {
                let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::default());
                z.write_all(&raw).unwrap();
                let packed = z.finish().unwrap();
                bytes.extend((packed.len() as u64).to_le_bytes());
                bytes.extend((raw.len() as u64).to_le_bytes());
                bytes.extend(packed);
            } else {
                bytes.extend(raw);
            }
            assert_eq!(decode(&bytes).unwrap(), text);
        }
    }
    #[test]
    fn malformed_modes() {
        for bytes in [
            &[0xfe, 0xfe][..],
            &[0xfe, 0xfe, 3, 0xff, 0xfe],
            &[0xff, 0xfe, 0x41],
        ] {
            assert!(decode(bytes).is_err());
        }
    }
}
