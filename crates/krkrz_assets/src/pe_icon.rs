//! Read a Windows executable's application icon without loading executable code.
//! Pelite handles PE32/PE32+ resources; the existing image decoder handles ICO.
use crate::media::Image;
use anyhow::{Context, Result, ensure};

pub const MAX_EXECUTABLE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ICON_BYTES: usize = 16 * 1024 * 1024;

/// Extract the application's icon. Kirikiri 2 uses MAINICON; Kirikiri Z uses
/// resource 107. Other executables fall back to their first group icon.
pub fn decode(bytes: &[u8]) -> Result<Option<Image>> {
    ensure!(
        bytes.len() <= MAX_EXECUTABLE_BYTES,
        "icon executable exceeds 64 MiB"
    );
    let pe = pelite::PeFile::from_bytes(bytes).context("parse icon executable")?;
    let resources = match pe.resources() {
        Ok(resources) => resources,
        Err(pelite::Error::Null) => return Ok(None),
        Err(error) => return Err(error).context("read executable resources"),
    };
    let mut groups = resources.icons().take(65).collect::<Vec<_>>();
    ensure!(groups.len() <= 64, "executable has too many icon groups");
    groups.sort_by_key(|group| match group {
        Ok((name, _)) if *name == *"MAINICON" => 0,
        Ok((name, _)) if *name == 107u32 => 1,
        _ => 2,
    });
    let Some(group) = groups.first() else {
        return Ok(None);
    };
    let (_, group) = group
        .as_ref()
        .map_err(|error| anyhow::anyhow!("read icon group: {error}"))?;
    let entries = group.entries();
    ensure!(
        !entries.is_empty() && entries.len() <= 64,
        "invalid icon image count"
    );
    let mut length = 6 + entries.len() * 16;
    for entry in entries {
        let data = group.image(entry.nId).context("read icon image resource")?;
        ensure!(
            data.len() == entry.bytes_in_resource() as usize,
            "icon resource size mismatch"
        );
        length = length
            .checked_add(data.len())
            .context("icon resource size overflow")?;
        ensure!(length <= MAX_ICON_BYTES, "executable icon exceeds 16 MiB");
    }
    let mut ico = Vec::with_capacity(length);
    group
        .write(&mut ico)
        .context("reassemble executable icon")?;
    let image = Image::decode(&ico).context("decode executable icon")?;
    ensure!(
        image.width <= 256 && image.height <= 256,
        "executable icon exceeds 256 pixels"
    );
    Ok(Some(image))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Minimal PE containing a two-pixel-square 32-bit DIB and icon group 107.
    fn executable(pe64: bool) -> Vec<u8> {
        let mut bytes = vec![0; 1024];
        bytes[..2].copy_from_slice(b"MZ");
        put32(&mut bytes, 60, 128);
        bytes[128..132].copy_from_slice(b"PE\0\0");
        put16(&mut bytes, 132, if pe64 { 0x8664 } else { 0x14c });
        put16(&mut bytes, 134, 1);
        let optional_size = if pe64 { 240 } else { 224 };
        put16(&mut bytes, 148, optional_size);
        put16(&mut bytes, 152, if pe64 { 0x20b } else { 0x10b });
        put32(&mut bytes, 152 + 32, 4096);
        put32(&mut bytes, 152 + 36, 512);
        put32(&mut bytes, 152 + 56, 8192);
        put32(&mut bytes, 152 + 60, 512);
        let dirs = 152 + if pe64 { 112 } else { 96 };
        put32(&mut bytes, dirs - 4, 16);
        put32(&mut bytes, dirs + 16, 4096);
        put32(&mut bytes, dirs + 20, 244);
        let section = 152 + optional_size as usize;
        bytes[section..section + 5].copy_from_slice(b".rsrc");
        for (offset, value) in [(8, 244), (12, 4096), (16, 512), (20, 512)] {
            put32(&mut bytes, section + offset, value);
        }
        let r = &mut bytes[512..];
        put16(r, 14, 2);
        for (offset, id, child) in [(16, 3, 32), (24, 14, 56), (48, 1, 80), (72, 107, 104)] {
            put32(r, offset, id);
            put32(r, offset + 4, 0x8000_0000 | child);
        }
        for offset in [32, 56, 80, 104] {
            put16(r, offset + 14, 1);
        }
        for (offset, entry) in [(96, 128), (120, 144)] {
            put32(r, offset, 1033);
            put32(r, offset + 4, entry);
        }
        put32(r, 128, 4096 + 160);
        put32(r, 132, 64);
        put32(r, 144, 4096 + 224);
        put32(r, 148, 20);
        let dib = &mut r[160..224];
        put32(dib, 0, 40);
        put32(dib, 4, 2);
        put32(dib, 8, 4);
        put16(dib, 12, 1);
        put16(dib, 14, 32);
        for pixel in dib[40..56].as_chunks_mut::<4>().0 {
            *pixel = [0x33, 0x66, 0x99, 0xff];
        }
        let group = &mut r[224..244];
        put16(group, 2, 1);
        put16(group, 4, 1);
        group[6] = 2;
        group[7] = 2;
        put16(group, 10, 1);
        put16(group, 12, 32);
        put32(group, 14, 64);
        put16(group, 18, 1);
        bytes
    }

    #[test]
    fn extracts_pe32_and_pe64_group_icons() {
        for pe64 in [false, true] {
            let image = decode(&executable(pe64)).unwrap().unwrap();
            assert_eq!((image.width, image.height), (2, 2));
            assert_eq!(image.rgba, [0x99, 0x66, 0x33, 0xff].repeat(4));
        }
    }

    #[test]
    fn missing_and_malformed_resources() {
        assert!(decode(b"not an executable").is_err());
        let mut missing = executable(false);
        put32(&mut missing, 152 + 96 + 16, 0);
        put32(&mut missing, 152 + 96 + 20, 0);
        assert_eq!(decode(&missing).unwrap(), None);
        let mut malformed = executable(false);
        put32(&mut malformed, 512 + 224 + 14, u32::MAX);
        assert!(decode(&malformed).is_err());
        for length in [0, 64, 128, 256, 512, 700] {
            assert!(decode(&executable(false)[..length]).is_err());
        }
    }

    #[test]
    #[ignore = "requires installed game and KRKRZ_PROJECT_DIR"]
    fn installed_game_icon() {
        let project = std::path::PathBuf::from(std::env::var_os("KRKRZ_PROJECT_DIR").unwrap());
        let bytes = std::fs::read(project.join("otomedomain.exe")).unwrap();
        let image = decode(&bytes).unwrap().expect("game application icon");
        assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
        eprintln!("game icon: {}x{}", image.width, image.height);
        if let Some(path) = std::env::var_os("KRKRZ_ICON_CAPTURE") {
            image.write_png(std::path::Path::new(&path)).unwrap();
        }
    }
}
