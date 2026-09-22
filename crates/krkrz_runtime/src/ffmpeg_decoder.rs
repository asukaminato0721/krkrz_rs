//! ASF/movie and AVI fallback using local FFmpeg tools. Compressed input lives
//! in a private temporary file; video is streamed one RGBA frame at a time.
use super::{Audio, MAX_BYTES};
use anyhow::{Context, Result, bail, ensure};
use krkrz_assets::media::Image;
use serde::Deserialize;
use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom, Write},
    process::{Child, ChildStdout, Command, Stdio},
};
use tempfile::NamedTempFile;

const ASF_HEADER: &[u8] = &[
    0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62, 0xce, 0x6c,
];

pub(super) fn detects(bytes: &[u8]) -> bool {
    bytes.starts_with(ASF_HEADER)
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"AVI "))
}

#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
    format: Format,
}
#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
}
#[derive(Deserialize)]
struct Stream {
    codec_type: String,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    start_time: Option<String>,
    duration: Option<String>,
}

fn rate(value: &str) -> Option<f64> {
    let (n, d) = value.split_once('/')?;
    let rate = n.parse::<f64>().ok()? / d.parse::<f64>().ok()?;
    (rate.is_finite() && rate > 0.0 && rate <= 240.0).then_some(rate)
}
fn seconds(value: Option<&str>) -> Option<f64> {
    value?.parse::<f64>().ok().filter(|v| v.is_finite())
}

pub(super) struct Video {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_ms: f64,
    source: NamedTempFile,
    process: Option<Process>,
    next_frame: u64,
    ended: bool,
}

impl Video {
    pub fn open(bytes: &[u8]) -> Result<(Self, Audio)> {
        let mut source = tempfile::Builder::new().suffix(".movie").tempfile()?;
        source.write_all(bytes)?;
        source.flush()?;
        let probe = Process::read_bounded(
            Command::new("ffprobe")
                .args(["-v", "error", "-protocol_whitelist", "file,pipe", "-show_entries",
                    "format=duration:stream=codec_type,width,height,avg_frame_rate,r_frame_rate,start_time,duration",
                    "-of", "json"])
                .arg(source.path()),
            1024 * 1024,
        ).context("probe movie (ffprobe must be installed on PATH)")?;
        let probe: Probe = serde_json::from_slice(&probe).context("invalid movie metadata")?;
        let stream = probe
            .streams
            .iter()
            .find(|s| s.codec_type == "video")
            .context("movie contains no video stream")?;
        let width = stream.width.context("movie video width is missing")?;
        let height = stream.height.context("movie video height is missing")?;
        ensure!(
            width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 16 * 1024 * 1024,
            "movie dimensions exceed limit"
        );
        let fps = stream
            .avg_frame_rate
            .as_deref()
            .and_then(rate)
            .or_else(|| stream.r_frame_rate.as_deref().and_then(rate))
            .context("unsupported movie frame rate")?;
        let duration_ms = seconds(stream.duration.as_deref())
            .or_else(|| seconds(probe.format.duration.as_deref()))
            .context("movie duration is missing")?
            * 1000.0;
        ensure!(
            duration_ms > 0.0 && duration_ms <= 3_600_000.0,
            "movie duration exceeds one-hour limit"
        );
        let mut audio = Audio::default();
        if let Some(sound) = probe.streams.iter().find(|s| s.codec_type == "audio") {
            let pcm = Process::read_bounded(
                Self::command(source.path()).args([
                    "-map", "0:a:0", "-vn", "-ac", "2", "-ar", "48000", "-f", "f32le", "pipe:1",
                ]),
                MAX_BYTES,
            )
            .context("decode movie audio (ffmpeg must be installed on PATH)")?;
            ensure!(
                !pcm.is_empty() && pcm.len().is_multiple_of(8),
                "incomplete movie audio"
            );
            audio.samples = pcm
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            ensure!(
                audio.samples.iter().all(|s| s.is_finite()),
                "non-finite movie audio sample"
            );
            audio.rate = 48_000;
            audio.channels = 2;
            audio.start_ms = Some(
                ((seconds(sound.start_time.as_deref()).unwrap_or(0.0)
                    - seconds(stream.start_time.as_deref()).unwrap_or(0.0))
                    * 1000.0)
                    .round() as i64,
            );
        }
        Ok((
            Self {
                width,
                height,
                fps,
                duration_ms,
                source,
                process: None,
                next_frame: 0,
                ended: false,
            },
            audio,
        ))
    }

