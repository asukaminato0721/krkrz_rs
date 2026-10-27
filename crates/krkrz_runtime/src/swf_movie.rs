//! Embedded-video SWF timelines, including one static wrapper sprite.
//! Adobe SWF 19: RECT, MATRIX, PlaceObject2, DefineSprite, streaming sound,
//! DefineVideoStream, and VideoFrame. No ActionScript or vector rendering.
//! Flatten validated control tags because FFmpeg's SWF demuxer skips sprites.
use super::MAX_BYTES;
use anyhow::{Context, Result, bail, ensure};
use std::io::Read;

#[derive(Debug)]
pub(super) struct Timing {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub frames: u16,
    pub audio_start_ms: f64,
}

#[derive(Debug)]
pub(super) struct Movie {
    pub bytes: Vec<u8>,
    pub timing: Timing,
}

pub(super) fn detects(bytes: &[u8]) -> bool {
    matches!(bytes.get(..3), Some(b"FWS" | b"CWS" | b"ZWS"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Character(u16);

#[derive(Clone, Copy)]
struct Tag<'a> {
    code: u16,
    data: &'a [u8],
    encoded: &'a [u8],
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let (part, rest) = self.0.split_at_checked(count).context("truncated SWF")?;
        self.0 = rest;
        Ok(part)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn tags(mut self) -> Result<Vec<Tag<'a>>> {
        let mut tags = Vec::new();
        let mut ended = false;
        while !self.0.is_empty() && !ended {
            ensure!(tags.len() < 1_000_000, "SWF tag count exceeds limit");
            let start = self.0;
            let header = self.u16()?;
            let code = header >> 6;
            let short = usize::from(header & 63);
            let len = if short == 63 {
                self.u32()? as usize
            } else {
                short
            };
            let data = self.take(len)?;
            let encoded = &start[..start.len() - self.0.len()];
            ended = code == 0;
            ensure!(!ended || data.is_empty(), "invalid SWF End tag");
            tags.push(Tag {
                code,
                data,
                encoded,
            });
        }
        ensure!(ended && self.0.is_empty(), "invalid SWF timeline end");
        Ok(tags)
    }
}

struct Bits<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn unsigned(&mut self, count: u32) -> Result<u32> {
        ensure!(count <= 31, "invalid SWF bit width");
        let mut value = 0;
        for _ in 0..count {
            let byte = self
                .bytes
                .get(self.position / 8)
                .context("truncated SWF bits")?;
            value = (value << 1) | u32::from((byte >> (7 - self.position % 8)) & 1);
            self.position += 1;
        }
        Ok(value)
    }
    fn signed(&mut self, count: u32) -> Result<i32> {
        let value = self.unsigned(count)?;
        Ok(if count == 0 {
            0
        } else {
            ((value << (32 - count)) as i32) >> (32 - count)
        })
    }
    fn length(&self) -> usize {
        self.position.div_ceil(8)
    }
    fn identity_matrix(&mut self) -> Result<()> {
        if self.unsigned(1)? != 0 {
            let count = self.unsigned(5)?;
            ensure!(
                self.signed(count)? == 65536 && self.signed(count)? == 65536,
                "SWF video scaling is not supported"
            );
        }
        if self.unsigned(1)? != 0 {
            let count = self.unsigned(5)?;
            ensure!(
                self.signed(count)? == 0 && self.signed(count)? == 0,
                "SWF video rotation is not supported"
            );
        }
        let count = self.unsigned(5)?;
        ensure!(
            self.signed(count)? == 0 && self.signed(count)? == 0,
            "SWF video translation is not supported"
        );
        Ok(())
    }
}

struct Placement {
    character: Character,
    depth: u16,
    frame: u16,
}

fn place(data: &[u8], displayed: &mut Option<Placement>, expected: Character) -> Result<()> {
    let mut reader = Reader(data);
    let flags = reader.u8()?;
    ensure!(flags & !0x17 == 0, "unsupported SWF video placement flags");
    let depth = reader.u16()?;
    if flags & 2 != 0 {
        let character = Character(reader.u16()?);
        ensure!(character == expected, "unsupported SWF display character");
        ensure!(
            displayed.is_none(),
            "multiple SWF display characters are not supported"
        );
        *displayed = Some(Placement {
            character,
            depth,
            frame: 0,
        });
    } else {
        ensure!(flags & 1 != 0, "invalid SWF placement update");
    }
    let display = displayed
        .as_mut()
        .context("SWF updates an absent character")?;
    ensure!(
        display.depth == depth && display.character == expected,
        "unsupported SWF display depth"
    );
    if flags & 4 != 0 {
        let mut matrix = Bits::new(reader.0);
        matrix.identity_matrix()?;
        reader.take(matrix.length())?;
    }
    if flags & 16 != 0 {
        display.frame = reader.u16()?;
    }
    ensure!(reader.0.is_empty(), "unexpected SWF placement data");
    Ok(())
}

