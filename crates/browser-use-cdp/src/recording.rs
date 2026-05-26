//! HAR, video, trace, and auto-PDF recording helpers.
//!
//! Recording is split out of the session facade so each artifact type can own
//! its CDP subscriptions and serialization details. The public session calls
//! into these helpers when a profile enables HAR, video, trace, or PDF
//! auto-download behavior.

use crate::{
    AttachedPage, BrowserError, BrowserLifecycleEvent, BrowserProfile, BrowserSecurityEvent,
    BrowserViewport, CdpConnection, CdpEvent, VideoRecordingFormat,
};
use base64::Engine;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

mod har;

pub(crate) use har::CdpHarRecorder;

pub(crate) const TRACE_ARTIFACT_SCHEMA_VERSION: &str = "browser-use-rs.trace.v1";
pub(crate) const TRACE_ARTIFACT_KIND: &str = "browser-use-rs.cdp_json_trace";

#[derive(Debug, Clone)]
pub(crate) struct CdpTraceRecorder {
    pub(crate) dir: PathBuf,
}

impl CdpTraceRecorder {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Option<Self> {
        profile
            .traces_dir
            .as_ref()
            .map(|dir| Self { dir: dir.clone() })
    }

    pub(crate) async fn write_trace_artifact(
        &self,
        artifact: Value,
    ) -> Result<PathBuf, BrowserError> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;

        let path = self
            .unique_artifact_path(trace_epoch_millis())
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let bytes = serde_json::to_vec_pretty(&artifact)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let tmp_path = trace_tmp_path(&path);
        tokio::fs::write(&tmp_path, bytes)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        tokio::fs::rename(&tmp_path, &path)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        Ok(path)
    }

    pub(crate) async fn unique_artifact_path(
        &self,
        epoch_millis: u128,
    ) -> Result<PathBuf, std::io::Error> {
        for attempt in 0..1_000 {
            let path = self.artifact_path(epoch_millis, attempt);
            match tokio::fs::metadata(&path).await {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
                Err(error) => return Err(error),
            }
        }
        Ok(self.artifact_path(epoch_millis, 1_000))
    }

    pub(crate) fn artifact_path(&self, epoch_millis: u128, attempt: usize) -> PathBuf {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        self.dir.join(format!(
            "browser-use-rs-trace-{epoch_millis}-{}{suffix}.json",
            std::process::id()
        ))
    }
}

pub(crate) fn trace_epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(crate) fn trace_timestamp(epoch_millis: u128) -> String {
    format_har_timestamp(Some(epoch_millis as f64 / 1_000.0))
}

fn trace_tmp_path(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}tmp",
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!("{extension}."))
            .unwrap_or_default()
    ))
}

pub(crate) fn trace_security_event_json(event: &BrowserSecurityEvent) -> Value {
    json!({
        "message": &event.message,
        "browser_error_message": &event.browser_error_message,
        "closed_popup_message": &event.closed_popup_message,
        "lifecycle_event": &event.lifecycle_event,
    })
}

pub(crate) fn video_recording_failed_event(
    phase: &str,
    error: &BrowserError,
) -> BrowserLifecycleEvent {
    BrowserLifecycleEvent::browser_diagnostic(
        "video_recording_failed",
        BTreeMap::from([("phase".to_owned(), phase.to_owned())]),
        Some(error.to_string()),
        format!("Browser video recording {phase} failed: {error}"),
    )
}

pub(crate) fn trace_recording_failed_event(
    phase: &str,
    error: &BrowserError,
) -> BrowserLifecycleEvent {
    BrowserLifecycleEvent::browser_diagnostic(
        "trace_recording_failed",
        BTreeMap::from([("phase".to_owned(), phase.to_owned())]),
        Some(error.to_string()),
        format!("Browser trace recording {phase} failed: {error}"),
    )
}