    fn command(path: &std::path::Path) -> Command {
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-nostdin",
                "-v",
                "error",
                "-xerror",
                "-threads",
                "2",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
            ])
            .arg(path);
        command
    }

    pub fn frame(&mut self, target: u64, image: &mut Option<Image>) -> Result<()> {
        if target < self.next_frame {
            self.process = None; // Drop kills and reaps the previous decoder.
            self.next_frame = 0;
            self.ended = false;
        }
        if self.process.is_none() && !self.ended {
            self.process = Some(
                Process::spawn(
                    Self::command(self.source.path())
                        .args(["-map", "0:v:0", "-an", "-vf"])
                        .arg(format!(
                            "setpts=PTS-STARTPTS,fps=fps={}:start_time=0",
                            self.fps
                        ))
                        .args([
                            "-pix_fmt", "rgba", "-threads", "2", "-f", "rawvideo", "pipe:1",
                        ]),
                )
                .context("decode movie video (ffmpeg must be installed on PATH)")?,
            );
        }
        let image = image.get_or_insert_with(|| Image {
            width: self.width,
            height: self.height,
            rgba: vec![0; self.width as usize * self.height as usize * 4],
        });
        while self.next_frame <= target && !self.ended {
            let process = self.process.as_mut().unwrap();
            // Read a byte first so normal EOF leaves the last complete frame intact.
            let mut first = [0];
            if process.output.read(&mut first)? == 0 {
                process.finish()?;
                ensure!(self.next_frame > 0, "movie contains no decoded frames");
                self.process = None;
                self.ended = true;
                break;
            }
            image.rgba[0] = first[0];
            if let Err(error) = process.output.read_exact(&mut image.rgba[1..]) {
                process.stop();
                bail!(
                    "incomplete movie video frame: {error}; {}",
                    process.diagnostic()
                );
            }
            self.next_frame += 1;
        }
        // Container duration can outlast its last video packet. Hold that frame until
        // the session reaches EOF, including any remaining audio samples.
        self.next_frame = target + 1;
        Ok(())
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        self.process = None; // Close decoder handles before deleting the input on Windows.
    }
}

struct Process {
    child: Child,
    output: BufReader<ChildStdout>,
    errors: File,
}
impl Process {
    fn spawn(command: &mut Command) -> Result<Self> {
        let errors = tempfile::tempfile()?;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(errors.try_clone()?)
            .spawn()
            .with_context(|| {
                format!(
                    "could not start {}",
                    command.get_program().to_string_lossy()
                )
            })?;
        let output = BufReader::new(child.stdout.take().context("decoder stdout is missing")?);
        Ok(Self {
            child,
            output,
            errors,
        })
    }
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    fn diagnostic(&mut self) -> String {
        let mut bytes = Vec::new();
        let _ = self.errors.seek(SeekFrom::Start(0));
        let _ = (&mut self.errors).take(64 * 1024).read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).trim().to_owned()
    }
    fn finish(&mut self) -> Result<()> {
        let status = self.child.wait()?;
        ensure!(
            status.success(),
            "FFmpeg decoder failed ({status}): {}",
            self.diagnostic()
        );
        Ok(())
    }
    fn read_bounded(command: &mut Command, limit: usize) -> Result<Vec<u8>> {
        let mut process = Self::spawn(command)?;
        let mut bytes = Vec::new();
        (&mut process.output)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= limit,
            "FFmpeg decoder output exceeds {limit} bytes"
        );
        process.finish()?;
        Ok(bytes)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_container_and_rejects_invalid_rates() {
        assert!(detects(ASF_HEADER));
        assert!(!detects(&ASF_HEADER[..15]));
        assert!(!detects(b"fake.wmv"));
        assert!(detects(b"RIFF\x00\x00\x00\x00AVI "));
        assert!(!detects(b"RIFF\x00\x00\x00\x00WAVE"));
        assert!(!detects(b"RIFF\x00\x00\x00\x00WEBP"));
        assert!(!detects(b"RIFF\x00\x00\x00\x00AVI"));
        assert!(!detects(b"fake.avi"));
        for invalid in ["0/0", "1/0", "NaN/1", "241/1", "-30/1", "30"] {
            assert!(rate(invalid).is_none());
        }
        assert_eq!(rate("30000/1001"), Some(30000.0 / 1001.0));
    }
}