struct Definition<'a> {
    id: Character,
    frames: u16,
    width: u32,
    height: u32,
    tag: Tag<'a>,
}
impl<'a> Definition<'a> {
    fn parse(tag: Tag<'a>) -> Result<Self> {
        let mut reader = Reader(tag.data);
        let id = Character(reader.u16()?);
        let frames = reader.u16()?;
        let width = u32::from(reader.u16()?);
        let height = u32::from(reader.u16()?);
        let flags = reader.u8()?;
        let codec = reader.u8()?;
        ensure!(
            codec == 4,
            "unsupported SWF video codec {codec}: expected VP6"
        );
        ensure!(flags & 0xfe == 0, "unsupported SWF video deblocking flags");
        ensure!(
            reader.0.is_empty() && frames > 0 && width > 0 && height > 0,
            "invalid SWF video definition"
        );
        ensure!(
            u64::from(width) * u64::from(height) <= 16 * 1024 * 1024,
            "SWF video dimensions exceed limit"
        );
        Ok(Self {
            id,
            frames,
            width,
            height,
            tag,
        })
    }
}

impl Movie {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader(bytes);
        let signature = reader.take(3)?;
        let version = reader.u8()?;
        let length = reader.u32()? as usize;
        ensure!(
            (8..=MAX_BYTES).contains(&length),
            "SWF length exceeds limit"
        );
        ensure!(
            (8..=19).contains(&version),
            "unsupported SWF version {version}"
        );
        let body = match signature {
            b"FWS" => reader.0.to_vec(),
            b"CWS" => {
                let mut body = Vec::new();
                flate2::read::ZlibDecoder::new(reader.0)
                    .take((length - 8) as u64 + 1)
                    .read_to_end(&mut body)?;
                body
            }
            _ => bail!("unsupported SWF compression"),
        };
        ensure!(body.len() == length - 8, "SWF length does not match header");
        let mut rectangle = Bits::new(&body);
        let count = rectangle.unsigned(5)?;
        let x_min = rectangle.signed(count)?;
        let x_max = rectangle.signed(count)?;
        let y_min = rectangle.signed(count)?;
        let y_max = rectangle.signed(count)?;
        ensure!(
            x_min == 0 && y_min == 0 && x_max > 0 && y_max > 0,
            "unsupported SWF stage rectangle"
        );
        let mut reader = Reader(&body[rectangle.length()..]);
        let fps = f64::from(reader.u16()?) / 256.0;
        ensure!(fps > 0.0 && fps <= 240.0, "unsupported SWF frame rate");
        let root_frames = reader.u16()?;
        let header_size = body.len() - reader.0.len();
        let tags = reader.tags()?;
        let definitions = tags
            .iter()
            .filter(|t| t.code == 60)
            .copied()
            .collect::<Vec<_>>();
        ensure!(
            definitions.len() == 1,
            "SWF requires one embedded video stream"
        );
        let video = Definition::parse(definitions[0])?;
        ensure!(
            x_max == video.width as i32 * 20 && y_max == video.height as i32 * 20,
            "SWF stage and video dimensions differ"
        );
        ensure!(
            f64::from(video.frames) / fps <= 3600.0,
            "SWF duration exceeds one-hour limit"
        );
        let sprites = tags.iter().filter(|t| t.code == 39).collect::<Vec<_>>();
        ensure!(sprites.len() <= 1, "multiple SWF sprites are not supported");
        let timeline = if let Some(sprite) = sprites.first() {
            ensure!(root_frames == 1, "animated SWF wrapper is not supported");
            let mut reader = Reader(sprite.data);
            let sprite_id = Character(reader.u16()?);
            ensure!(sprite_id != video.id, "duplicate SWF character ID");
            ensure!(
                reader.u16()? == video.frames,
                "SWF sprite frame count differs from video"
            );
            let mut displayed = None;
            let mut shown = 0;
            for tag in &tags {
                match tag.code {
                    26 => place(tag.data, &mut displayed, sprite_id)?,
                    1 => {
                        ensure!(
                            tag.data.is_empty() && displayed.as_ref().is_some_and(|p| p.frame == 0),
                            "SWF wrapper does not display its sprite"
                        );
                        shown += 1;
                    }
                    0 | 9 | 39 | 60 | 69 => validate_metadata(*tag)?,
                    code => bail!("unsupported SWF wrapper tag {code}"),
                }
            }
            ensure!(shown == 1, "invalid SWF wrapper frame count");
            reader.tags()?
        } else {
            ensure!(
                root_frames == video.frames,
                "SWF timeline frame count differs from video"
            );
            tags
        };
        let mut output = Vec::with_capacity(body.len() + 8);
        output.extend_from_slice(b"FWS");
        output.push(version);
        output.extend_from_slice(&[0; 4]);
        output.extend_from_slice(&body[..header_size - 2]);
        output.extend_from_slice(&video.frames.to_le_bytes());
        output.extend_from_slice(video.tag.encoded);
        let audio_start_ms = flatten(&timeline, &video, &mut output)?;
        output.extend_from_slice(&[0, 0]);
        let output_len = u32::try_from(output.len())?;
        output[4..8].copy_from_slice(&output_len.to_le_bytes());
        Ok(Self {
            bytes: output,
            timing: Timing {
                width: video.width,
                height: video.height,
                fps,
                frames: video.frames,
                audio_start_ms,
            },
        })
    }
}