#[derive(Debug, Default)]
pub(crate) struct CdpVideoState {
    pub(crate) active_session_id: Option<String>,
    pub(crate) frames: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct CdpVideoRecorder {
    pub(crate) dir: PathBuf,
    pub(crate) size: BrowserViewport,
    pub(crate) framerate: u32,
    pub(crate) format: VideoRecordingFormat,
    pub(crate) ffmpeg_path: PathBuf,
    pub(crate) state: Mutex<CdpVideoState>,
}

impl CdpVideoRecorder {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Option<Arc<Self>> {
        let dir = profile.record_video_dir.clone()?;
        Some(Arc::new(Self {
            dir,
            size: profile.record_video_size.unwrap_or(profile.viewport),
            framerate: profile.record_video_framerate.max(1),
            format: profile.record_video_format,
            ffmpeg_path: std::env::var_os("BROWSER_USE_RS_FFMPEG")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("ffmpeg")),
            state: Mutex::new(CdpVideoState::default()),
        }))
    }

    pub(crate) async fn start_screencast_for_page(
        &self,
        connection: &CdpConnection,
        page: &AttachedPage,
    ) -> Result<(), BrowserError> {
        let previous_session_id = {
            let state = self.state.lock().await;
            if state.active_session_id.as_deref() == Some(page.session_id.as_str()) {
                return Ok(());
            }
            state.active_session_id.clone()
        };

        if let Some(previous_session_id) = previous_session_id {
            let _ = connection
                .command("Page.stopScreencast", json!({}), Some(&previous_session_id))
                .await;
        }

        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        connection
            .command(
                "Page.startScreencast",
                json!({
                    "format": "png",
                    "quality": 90,
                    "maxWidth": self.size.width,
                    "maxHeight": self.size.height,
                    "everyNthFrame": 1,
                }),
                Some(&page.session_id),
            )
            .await?;

        self.state.lock().await.active_session_id = Some(page.session_id.clone());
        Ok(())
    }

    pub(crate) async fn observe_cdp_event(&self, connection: &CdpConnection, event: &CdpEvent) {
        if event.method != "Page.screencastFrame" {
            return;
        }
        let Some(data) = event.params.get("data").and_then(Value::as_str) else {
            return;
        };
        let frame_session_id = event.params.get("sessionId").and_then(Value::as_u64);
        let should_ack = {
            let mut state = self.state.lock().await;
            if state.active_session_id.as_deref() != event.session_id.as_deref() {
                return;
            }
            state.frames.push(data.to_owned());
            frame_session_id
        };

        if let Some(frame_session_id) = should_ack {
            let _ = connection
                .command(
                    "Page.screencastFrameAck",
                    json!({ "sessionId": frame_session_id }),
                    event.session_id.as_deref(),
                )
                .await;
        }
    }

    pub(crate) async fn stop_and_write(
        &self,
        connection: &CdpConnection,
    ) -> Result<(Option<PathBuf>, Option<BrowserError>), BrowserError> {
        let (active_session_id, frames) = {
            let mut state = self.state.lock().await;
            (
                state.active_session_id.take(),
                std::mem::take(&mut state.frames),
            )
        };
        let Some(active_session_id) = active_session_id else {
            return Ok((None, None));
        };

        let stop_result = connection
            .command("Page.stopScreencast", json!({}), Some(&active_session_id))
            .await
            .map(|_| ());
        let (path, encoder_error) = self
            .write_recording_artifact(trace_epoch_millis(), &frames)
            .await?;
        stop_result?;
        Ok((Some(path), encoder_error))
    }

