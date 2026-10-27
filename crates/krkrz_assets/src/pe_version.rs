//! Read the original executable's fixed file version without executing it.
use std::{io::Read, path::Path};

pub fn decode(bytes: &[u8]) -> Option<[u16; 4]> {
    if bytes.len() > crate::pe_icon::MAX_EXECUTABLE_BYTES {
        return None;
    }
    let pe = pelite::PeFile::from_bytes(bytes).ok()?;
    let resources = pe.resources().ok()?;
    let info = resources.version_info().ok()?;
    let version = info.fixed()?.dwFileVersion;
    Some([version.Major, version.Minor, version.Patch, version.Build])
}

/// Missing, invalid and oversized optional metadata uses the host's default version.
pub fn read(path: &Path) -> Option<[u16; 4]> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(crate::pe_icon::MAX_EXECUTABLE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    decode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn executable(pe64: bool, version: [u16; 4]) -> Vec<u8> {
        let mut bytes = crate::pe_icon::tests::executable(pe64);
        let r = &mut bytes[512..];
        r.fill(0);
        for offset in [14, 38, 62] {
            r[offset..offset + 2].copy_from_slice(&1u16.to_le_bytes());
        }
        for (offset, value) in [
            (16, 16u32),
            (20, 0x80000018),
            (40, 1),
            (44, 0x80000030),
            (64, 1033),
            (68, 72),
            (72, 4096 + 88),
            (76, 92),
        ] {
            r[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let data = &mut r[88..180];
        data[..2].copy_from_slice(&92u16.to_le_bytes());
        data[2..4].copy_from_slice(&52u16.to_le_bytes());
        for (i, unit) in "VS_VERSION_INFO\0".encode_utf16().enumerate() {
            data[6 + i * 2..8 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
        data[40..44].copy_from_slice(&0xfeef04bdu32.to_le_bytes());
        data[44..48].copy_from_slice(&0x10000u32.to_le_bytes());
        let high = (version[0] as u32) << 16 | version[1] as u32;
        let low = (version[2] as u32) << 16 | version[3] as u32;
        data[48..52].copy_from_slice(&high.to_le_bytes());
        data[52..56].copy_from_slice(&low.to_le_bytes());
        bytes
    }

    #[test]
    fn reads_fixed_version_from_pe32_and_pe64() {
        for pe64 in [false, true] {
            let expected = [2, 11, 19, 13];
            assert_eq!(decode(&executable(pe64, expected)), Some(expected));
        }
    }

    #[test]
    fn invalid_or_missing_version_is_optional() {
        assert_eq!(decode(b"not a PE file"), None);
        assert_eq!(decode(&crate::pe_icon::tests::executable(false)), None);
        let bytes = executable(false, [2, 11, 19, 13]);
        for size in [0, 64, 128, 512, 700] {
            assert_eq!(decode(&bytes[..size]), None);
        }
    }
}