fn validate_metadata(tag: Tag<'_>) -> Result<()> {
    match tag.code {
        9 => ensure!(tag.data.len() == 3, "invalid SWF background color"),
        69 => ensure!(
            tag.data.len() == 4 && tag.data[0] & 8 == 0,
            "SWF ActionScript 3 is not supported"
        ),
        _ => {}
    }
    Ok(())
}

fn flatten(tags: &[Tag<'_>], video: &Definition<'_>, output: &mut Vec<u8>) -> Result<f64> {
    let mut frame = 0;
    let mut displayed = None;
    let mut packet = false;
    let mut sound_header = None;
    let mut sound_block = false;
    let mut first_sound = true;
    let mut audio_start_ms = 0.0;
    for tag in tags {
        match tag.code {
            0 | 9 | 69 => validate_metadata(*tag)?,
            60 => ensure!(
                tag.encoded == video.tag.encoded,
                "multiple SWF video definitions are not supported"
            ),
            18 | 45 => {
                ensure!(
                    sound_header.is_none() && frame == 0 && tag.data.len() == 6,
                    "unsupported SWF sound stream header"
                );
                let format = tag.data[1];
                ensure!(
                    format >> 4 == 2 && format & 12 != 0 && format & 2 != 0,
                    "unsupported SWF sound codec: expected MP3"
                );
                let rate = 44100u32 >> (3 - ((format >> 2) & 3));
                let latency = i16::from_le_bytes(tag.data[4..6].try_into()?);
                sound_header = Some((rate, latency));
                output.extend_from_slice(tag.encoded);
            }
            19 => {
                let (rate, latency) = sound_header.context("SWF sound block has no header")?;
                ensure!(!sound_block && tag.data.len() >= 4, "invalid SWF MP3 block");
                if first_sound {
                    ensure!(frame == 0, "delayed SWF sound stream is not supported");
                    ensure!(
                        i16::from_le_bytes(tag.data[2..4].try_into()?) == latency,
                        "SWF initial sound seek differs from latency"
                    );
                    audio_start_ms = -f64::from(latency) * 1000.0 / f64::from(rate);
                    first_sound = false;
                }
                sound_block = true;
                output.extend_from_slice(tag.encoded);
            }
            26 => place(tag.data, &mut displayed, video.id)?,
            61 => {
                let mut reader = Reader(tag.data);
                ensure!(
                    Character(reader.u16()?) == video.id
                        && reader.u16()? == frame
                        && !reader.0.is_empty()
                        && !packet,
                    "unsupported SWF video packet sequence"
                );
                packet = true;
                output.extend_from_slice(tag.encoded);
            }
            1 => {
                ensure!(
                    tag.data.is_empty()
                        && packet
                        && displayed.as_ref().is_some_and(|p| p.frame == frame),
                    "SWF frame does not display its video packet"
                );
                frame = frame.checked_add(1).context("SWF frame count overflow")?;
                packet = false;
                sound_block = false;
                output.extend_from_slice(tag.encoded);
            }
            code => bail!("unsupported SWF timeline tag {code}"),
        }
    }
    ensure!(
        frame == video.frames && !packet && !sound_block,
        "SWF video frame count does not match timeline"
    );
    ensure!(
        sound_header.is_none() || !first_sound,
        "SWF sound stream has no blocks"
    );
    Ok(audio_start_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tag(code: u16, data: &[u8]) -> Vec<u8> {
        let mut encoded = ((code << 6) | 63).to_le_bytes().to_vec();
        encoded.extend_from_slice(&(data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(data);
        encoded
    }

    fn sample(extra: &[u8], ratio: u16) -> Vec<u8> {
        let mut timeline = Vec::new();
        timeline.extend(tag(18, &[0x0e, 0x2e, 0xbe, 5, 0x61, 6]));
        timeline.extend(tag(26, &[6, 1, 0, 1, 0, 0]));
        timeline.extend(tag(61, &[1, 0, 0, 0, 0]));
        timeline.extend(tag(19, &[1, 0, 0x61, 6, 0]));
        timeline.extend(tag(1, &[]));
        timeline.extend(tag(26, &[17, 1, 0, ratio as u8, (ratio >> 8) as u8]));
        timeline.extend(tag(61, &[1, 0, 1, 0, 0]));
        timeline.extend(tag(1, &[]));
        timeline.extend_from_slice(extra);
        timeline.extend(tag(0, &[]));
        let mut sprite = vec![2, 0, 2, 0];
        sprite.extend(timeline);
        // RECT: 10 signed bits per coordinate, (0, 320, 0, 320) twips.
        let fields = [(10, 5), (0, 10), (320, 10), (0, 10), (320, 10)];
        let mut bits = Vec::new();
        for (value, count) in fields {
            for shift in (0..count).rev() {
                bits.push(((value >> shift) & 1) as u8);
            }
        }
        let mut body = vec![0; bits.len().div_ceil(8)];
        for (position, bit) in bits.into_iter().enumerate() {
            body[position / 8] |= bit << (7 - position % 8);
        }
        body.extend_from_slice(&[0, 30, 1, 0]);
        body.extend(tag(60, &[1, 0, 2, 0, 16, 0, 16, 0, 0, 4]));
        body.extend(tag(39, &sprite));
        body.extend(tag(26, &[6, 1, 0, 2, 0, 0]));
        body.extend(tag(1, &[]));
        body.extend(tag(0, &[]));
        let mut bytes = b"FWS\x08".to_vec();
        bytes.extend_from_slice(&((body.len() + 8) as u32).to_le_bytes());
        bytes.extend(body);
        bytes
    }

    #[test]
    fn flattens_sprite_and_preserves_timing_dimensions_and_audio_latency() {
        let bytes = sample(&[], 1);
        let movie = Movie::parse(&bytes).unwrap();
        assert_eq!(
            (movie.timing.width, movie.timing.height, movie.timing.frames),
            (16, 16, 2)
        );
        assert_eq!(movie.timing.fps, 30.0);
        assert!((movie.timing.audio_start_ms + 1633.0 / 44.1).abs() < 1e-9);
        // The decoder input contains media tags only. It is not a display SWF.
        assert_eq!(
            u16::from_le_bytes(movie.bytes[16..18].try_into().unwrap()),
            2
        );
        let flat = Reader(&movie.bytes[18..]).tags().unwrap();
        assert_eq!(
            flat.iter().map(|t| t.code).collect::<Vec<_>>(),
            [60, 18, 61, 19, 1, 61, 1, 0]
        );
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&bytes[8..]).unwrap();
        let mut compressed = bytes[..8].to_vec();
        compressed[0] = b'C';
        compressed.extend(encoder.finish().unwrap());
        assert_eq!(Movie::parse(&compressed).unwrap().bytes, movie.bytes);
    }

    #[test]
    fn rejects_actions_bad_frame_selection_and_truncated_or_oversized_data() {
        assert!(
            Movie::parse(&sample(&tag(12, &[0]), 1))
                .unwrap_err()
                .to_string()
                .contains("tag 12")
        );
        assert!(Movie::parse(&sample(&[], 0)).is_err());
        let bytes = sample(&[], 1);
        for end in [0, 3, 8, 10, bytes.len() - 1] {
            assert!(Movie::parse(&bytes[..end]).is_err());
        }
        let mut oversized = bytes.clone();
        oversized[4..8].copy_from_slice(&(MAX_BYTES as u32 + 1).to_le_bytes());
        assert!(Movie::parse(&oversized).is_err());
    }
}