    pub(crate) async fn write_recording_artifact(
        &self,
        epoch_millis: u128,
        frames: &[String],
    ) -> Result<(PathBuf, Option<BrowserError>), BrowserError> {
        let path = self.unique_artifact_path(epoch_millis, self.format).await?;
        if self.format == VideoRecordingFormat::Gif {
            self.write_gif(&path, frames)?;
            return Ok((path, None));
        }

        match self.write_ffmpeg_video(&path, frames, self.format) {
            Ok(()) => Ok((path, None)),
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                let fallback_path = self
                    .unique_artifact_path(epoch_millis, VideoRecordingFormat::Gif)
                    .await?;
                self.write_gif(&fallback_path, frames)?;
                Ok((fallback_path, Some(error)))
            }
        }
    }

    pub(crate) async fn unique_artifact_path(
        &self,
        epoch_millis: u128,
        format: VideoRecordingFormat,
    ) -> Result<PathBuf, BrowserError> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        for attempt in 0..1_000 {
            let path = self.artifact_path(epoch_millis, attempt, format);
            match tokio::fs::metadata(&path).await {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
                Err(error) => return Err(BrowserError::StateUnavailable(error.to_string())),
            }
        }
        Ok(self.artifact_path(epoch_millis, 1_000, format))
    }

    pub(crate) fn artifact_path(
        &self,
        epoch_millis: u128,
        attempt: usize,
        format: VideoRecordingFormat,
    ) -> PathBuf {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        self.dir.join(format!(
            "browser-use-rs-video-{epoch_millis}-{}{suffix}.{}",
            std::process::id(),
            format.as_str()
        ))
    }

    fn write_gif(&self, path: &Path, frames: &[String]) -> Result<(), BrowserError> {
        let file = std::fs::File::create(path)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        encoder
            .set_repeat(image::codecs::gif::Repeat::Infinite)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;

        if frames.is_empty() {
            let frame = image::RgbaImage::from_pixel(
                self.size.width.max(1),
                self.size.height.max(1),
                image::Rgba([0, 0, 0, 255]),
            );
            encoder
                .encode_frame(image::Frame::from_parts(
                    frame,
                    0,
                    0,
                    video_frame_delay(self.framerate),
                ))
                .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
            return Ok(());
        }

        for frame in frames {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(frame)
                .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
            let image = image::load_from_memory(&bytes)
                .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
            let frame = if image.width() == self.size.width && image.height() == self.size.height {
                image.to_rgba8()
            } else {
                image
                    .resize_exact(
                        self.size.width.max(1),
                        self.size.height.max(1),
                        image::imageops::FilterType::Triangle,
                    )
                    .to_rgba8()
            };
            encoder
                .encode_frame(image::Frame::from_parts(
                    frame,
                    0,
                    0,
                    video_frame_delay(self.framerate),
                ))
                .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        }

        Ok(())
    }

    fn write_ffmpeg_video(
        &self,
        path: &Path,
        frames: &[String],
        format: VideoRecordingFormat,
    ) -> Result<(), BrowserError> {
        let frame_dir = tempfile::Builder::new()
            .prefix("browser-use-rs-video-frames-")
            .tempdir_in(&self.dir)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        self.write_frame_png_sequence(frame_dir.path(), frames)?;

        let input_pattern = frame_dir.path().join("frame-%06d.png");
        let mut command = std::process::Command::new(&self.ffmpeg_path);
        command
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-framerate")
            .arg(self.framerate.to_string())
            .arg("-i")
            .arg(&input_pattern)
            .arg("-an")
            .arg("-pix_fmt")
            .arg("yuv420p");

        match format {
            VideoRecordingFormat::Mp4 => {
                command
                    .arg("-c:v")
                    .arg("libx264")
                    .arg("-preset")
                    .arg("veryfast")
                    .arg("-crf")
                    .arg("23")
                    .arg("-movflags")
                    .arg("+faststart");
            }
            VideoRecordingFormat::Webm => {
                command
                    .arg("-c:v")
                    .arg("libvpx-vp9")
                    .arg("-b:v")
                    .arg("0")
                    .arg("-crf")
                    .arg("35");
            }
            VideoRecordingFormat::Gif => {
                return Err(BrowserError::StateUnavailable(
                    "GIF recording does not use ffmpeg video encoding".to_owned(),
                ));
            }
        }

        let output = command
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| {
                BrowserError::StateUnavailable(format!(
                    "ffmpeg video encoder unavailable at {}: {error}",
                    self.ffmpeg_path.display()
                ))
            })?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let detail = if stderr.is_empty() {
            format!("ffmpeg exited with status {}", output.status)
        } else {
            format!("ffmpeg exited with status {}: {stderr}", output.status)
        };
        Err(BrowserError::StateUnavailable(detail))
    }

    fn write_frame_png_sequence(
        &self,
        frame_dir: &Path,
        frames: &[String],
    ) -> Result<(), BrowserError> {
        if frames.is_empty() {
            self.write_frame_png(frame_dir, 0, self.blank_video_frame())?;
            return Ok(());
        }

        for (index, frame) in frames.iter().enumerate() {
            let frame = self.normalized_video_frame(frame)?;
            self.write_frame_png(frame_dir, index, frame)?;
        }
        Ok(())
    }

    fn write_frame_png(
        &self,
        frame_dir: &Path,
        index: usize,
        frame: image::RgbaImage,
    ) -> Result<(), BrowserError> {
        let path = frame_dir.join(format!("frame-{index:06}.png"));
        image::DynamicImage::ImageRgba8(self.pad_video_frame(frame))
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))
    }

    fn normalized_video_frame(&self, frame: &str) -> Result<image::RgbaImage, BrowserError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(frame)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let image = image::load_from_memory(&bytes)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        Ok(
            if image.width() == self.size.width && image.height() == self.size.height {
                image.to_rgba8()
            } else {
                image
                    .resize_exact(
                        self.size.width.max(1),
                        self.size.height.max(1),
                        image::imageops::FilterType::Triangle,
                    )
                    .to_rgba8()
            },
        )
    }

    fn blank_video_frame(&self) -> image::RgbaImage {
        image::RgbaImage::from_pixel(
            self.size.width.max(1),
            self.size.height.max(1),
            image::Rgba([0, 0, 0, 255]),
        )
    }

    fn pad_video_frame(&self, frame: image::RgbaImage) -> image::RgbaImage {
        let padded_width = padded_video_dimension(frame.width(), 16);
        let padded_height = padded_video_dimension(frame.height(), 16);
        if padded_width == frame.width() && padded_height == frame.height() {
            return frame;
        }

        let mut padded =
            image::RgbaImage::from_pixel(padded_width, padded_height, image::Rgba([0, 0, 0, 255]));
        let x_offset = ((padded_width - frame.width()) / 2) as i64;
        let y_offset = ((padded_height - frame.height()) / 2) as i64;
        image::imageops::overlay(&mut padded, &frame, x_offset, y_offset);
        padded
    }
}

fn video_frame_delay(framerate: u32) -> image::Delay {
    image::Delay::from_numer_denom_ms(1_000, framerate.max(1))
}

fn padded_video_dimension(value: u32, macro_block_size: u32) -> u32 {
    let value = value.max(1);
    value.div_ceil(macro_block_size) * macro_block_size
}

pub(super) fn format_har_timestamp(timestamp: Option<f64>) -> String {
    let Some(timestamp) = timestamp else {
        return String::new();
    };
    if !timestamp.is_finite() || timestamp < 0.0 {
        return String::new();
    }
    let total_millis = (timestamp * 1_000.0).round() as i64;
    let total_seconds = total_millis.div_euclid(1_000);
    let millis = total_millis.rem_euclid(1_000);
    let days = total_seconds.div_euclid(86_400);
    let seconds_of_day = total_seconds.rem_euclid(86_400);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let (year, month, day) = civil_from_unix_days(days);
    if millis == 0 {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    } else {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
    }
}

fn civil_from_unix_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096).div_euclid(365);
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let day = doy - (153 * mp + 2).div_euclid(5) + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    (year, month, day)
}
