//! In-process MPEG playback. Keep compressed video and bounded PCM, and decode
//! only a small queue of pictures. Rewinds restart the decoder from the source.
use anyhow::{Context, Result, ensure};
use krkrz_assets::media::Image;
use na_mpeg2_decoder::{
    Decoder, Demuxer, Frame, MpegAudioF32, MpegAudioPipeline, StreamType,
    frame_to_rgba_bt601_limited,
};
use std::{collections::VecDeque, sync::Arc};

#[path = "ffmpeg_decoder.rs"]
mod ffmpeg;

const MAX_BYTES: usize = 256 * 1024 * 1024;
const CHUNK: usize = 2048;

pub(super) struct Movie {
    ffmpeg: Option<ffmpeg::Video>,
    video: Vec<u8>,
    decoder: Decoder,
    offset: usize,
    pending: VecDeque<Arc<Frame>>,
    flushed: bool,
    next_frame: u64,
    padded_rgba: Vec<u8>,
    audio_rate: u32,
    audio_channels: usize,
    audio_start_ms: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_ms: f64,
    pub frames: u64,
    pub audio: Vec<f32>,
    pub image: Option<Image>,
    pub image_frame: Option<u64>,
}

impl Movie {
    pub fn open(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "compressed movie exceeds 256 MiB");
        if ffmpeg::detects(bytes) {
            return Self::open_ffmpeg(bytes);
        }
        let mut demux = Demuxer::new_auto();
        let mut video = Vec::new();
        let mut video_pts = None;
        let mut audio_present = false;
        for chunk in bytes.chunks(CHUNK) {
            for packet in demux.push(chunk, None) {
                match packet.stream_type {
                    StreamType::MpegVideo => {
                        if video_pts.is_none() {
                            video_pts = packet.pts_90k;
                        }
                        ensure!(
                            video.len() + packet.data.len() <= MAX_BYTES,
                            "movie video exceeds 256 MiB"
                        );
                        video.extend_from_slice(&packet.data);
                    }
                    StreamType::MpegAudio | StreamType::DvdLpcmAudio => audio_present = true,
                    StreamType::Unknown => {}
                }
            }
        }
        let (width, height, fps, frames) = metadata(&video)?;
        let mut pcm = Audio::default();
        let mut audio_decoder = MpegAudioPipeline::new();
        for chunk in bytes.chunks(CHUNK) {
            let mut result = Ok(());
            audio_decoder.push_with(chunk, None, |audio| {
                if result.is_ok() {
                    result = pcm.push(audio);
                }
            })?;
            result?;
        }
        ensure!(
            !audio_present || !pcm.samples.is_empty(),
            "movie audio stream contains no supported samples"
        );
        let audio_start_ms =
            pcm.start_ms.unwrap_or(0) as f64 - video_pts.unwrap_or(0) as f64 / 90.0;
        let duration_ms = (frames as f64 * 1000.0 / fps).max(if pcm.samples.is_empty() {
            0.0
        } else {
            audio_start_ms
                + pcm.samples.len() as f64 * 1000.0 / pcm.channels as f64 / pcm.rate as f64
        });
        ensure!(
            duration_ms > 0.0 && duration_ms <= 3_600_000.0,
            "movie duration exceeds one-hour limit"
        );
        // A final delimiter makes the decoder consume the last slice even when
        // an elementary stream has no sequence-end marker.
        video.extend_from_slice(&[0, 0, 1, 0xb7]);
        Ok(Self {
            ffmpeg: None,
            video,
            decoder: Decoder::new(),
            offset: 0,
            pending: VecDeque::new(),
            flushed: false,
            next_frame: 0,
            padded_rgba: Vec::new(),
            width,
            height,
            fps,
            duration_ms,
            frames,
            audio: pcm.samples,
            audio_rate: pcm.rate,
            audio_channels: pcm.channels,
            audio_start_ms,
            image: None,
            image_frame: None,
        })
    }

    fn open_ffmpeg(bytes: &[u8]) -> Result<Self> {
        let (video, pcm) = ffmpeg::Video::open(bytes)?;
        let audio_start_ms = pcm.start_ms.unwrap_or(0) as f64;
        let duration_ms = video.duration_ms.max(if pcm.samples.is_empty() {
            0.0
        } else {
            audio_start_ms
                + pcm.samples.len() as f64 * 1000.0 / pcm.channels as f64 / pcm.rate as f64
        });
        ensure!(
            duration_ms <= 3_600_000.0,
            "movie duration exceeds one-hour limit"
        );
        Ok(Self {
            width: video.width,
            height: video.height,
            fps: video.fps,
            frames: (duration_ms * video.fps / 1000.0).ceil() as u64,
            duration_ms,
            audio: pcm.samples,
            audio_rate: pcm.rate,
            audio_channels: pcm.channels,
            audio_start_ms,
            ffmpeg: Some(video),
            video: Vec::new(),
            decoder: Decoder::new(),
            offset: 0,
            pending: VecDeque::new(),
            flushed: false,
            next_frame: 0,
            padded_rgba: Vec::new(),
            image: None,
            image_frame: None,
        })
    }

    fn next_picture(&mut self) -> Result<Arc<Frame>> {
        while self.pending.is_empty() && !self.flushed {
            if self.offset == self.video.len() {
                self.pending.extend(self.decoder.flush_shared()?);
                self.flushed = true;
            } else {
                let end = (self.offset + CHUNK).min(self.video.len());
                self.pending.extend(
                    self.decoder
                        .decode_shared(&self.video[self.offset..end], None)?,
                );
                self.offset = end;
            }
            let buffered: usize = self
                .pending
                .iter()
                .map(|f| f.data_y.len() + f.data_u.len() + f.data_v.len())
                .sum();
            ensure!(
                buffered <= 64 * 1024 * 1024,
                "movie frame queue exceeds 64 MiB"
            );
        }
        self.pending
            .pop_front()
            .context("movie video decoding ended before its declared frame count")
    }

    pub fn frame(&mut self, frame: u64) -> Result<bool> {
        let frame = frame.min(self.frames - 1);
        if self.image_frame == Some(frame) {
            return Ok(false);
        }
        if let Some(video) = &mut self.ffmpeg {
            video.frame(frame, &mut self.image)?;
            self.image_frame = Some(frame);
            return Ok(true);
        }
        if frame < self.next_frame {
            self.decoder = Decoder::new();
            self.offset = 0;
            self.pending.clear();
            self.flushed = false;
            self.next_frame = 0;
        }
        while self.next_frame <= frame {
            let decoded = self.next_picture()?;
            ensure!(
                decoded.width == (self.width as usize).div_ceil(16) * 16
                    && decoded.height == (self.height as usize).div_ceil(16) * 16,
                "movie dimensions changed during decoding"
            );
            if self.next_frame == frame {
                let image = self.image.get_or_insert_with(|| Image {
                    width: self.width,
                    height: self.height,
                    rgba: vec![0; self.width as usize * self.height as usize * 4],
                });
                // The decoder retains full macroblocks for motion prediction.
                // Present only the sequence's visible dimensions.
                self.padded_rgba
                    .resize(decoded.width * decoded.height * 4, 0);
                frame_to_rgba_bt601_limited(&decoded, &mut self.padded_rgba);
                for (dst, src) in image
                    .rgba
                    .chunks_exact_mut(self.width as usize * 4)
                    .zip(self.padded_rgba.chunks_exact(decoded.width * 4))
                {
                    dst.copy_from_slice(&src[..self.width as usize * 4]);
                }
            }
            self.next_frame += 1;
        }
        self.image_frame = Some(frame);
        Ok(true)
    }

    /// Convert native-rate mono/stereo PCM to the session clock without making
    /// another full-track buffer. Preserve the audio/video timestamp offset.
    pub fn audio_sample(&self, position_ms: f64, channel: usize) -> f32 {
        if self.audio.is_empty() || position_ms < self.audio_start_ms {
            return 0.0;
        }
        let position = (position_ms - self.audio_start_ms) * self.audio_rate as f64 / 1000.0;
        let at = position.floor() as usize;
        let channel = channel.min(self.audio_channels - 1);
        let Some(&a) = self.audio.get(at * self.audio_channels + channel) else {
            return 0.0;
        };
        let b = self
            .audio
            .get((at + 1) * self.audio_channels + channel)
            .copied()
            .unwrap_or(a);
        a + (b - a) * position.fract() as f32
    }
}

