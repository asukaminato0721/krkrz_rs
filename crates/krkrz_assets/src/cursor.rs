//! Static Windows CUR files, including their pointer hotspots.
use crate::media::Image;
use anyhow::{Context, Result, ensure};

#[derive(Clone, Debug, PartialEq)]
pub struct CursorImage {
    image: Image,
    hotspot: [u16; 2],
}
impl CursorImage {
    pub fn image(&self) -> &Image {
        &self.image
    }
    pub fn hotspot(&self) -> [u16; 2] {
        self.hotspot
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= 16 * 1024 * 1024, "cursor exceeds 16 MiB");
        ensure!(
            bytes.get(..4) == Some(&[0, 0, 2, 0]),
            "unsupported cursor format (expected static CUR)"
        );
        let word = |offset: usize| -> Result<u16> {
            Ok(u16::from_le_bytes(
                bytes
                    .get(offset..offset + 2)
                    .context("truncated cursor header")?
                    .try_into()?,
            ))
        };
        let count = usize::from(word(4)?);
        ensure!((1..=256).contains(&count), "invalid cursor image count");
        let directory_end = 6 + count * 16;
        let entries = bytes
            .get(6..directory_end)
            .context("truncated cursor directory")?;
        // Match the legacy 32-pixel system cursor size. A directory byte of
        // zero represents 256 pixels, as in ICO files.
        let size = |value: u8| if value == 0 { 256u16 } else { u16::from(value) };
        let entry = entries
            .as_chunks::<16>()
            .0
            .iter()
            .min_by_key(|entry| size(entry[0]).abs_diff(32) + size(entry[1]).abs_diff(32))
            .context("empty cursor directory")?;
        let width = size(entry[0]);
        let height = size(entry[1]);
        let hotspot = [
            u16::from_le_bytes(entry[4..6].try_into()?),
            u16::from_le_bytes(entry[6..8].try_into()?),
        ];
        ensure!(
            hotspot[0] < width && hotspot[1] < height,
            "cursor hotspot is outside the image"
        );
        let length = u32::from_le_bytes(entry[8..12].try_into()?) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into()?) as usize;
        ensure!(
            offset >= directory_end,
            "cursor image overlaps the directory"
        );
        let end = offset
            .checked_add(length)
            .context("cursor image range overflow")?;
        let data = bytes.get(offset..end).context("truncated cursor image")?;
        // CUR and ICO share the same image data. Replace only the directory
        // fields that hold a hotspot in CUR and a plane/depth pair in ICO.
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend_from_slice(entry);
        ico[10..14].copy_from_slice(&[1, 0, 0, 0]);
        ico[18..22].copy_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(data);
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(ico), image::ImageFormat::Ico);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(u32::from(width));
        limits.max_image_height = Some(u32::from(height));
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader.decode().context("decode cursor image")?.into_rgba8();
        let image = Image {
            width: decoded.width(),
            height: decoded.height(),
            rgba: decoded.into_raw(),
        };
        ensure!(
            image.width == u32::from(width) && image.height == u32::from(height),
            "cursor directory dimensions do not match the image"
        );
        Ok(Self { image, hotspot })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cursor() -> Vec<u8> {
        let mut bytes = vec![0, 0, 2, 0, 1, 0, 2, 2, 0, 0, 1, 0, 0, 0];
        bytes.extend_from_slice(&64u32.to_le_bytes());
        bytes.extend_from_slice(&22u32.to_le_bytes());
        let mut dib = vec![0; 64];
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2u32.to_le_bytes());
        dib[8..12].copy_from_slice(&4u32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[40..56].copy_from_slice(&[
            0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 0, 255, 255, 255,
        ]);
        dib[60] = 0x80; // The upper-left pixel is transparent in the AND mask.
        bytes.extend(dib);
        bytes
    }
    #[test]
    fn cur_retains_hotspot_color_orientation_and_transparency() {
        let cursor = CursorImage::decode(&cursor()).unwrap();
        assert_eq!(cursor.hotspot(), [1, 0]);
        let image = cursor.image();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(
            &image.rgba[4..],
            &[255, 255, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255]
        );
        assert_eq!(image.rgba[3], 0);
    }
    #[test]
    fn malformed_directories_hotspots_and_image_ranges_are_rejected() {
        for (offset, value) in [(2, 1), (4, 0), (10, 2), (18, 0), (14, 255)] {
            let mut bytes = cursor();
            bytes[offset] = value;
            assert!(CursorImage::decode(&bytes).is_err(), "offset {offset}");
        }
        for length in [0, 4, 5, 6, 21, 40, 85] {
            assert!(CursorImage::decode(&cursor()[..length]).is_err());
        }
    }
}
