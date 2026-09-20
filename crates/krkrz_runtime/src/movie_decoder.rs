//! FFmpeg subprocess adapter: compressed storage stays on disk, video frames stream.
use anyhow::{Context, Result, ensure};
use krkrz_assets::media::Image;
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
};

pub(super) struct Movie {
    file: tempfile::NamedTempFile,
    decoder: Option<Child>,
    next_frame: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_ms: f64,
    pub frames: u64,
    pub audio: Vec<f32>,
    pub image: Option<Image>,
    pub image_frame: Option<u64>,
}
impl Drop for Movie {
    fn drop(&mut self) {
        self.stop_decoder();
    }
}
impl Movie {
    pub fn open(bytes: &[u8]) -> Result<Self> {
        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all(bytes)?;
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
            ])
            .arg(file.path())
            .output()
            .context("VideoOverlay requires ffprobe and ffmpeg on PATH")?;
        ensure!(
            probe.status.success(),
            "unsupported movie: {}",
            String::from_utf8_lossy(&probe.stderr)
        );
        let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        let streams = metadata["streams"]
            .as_array()
            .context("movie has no streams")?;
        let videos: Vec<_> = streams
            .iter()
            .filter(|s| s["codec_type"] == "video")
            .collect();
        ensure!(
            videos.len() == 1,
            "movie must have exactly one video stream"
        );
        let video = videos[0];
        let width = video["width"].as_u64().context("missing movie width")? as u32;
        let height = video["height"].as_u64().context("missing movie height")? as u32;
        ensure!(
            width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 16 * 1024 * 1024,
            "movie dimensions exceed limit"
        );
        let rate = video["avg_frame_rate"]
            .as_str()
            .context("missing frame rate")?;
        let (num, den) = rate.split_once('/').context("invalid frame rate")?;
        let fps = num.parse::<f64>()? / den.parse::<f64>()?;
        ensure!(
            fps.is_finite() && fps > 0.0 && fps <= 240.0,
            "unsupported movie frame rate"
        );
        let duration_ms = metadata["format"]["duration"]
            .as_str()
            .or_else(|| video["duration"].as_str())
            .context("missing movie duration")?
            .parse::<f64>()?
            * 1000.0;
        ensure!(
            duration_ms.is_finite() && duration_ms > 0.0 && duration_ms <= 3_600_000.0,
            "movie duration exceeds one-hour limit"
        );
        let frames = video["nb_frames"]
            .as_str()
            .and_then(|n| n.parse().ok())
            .unwrap_or((duration_ms * fps / 1000.0).round() as u64)
            .max(1);
        let mut audio = vec![];
        let audio_streams = streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .count();
        ensure!(
            audio_streams <= 1,
            "multiple movie audio streams are unsupported"
        );
        if audio_streams == 1 {
            let mut child = Command::new("ffmpeg")
                .args(["-v", "error", "-nostdin", "-threads", "1", "-i"])
                .arg(file.path())
                .args([
                    "-map", "0:a:0", "-vn", "-f", "f32le", "-ar", "48000", "-ac", "2", "pipe:1",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .context("VideoOverlay requires ffmpeg on PATH")?;
            let mut pcm = vec![];
            const MAX_PCM: u64 = 256 * 1024 * 1024;
            let result = child
                .stdout
                .take()
                .unwrap()
                .take(MAX_PCM + 1)
                .read_to_end(&mut pcm);
            if result.is_err() || pcm.len() as u64 > MAX_PCM {
                let _ = child.kill();
            }
            let status = child.wait()?;
            result?;
            ensure!(
                pcm.len() as u64 <= MAX_PCM,
                "movie PCM exceeds 256 MiB limit"
            );
            ensure!(status.success(), "movie audio decoding failed");
            audio = pcm
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
        }
        Ok(Self {
            file,
            decoder: None,
            next_frame: 0,
            width,
            height,
            fps,
            duration_ms,
            frames,
            audio,
            image: None,
            image_frame: None,
        })
    }
    fn stop_decoder(&mut self) {
        if let Some(mut child) = self.decoder.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    pub fn frame(&mut self, frame: u64) -> Result<bool> {
        let frame = frame.min(self.frames - 1);
        if self.image_frame == Some(frame) {
            return Ok(false);
        }
        // Large forward jumps and rewinds seek the compressed source instead of
        // decoding every intervening frame. FFmpeg performs accurate output seek.
        if self.decoder.is_none() || frame < self.next_frame || frame > self.next_frame + 120 {
            self.stop_decoder();
            // MPEG program streams can omit timestamps in the final GOP.
            // Decode a pre-roll before the accurate output seek.
            let seconds = frame as f64 / self.fps;
            let coarse = (seconds - 2.0).max(0.0);
            self.decoder = Some(
                Command::new("ffmpeg")
                    .args([
                        "-v",
                        "error",
                        "-nostdin",
                        "-threads",
                        "1",
                        "-ss",
                        &format!("{coarse:.9}"),
                        "-i",
                    ])
                    .arg(self.file.path())
                    .args([
                        "-ss",
                        &format!("{:.9}", seconds - coarse),
                        "-map",
                        "0:v:0",
                        "-an",
                        "-sn",
                        "-r",
                        &format!("{:.9}", self.fps),
                        "-pix_fmt",
                        "rgba",
                        "-f",
                        "rawvideo",
                        "pipe:1",
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .context("VideoOverlay requires ffmpeg on PATH")?,
            );
            self.next_frame = frame;
        }
        let size = self.width as usize * self.height as usize * 4;
        let image = self.image.get_or_insert_with(|| Image {
            width: self.width,
            height: self.height,
            rgba: vec![0; size],
        });
        while self.next_frame <= frame {
            self.decoder
                .as_mut()
                .unwrap()
                .stdout
                .as_mut()
                .unwrap()
                .read_exact(&mut image.rgba)
                .context("movie video decoding ended before its declared frame count")?;
            self.next_frame += 1;
        }
        self.image_frame = Some(frame);
        Ok(true)
    }
}