#[derive(Default)]
struct Audio {
    samples: Vec<f32>,
    rate: u32,
    channels: usize,
    start_ms: Option<i64>,
}
impl Audio {
    fn push(&mut self, audio: MpegAudioF32) -> Result<()> {
        ensure!(
            matches!(audio.channels, 1 | 2) && (1..=192_000).contains(&audio.sample_rate),
            "unsupported movie audio format"
        );
        ensure!(
            audio.samples.len().is_multiple_of(audio.channels as usize),
            "incomplete movie audio frame"
        );
        if self.start_ms.is_none() {
            self.start_ms = Some(audio.pts_ms);
            self.rate = audio.sample_rate;
            self.channels = audio.channels as usize;
        }
        ensure!(
            self.rate == audio.sample_rate && self.channels == audio.channels as usize,
            "movie audio format changes during playback"
        );
        ensure!(
            self.samples.len() + audio.samples.len() <= MAX_BYTES / 4,
            "movie PCM exceeds 256 MiB limit"
        );
        ensure!(
            audio.samples.iter().all(|s| s.is_finite()),
            "non-finite movie audio sample"
        );
        self.samples.extend(audio.samples);
        Ok(())
    }
}

/// Scan demuxed elementary video, so picture headers split across PES packets
/// are counted correctly and compressed audio cannot masquerade as a picture.
fn metadata(video: &[u8]) -> Result<(u32, u32, f64, u64)> {
    let mut format = None;
    let mut base_fps = 0.0;
    let mut fps = 0.0;
    let mut frames = 0;
    for (at, prefix) in video.windows(4).enumerate() {
        if prefix[..3] != [0, 0, 1] {
            continue;
        }
        let payload = &video[at + 4..];
        match prefix[3] {
            0xb3 => {
                ensure!(payload.len() >= 8, "truncated MPEG sequence header");
                let width = u32::from(payload[0]) * 16 + u32::from(payload[1] >> 4);
                let height = u32::from(payload[1] & 15) * 256 + u32::from(payload[2]);
                ensure!(
                    width > 0 && height > 0 && width * height <= 16 * 1024 * 1024,
                    "movie dimensions exceed limit"
                );
                base_fps = match payload[3] & 15 {
                    1 => 24000.0 / 1001.0,
                    2 => 24.0,
                    3 => 25.0,
                    4 => 30000.0 / 1001.0,
                    5 => 30.0,
                    6 => 50.0,
                    7 => 60000.0 / 1001.0,
                    8 => 60.0,
                    _ => anyhow::bail!("invalid MPEG frame rate"),
                };
                if let Some((w, h)) = format {
                    ensure!(
                        (width, height) == (w, h),
                        "movie dimensions change during playback"
                    );
                }
                format = Some((width, height));
                fps = base_fps;
            }
            0xb5 => {
                let id = payload.first().context("truncated MPEG extension")? >> 4;
                if id == 1 {
                    ensure!(
                        payload.len() >= 6 && format.is_some(),
                        "invalid MPEG sequence extension"
                    );
                    ensure!(
                        payload[1] & 1 == 0 && payload[2] & 0xe0 == 0,
                        "extended MPEG dimensions are unsupported"
                    );
                    fps = base_fps * f64::from(1 + ((payload[5] >> 5) & 3))
                        / f64::from(1 + (payload[5] & 31));
                } else if id == 8 {
                    ensure!(
                        payload.len() >= 3 && payload[2] & 3 == 3,
                        "field-coded MPEG pictures are unsupported"
                    );
                }
            }
            0 => {
                ensure!(
                    format.is_some() && payload.len() >= 4,
                    "invalid MPEG picture header"
                );
                frames += 1;
            }
            _ => {}
        }
    }
    let (width, height) = format.context("unsupported movie format: expected MPEG-1/2 video")?;
    ensure!(frames > 0, "movie contains no video pictures");
    ensure!(fps > 0.0 && fps <= 240.0, "unsupported movie frame rate");
    ensure!(
        frames as f64 / fps <= 3600.0,
        "movie duration exceeds one-hour limit"
    );
    Ok((width, height, fps, frames))
}

