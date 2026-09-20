use crate::{binary::Reader, xp3::inflate};
use anyhow::{Result, ensure};
/// Decode BOM text, Kirikiri simple/compressed text, or unmarked UTF-8/Shift-JIS.
/// Unmarked encoding detection is a tooling convenience, not a locale implementation.
pub fn decode(bytes: &[u8]) -> Result<String> {
    Ok(String::from_utf16(&decode_units(bytes)?)?)
}

/// Text streams retain UTF-16 code units, including isolated surrogates.
pub fn decode_units(bytes: &[u8]) -> Result<Vec<u16>> {
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
        return Ok(units);
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
        return Ok(units);
    }
    if let Some(bytes) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return Ok(std::str::from_utf8(bytes)?.encode_utf16().collect());
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.encode_utf16().collect());
    }
    let (text, errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(bytes);
    ensure!(!errors, "invalid Shift-JIS source text");
    Ok(text.encode_utf16().collect())
}

/// Encode the native UTF-16 text stream, with optional cipher/compression.
/// Binary offsets are handled by the storage writer, not this encoder.
pub fn encode_units(units: &[u16], mode: &str) -> Result<Vec<u8>> {
    use std::io::Write;
    ensure!(units.len() <= 32 << 20, "text stream exceeds 64 MiB limit");
    let mut cipher = 0;
    let mut level = 6;
    let mut chars = mode.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            'c' => {
                let n = chars.peek().and_then(|v| v.to_digit(10));
                if n.is_some() {
                    chars.next();
                }
                cipher = n.unwrap_or(1);
                ensure!(cipher == 1 || cipher == 2, "unsupported text cipher mode");
            }
            'z' => {
                cipher = 2;
                if let Some(n) = chars.peek().and_then(|v| v.to_digit(10)) {
                    level = n;
                    chars.next();
                }
            }
            _ => anyhow::bail!("unsupported text write mode: {mode}"),
        }
    }
    let mut bytes = if cipher == 0 {
        vec![]
    } else {
        vec![0xfe, 0xfe, cipher as u8]
    };
    bytes.extend([0xff, 0xfe]);
    let raw: Vec<_> = units
        .iter()
        .flat_map(|&n| {
            let n = if cipher == 1 {
                ((n & 0xaaaa) >> 1) | ((n & 0x5555) << 1)
            } else {
                n
            };
            n.to_le_bytes()
        })
        .collect();
    if cipher == 2 {
        let mut writer =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(level));
        writer.write_all(&raw)?;
        let compressed = writer.finish()?;
        bytes.extend((compressed.len() as u64).to_le_bytes());
        bytes.extend((raw.len() as u64).to_le_bytes());
        bytes.extend(compressed);
    } else {
        bytes.extend(raw);
    }
    Ok(bytes)
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

    #[test]
    fn encoded_streams_preserve_all_utf16_units() {
        let units = [0, 7, 10, 13, 0x20, 0x65e5, 0xd800, 0xdc00, 0xdfff, 0xffff];
        for mode in ["", "c", "c1", "c2", "z", "z0", "z9"] {
            let bytes = encode_units(&units, mode).unwrap();
            assert_eq!(decode_units(&bytes).unwrap(), units, "{mode}");
        }
        // A malformed compressed stream must not be accepted as plain text.
        let mut bytes = encode_units(&units, "z").unwrap();
        bytes.pop();
        assert!(decode_units(&bytes).is_err());
        for mode in ["c0", "c9", "x", "z10"] {
            assert!(encode_units(&units, mode).is_err(), "{mode}");
        }
    }
}