#[cfg(test)]
mod tests {
    use super::*;
    const MOVIE: &[u8] = include_bytes!("../tests/fixtures/video_overlay.mpg");

    #[test]
    fn last_frame_and_rewind_match_sequential_decode() {
        let mut movie = Movie::open(MOVIE).unwrap();
        assert_eq!((movie.width, movie.height, movie.fps), (32, 24, 25.0));
        let mut frames = Vec::new();
        for frame in 0..movie.frames {
            assert!(movie.frame(frame).unwrap());
            frames.push(movie.image.as_ref().unwrap().rgba.clone());
        }
        for frame in [0, movie.frames - 1, 1, movie.frames / 2, 0] {
            movie.frame(frame).unwrap();
            assert_eq!(movie.image.as_ref().unwrap().rgba, frames[frame as usize]);
            assert!(!movie.frame(frame).unwrap());
        }
    }

    #[test]
    fn native_rate_mono_audio_interpolates_and_preserves_start_offset() {
        let mut movie = Movie::open(MOVIE).unwrap();
        movie.audio = vec![0.0, 1.0, 0.0];
        movie.audio_channels = 1;
        movie.audio_rate = 24_000;
        movie.audio_start_ms = 10.0;
        assert_eq!(movie.audio_sample(0.0, 0), 0.0);
        for channel in 0..2 {
            assert!((movie.audio_sample(10.0 + 1.0 / 48.0, channel) - 0.5).abs() < 1e-5);
            assert_eq!(movie.audio_sample(11.0, channel), 0.0);
        }
    }

    #[test]
    fn unsupported_and_truncated_movies_return_errors() {
        assert!(Movie::open(b"not MPEG").is_err());
        assert!(Movie::open(&[0, 0, 1, 0xb3, 2]).is_err());
        assert!(Movie::open(&MOVIE[..32]).is_err());
    }
}
