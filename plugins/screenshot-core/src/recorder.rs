use crate::capture::RecordingFrameGrabber;
use crate::model::{ScreenRect, RECORDING_FPS_OPTIONS};
use gif::{Encoder as GifEncoder, Frame as GifFrame, Repeat};
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageBuffer, Rgba};
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
#[path = "recording_mp4.rs"]
mod recording_mp4;

const MAX_DURATION: Duration = Duration::from_secs(10);
const MAX_FRAMES: usize = 300;
const MAX_FRAME_BYTES: usize = 512 * 1024 * 1024;
const MAX_PREVIEW_FRAMES: usize = 80;
const PREVIEW_WIDTH: u32 = 640;
const PREVIEW_HEIGHT: u32 = 360;
const PREVIEW_SAMPLE_INTERVAL_MS: u32 = 500;
const MP4_MIN_FREE_SPACE_BYTES: u64 = 512 * 1024 * 1024;
const MP4_DISK_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const MAX_OUTPUT_WIDTH: u32 = 1280;
const MAX_OUTPUT_HEIGHT: u32 = 720;
const MAX_MP4_OUTPUT_WIDTH: u32 = 1920;
const MAX_MP4_OUTPUT_HEIGHT: u32 = 1080;
const MAX_SOURCE_PIXELS: usize = 32_000_000;
const WEBP_CHUNK_LIMIT: usize = 0x00ff_ffff;

static FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatus {
    pub phase: String,
    pub paused: bool,
    pub fps: u32,
    pub frame_count: usize,
    pub elapsed_ms: u64,
    pub max_duration_ms: u64,
    pub message: Option<String>,
    pub preview_path: Option<String>,
    pub last_path: Option<String>,
}

impl Default for RecordingStatus {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            paused: false,
            fps: 15,
            frame_count: 0,
            elapsed_ms: 0,
            max_duration_ms: MAX_DURATION.as_millis() as u64,
            message: None,
            preview_path: None,
            last_path: None,
        }
    }
}

impl RecordingStatus {
    pub fn timeline_frame_count(&self) -> usize {
        if self.max_duration_ms != 0 || self.elapsed_ms == 0 {
            return self.frame_count;
        }
        let fps = u128::from(self.fps.max(1));
        let frames = u128::from(self.elapsed_ms)
            .saturating_mul(fps)
            .saturating_add(999)
            / 1000;
        frames
            .max(1)
            .min(usize::MAX as u128) as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationFormat {
    Webp,
    Gif,
    Mp4,
}

impl AnimationFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Webp => "webp",
            Self::Gif => "gif",
            Self::Mp4 => "mp4",
        }
    }
}

struct RecordedFrame {
    timestamp_ms: u64,
    pixels: Vec<u8>,
}

struct CaptureControl {
    stop: AtomicBool,
    discard: AtomicBool,
    paused: AtomicBool,
    pause_started: Mutex<Option<Instant>>,
    paused_ms: AtomicU64,
}

impl CaptureControl {
    fn set_paused(&self, paused: bool) {
        let mut pause_started = self.pause_started.lock().unwrap_or_else(|e| e.into_inner());
        if paused && !self.paused.load(Ordering::Acquire) {
            *pause_started = Some(Instant::now());
            self.paused.store(true, Ordering::Release);
        } else if !paused && self.paused.load(Ordering::Acquire) {
            if let Some(started) = pause_started.take() {
                self.paused_ms.fetch_add(
                    started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                    Ordering::AcqRel,
                );
            }
            self.paused.store(false, Ordering::Release);
        }
    }

    fn active_elapsed(&self, started_at: Instant) -> Duration {
        let paused_total = self.paused_ms.load(Ordering::Acquire);
        let current_pause = self
            .pause_started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|started| started.elapsed().as_millis().min(u64::MAX as u128) as u64)
            .unwrap_or(0);
        started_at
            .elapsed()
            .saturating_sub(Duration::from_millis(paused_total.saturating_add(current_pause)))
    }
}

struct RecordingInner {
    status: RecordingStatus,
    width: u32,
    height: u32,
    preview_width: u32,
    preview_height: u32,
    bytes: usize,
    frames: Vec<RecordedFrame>,
    started_at: Option<Instant>,
    capture_control: Option<Arc<CaptureControl>>,
    capture_thread: Option<JoinHandle<()>>,
    encode_cancel: Option<Arc<AtomicBool>>,
    encode_thread: Option<JoinHandle<()>>,
    source_video_file: Option<PathBuf>,
}

impl Default for RecordingInner {
    fn default() -> Self {
        Self {
            status: RecordingStatus::default(),
            width: 0,
            height: 0,
            preview_width: 0,
            preview_height: 0,
            bytes: 0,
            frames: Vec::new(),
            started_at: None,
            capture_control: None,
            capture_thread: None,
            encode_cancel: None,
            encode_thread: None,
            source_video_file: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct RecordingManager {
    inner: Arc<Mutex<RecordingInner>>,
}

impl RecordingManager {
    pub fn request_selection(&self, fps: u32) -> Result<(), String> {
        if !RECORDING_FPS_OPTIONS.contains(&fps) {
            return Err("录制帧率只支持 15 或 30 FPS".into());
        }
        let (stale_threads, source_video_file) = {
            let mut inner = self.lock()?;
            if is_busy(&inner.status.phase) {
                return Err("当前录制尚未完成".into());
            }
            if let Some(cancel) = inner.encode_cancel.as_ref() {
                cancel.store(true, Ordering::Release);
            }
            let mut threads = Vec::new();
            if let Some(thread) = inner.capture_thread.take() {
                threads.push(thread);
            }
            if let Some(thread) = inner.encode_thread.take() {
                threads.push(thread);
            }
            (threads, inner.source_video_file.take())
        };
        for thread in stale_threads {
            let _ = thread.join();
        }
        if let Some(path) = source_video_file {
            let _ = fs::remove_file(path);
        }
        {
            let mut inner = self.lock()?;
            inner.encode_cancel = None;
            inner.frames.clear();
            inner.bytes = 0;
            inner.width = 0;
            inner.height = 0;
            inner.preview_width = 0;
            inner.preview_height = 0;
            inner.started_at = None;
            inner.capture_control = None;
            inner.status = RecordingStatus {
                phase: "selecting".into(),
                fps,
                ..RecordingStatus::default()
            };
        }
        Ok(())
    }

    pub fn start_capture(
        &self,
        rect: ScreenRect,
        fps: u32,
        capture_cursor: bool,
        format: AnimationFormat,
        mp4_max_height: u32,
        mp4_bitrate_mbps: u32,
    ) -> Result<(), String> {
        if !RECORDING_FPS_OPTIONS.contains(&fps) {
            return Err("录制帧率只支持 15 或 30 FPS".into());
        }
        let source_width = u32::try_from(rect.width()).map_err(|_| "录制区域尺寸无效")?;
        let source_height = u32::try_from(rect.height()).map_err(|_| "录制区域尺寸无效")?;
        let source_pixels = (source_width as usize)
            .checked_mul(source_height as usize)
            .ok_or_else(|| "录制区域过大".to_string())?;
        if source_pixels > MAX_SOURCE_PIXELS {
            return Err("录制区域过大，请选择不超过 3200 万像素的区域".into());
        }
        let (max_output_width, max_output_height) = if format == AnimationFormat::Mp4 {
            if mp4_max_height <= 720 {
                (MAX_OUTPUT_WIDTH, MAX_OUTPUT_HEIGHT)
            } else {
                (MAX_MP4_OUTPUT_WIDTH, MAX_MP4_OUTPUT_HEIGHT)
            }
        } else {
            (MAX_OUTPUT_WIDTH, MAX_OUTPUT_HEIGHT)
        };
        let scale = (max_output_width as f64 / source_width as f64)
            .min(max_output_height as f64 / source_height as f64)
            .min(1.0);
        let width = ((source_width as f64 * scale).round() as u32).max(1);
        let height = ((source_height as f64 * scale).round() as u32).max(1);
        let interval = Duration::from_micros(1_000_000 / u64::from(fps));
        let control = Arc::new(CaptureControl {
            stop: AtomicBool::new(false),
            discard: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            pause_started: Mutex::new(None),
            paused_ms: AtomicU64::new(0),
        });
        let started_at = Instant::now();
        {
            let mut inner = self.lock()?;
            if inner.status.phase != "selecting" {
                return Err("录制选区状态已失效，请重新开始".into());
            }
            inner.status.phase = "recording".into();
            inner.status.message = None;
            inner.status.max_duration_ms = match format {
                AnimationFormat::Mp4 => 0,
                AnimationFormat::Gif | AnimationFormat::Webp => MAX_DURATION.as_millis() as u64,
            };
            inner.width = width;
            inner.height = height;
            inner.preview_width = width;
            inner.preview_height = height;
            inner.bytes = 0;
            inner.frames.clear();
            inner.source_video_file = None;
            inner.started_at = Some(started_at);
            inner.capture_control = Some(Arc::clone(&control));
        }

        let manager = self.clone();
        let worker = thread::Builder::new()
            .name("screenshot-recorder".into())
            .spawn(move || {
                manager.capture_loop(
                    rect,
                    width,
                    height,
                    fps,
                    interval,
                    started_at,
                    control,
                    capture_cursor,
                    format,
                    mp4_bitrate_mbps,
                );
            })
            .map_err(|error| {
                if let Ok(mut inner) = self.inner.lock() {
                    inner.status.phase = "error".into();
                    inner.status.message = Some(format!("无法启动录制线程：{error}"));
                }
                error.to_string()
            })?;
        self.lock()?.capture_thread = Some(worker);
        Ok(())
    }

    pub fn request_stop(&self) -> Result<(), String> {
        let inner = self.lock()?;
        if inner.status.phase != "recording" {
            return Err("当前没有正在录制的内容".into());
        }
        let Some(control) = inner.capture_control.as_ref() else {
            return Err("当前没有正在录制的内容".into());
        };
        control.stop.store(true, Ordering::Release);
        Ok(())
    }

    pub fn toggle_pause(&self) -> Result<bool, String> {
        let mut inner = self.lock()?;
        if inner.status.phase != "recording" {
            return Err("Recording is not active".into());
        }
        let Some(control) = inner.capture_control.as_ref() else {
            return Err("Recording is not active".into());
        };
        let paused = !control.paused.load(Ordering::Acquire);
        control.set_paused(paused);
        inner.status.paused = paused;
        Ok(paused)
    }

    pub fn cancel(&self) {
        let (has_encoder, capture_thread) = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(control) = inner.capture_control.as_ref() {
                control.discard.store(true, Ordering::Release);
                control.stop.store(true, Ordering::Release);
            }
            if let Some(cancel) = inner.encode_cancel.as_ref() {
                cancel.store(true, Ordering::Release);
            }
            inner.status.phase = "cancelling".into();
            (inner.encode_cancel.is_some(), inner.capture_thread.take())
        };
        if let Some(thread) = capture_thread {
            let _ = thread.join();
        }
        if has_encoder {
            return;
        }
        let source_video_file = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.frames.clear();
            inner.bytes = 0;
            inner.status.preview_path = None;
            inner.source_video_file.take()
        };
        if let Some(path) = source_video_file {
            let _ = fs::remove_file(path);
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.status.phase == "cancelling" {
            inner.status = RecordingStatus {
                phase: "cancelled".into(),
                ..RecordingStatus::default()
            };
        }
    }

    pub(crate) fn fail(&self, error: String) {
        self.set_error(error);
    }

    pub fn export(
        &self,
        format: AnimationFormat,
        path: PathBuf,
        output_fps: u32,
        mp4_bitrate_mbps: u32,
    ) -> Result<(), String> {
        if !RECORDING_FPS_OPTIONS.contains(&output_fps) {
            return Err("导出帧率无效".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (frames, width, height, fps, source_video_file) = {
            let mut inner = self.lock()?;
            if inner.status.phase != "preview" {
                return Err("录制预览尚未准备好".into());
            }
            if inner.status.frame_count < 2 {
                return Err("录制帧数不足".into());
            }
            let frames = std::mem::take(&mut inner.frames);
            let width = inner.width;
            let height = inner.height;
            let fps = inner.status.fps;
            let source_video = inner.source_video_file.clone();
            inner.status.phase = "exporting".into();
            inner.status.message = Some(format!("正在导出 {}…", format.extension().to_uppercase()));
            inner.encode_cancel = Some(Arc::clone(&cancel));
            (frames, width, height, fps, source_video)
        };
        let manager = self.clone();
        let frames = Arc::new(Mutex::new(Some(frames)));
        let worker_frames = Arc::clone(&frames);
        let start_gate = Arc::new(Barrier::new(2));
        let worker_gate = Arc::clone(&start_gate);
        let worker = thread::Builder::new()
            .name("screenshot-animation-export".into())
            .spawn(move || {
                worker_gate.wait();
                let frames = worker_frames
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .unwrap_or_default();
                let result = if cancel.load(Ordering::Acquire) {
                    Err("导出已取消".into())
                } else {
                    export_animation(
                        format,
                        &path,
                        &frames,
                        width,
                        height,
                        fps,
                        output_fps,
                        mp4_bitrate_mbps,
                        source_video_file.as_deref(),
                        &cancel,
                    )
                };
                let mut inner = manager.inner.lock().unwrap_or_else(|e| e.into_inner());
                inner.encode_cancel = None;
                inner.encode_thread = None;
                if cancel.load(Ordering::Acquire) {
                    inner.frames.clear();
                    inner.bytes = 0;
                    if let Some(source) = inner.source_video_file.take() {
                        let _ = fs::remove_file(source);
                    }
                    inner.status.preview_path = None;
                    inner.status.phase = "cancelled".into();
                    inner.status.message = Some("录制已取消".into());
                } else {
                    inner.frames = frames;
                    inner.bytes = inner.frames.iter().map(|frame| frame.pixels.len()).sum();
                    inner.status.phase = "preview".into();
                    match result {
                        Ok(()) => {
                            inner.status.last_path = Some(path.to_string_lossy().into_owned());
                            inner.status.message = Some(format!("已导出 {}", path.display()));
                        }
                        Err(error) => {
                            inner.status.phase = "preview".into();
                            inner.status.message = Some(error);
                        }
                    }
                }
            })
            .map_err(|error| {
                if let Ok(mut inner) = self.inner.lock() {
                    inner.frames = frames
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take()
                        .unwrap_or_default();
                    inner.bytes = inner.frames.iter().map(|frame| frame.pixels.len()).sum();
                    inner.encode_cancel = None;
                    inner.status.phase = "preview".into();
                    inner.status.message = Some(format!("无法启动导出线程：{error}"));
                }
                error.to_string()
            })?;
        self.lock()?.encode_thread = Some(worker);
        start_gate.wait();
        Ok(())
    }

    pub fn snapshot(&self) -> RecordingStatus {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.status.phase == "recording" {
            if let Some(started_at) = inner.started_at {
                let elapsed = inner
                    .capture_control
                    .as_ref()
                    .map(|control| control.active_elapsed(started_at))
                    .unwrap_or_else(|| started_at.elapsed());
                inner.status.elapsed_ms = elapsed.as_millis() as u64;
                inner.status.paused = inner
                    .capture_control
                    .as_ref()
                    .is_some_and(|control| control.paused.load(Ordering::Acquire));
            }
        }
        inner.status.clone()
    }

    pub fn preview_frame(&self, index: usize) -> Option<(u32, u32, Vec<u8>)> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.status.phase != "preview" || inner.frames.is_empty() {
            return None;
        }
        let frame = if inner.source_video_file.is_some() {
            let target_ms = (index as u64)
                .saturating_mul(1000)
                .checked_div(u64::from(inner.status.fps.max(1)))
                .unwrap_or(0);
            inner
                .frames
                .iter()
                .min_by_key(|frame| frame.timestamp_ms.abs_diff(target_ms))?
        } else {
            &inner.frames[index % inner.frames.len()]
        };
        Some((inner.preview_width, inner.preview_height, frame.pixels.clone()))
    }

    pub fn shutdown(&self) {
        self.cancel();
        loop {
            let (worker, still_encoding) = {
                let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                (inner.encode_thread.take(), inner.encode_cancel.is_some())
            };
            if let Some(worker) = worker {
                let _ = worker.join();
                break;
            }
            if !still_encoding {
                break;
            }
            thread::yield_now();
        }
    }

    fn capture_loop(
        &self,
        rect: ScreenRect,
        width: u32,
        height: u32,
        fps: u32,
        interval: Duration,
        started_at: Instant,
        control: Arc<CaptureControl>,
        capture_cursor: bool,
        format: AnimationFormat,
        mp4_bitrate_mbps: u32,
    ) {
        let mut deadline = started_at;
        let mut capture_error = None;
        let mut stop_reason = None;
        let mut next_disk_check = Instant::now();
        let Some(mut grabber) = RecordingFrameGrabber::new(rect, width, height, capture_cursor)
        else {
            self.set_error("无法初始化录屏帧采集".into());
            return;
        };
        let mut mp4_path = None;
        let mut mp4_encoder = if format == AnimationFormat::Mp4 {
            let path = temporary_recording_mp4_path();
            match recording_mp4::StreamingMp4Encoder::create(
                &path,
                width,
                height,
                fps,
                mp4_bitrate_mbps,
            ) {
                Ok(encoder) => {
                    if let Ok(mut inner) = self.inner.lock() {
                        inner.source_video_file = Some(path.clone());
                    }
                    mp4_path = Some(path);
                    Some(encoder)
                }
                Err(error) => {
                    let _ = fs::remove_file(path);
                    self.set_error(error);
                    return;
                }
            }
        } else {
            None
        };
        let (preview_width, preview_height) = preview_dimensions(width, height);
        let mut preview_interval_ms = PREVIEW_SAMPLE_INTERVAL_MS;
        let mut next_preview_ms = 0u64;
        let mut frame_count = 0usize;
        loop {
            if control.stop.load(Ordering::Acquire) {
                break;
            }
            if control.paused.load(Ordering::Acquire) {
                deadline = Instant::now() + interval;
                thread::sleep(interval.min(Duration::from_millis(100)));
                continue;
            }
            if format != AnimationFormat::Mp4
                && control.active_elapsed(started_at) >= MAX_DURATION
            {
                stop_reason = Some("已达到 10 秒录制上限，正在打开预览".into());
                break;
            }
            if format == AnimationFormat::Mp4 && Instant::now() >= next_disk_check {
                next_disk_check = Instant::now() + MP4_DISK_CHECK_INTERVAL;
                if available_temp_disk_space()
                    .is_some_and(|available| available < MP4_MIN_FREE_SPACE_BYTES)
                {
                    if frame_count == 0 {
                        capture_error = Some("临时磁盘剩余空间不足 512 MB，无法开始 MP4 录制".into());
                    } else {
                        stop_reason = Some("临时磁盘剩余空间不足 512 MB，已停止并保存 MP4".into());
                    }
                    break;
                }
            }
            let Some(raw) = grabber.capture() else {
                capture_error = Some("屏幕帧采集失败，录制已停止".to_string());
                break;
            };
            let elapsed = control.active_elapsed(started_at);
            let elapsed_ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
            let timestamp = if frame_count == 0 {
                0
            } else {
                elapsed_ms
            };

            if let Some(encoder) = mp4_encoder.as_mut() {
                if let Err(error) = encoder.write_frame(timestamp, &raw) {
                    capture_error = Some(error);
                    break;
                }
                let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                frame_count += 1;
                inner.status.frame_count = frame_count;
                inner.status.elapsed_ms = elapsed.as_millis() as u64;
                if timestamp >= next_preview_ms {
                    match make_preview_sample(width, height, raw, preview_width, preview_height) {
                        Some(pixels) => {
                            inner.bytes = inner.bytes.saturating_add(pixels.len());
                            inner.frames.push(RecordedFrame {
                                timestamp_ms: timestamp,
                                pixels,
                            });
                            inner.preview_width = preview_width;
                            inner.preview_height = preview_height;
                            next_preview_ms = timestamp.saturating_add(u64::from(preview_interval_ms));
                            if inner.frames.len() >= MAX_PREVIEW_FRAMES {
                                let mut index = 0usize;
                                inner.frames.retain(|_| {
                                    let keep = index % 2 == 0;
                                    index += 1;
                                    keep
                                });
                                inner.bytes = inner.frames.iter().map(|frame| frame.pixels.len()).sum();
                                preview_interval_ms = preview_interval_ms.saturating_mul(2);
                            }
                        }
                        None => {
                            capture_error = Some("无法生成录制预览帧".into());
                            break;
                        }
                    }
                }
            } else {
                let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                let byte_count = raw.len();
                if inner.bytes.saturating_add(byte_count) > MAX_FRAME_BYTES
                    || inner.frames.len() >= MAX_FRAMES
                {
                    stop_reason = Some("已达到 GIF/WebP 录制资源上限，正在打开预览".into());
                    break;
                }
                inner.bytes += byte_count;
                inner.frames.push(RecordedFrame {
                    timestamp_ms: timestamp,
                    pixels: raw,
                });
                inner.preview_width = width;
                inner.preview_height = height;
                frame_count += 1;
                inner.status.frame_count = frame_count;
                inner.status.elapsed_ms = elapsed.as_millis() as u64;
            }
            deadline += interval;
            let now = Instant::now();
            if deadline > now {
                thread::sleep(deadline - now);
            } else {
                deadline = now + interval;
            }
        }

        if control.discard.load(Ordering::Acquire) {
            drop(mp4_encoder.take());
            if let Some(path) = mp4_path.take() {
                let _ = fs::remove_file(path);
            }
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.frames.clear();
            inner.bytes = 0;
            inner.source_video_file = None;
            inner.status = RecordingStatus {
                phase: "cancelled".into(),
                ..RecordingStatus::default()
            };
            return;
        }
        if let Some(error) = capture_error {
            drop(mp4_encoder.take());
            if let Some(path) = mp4_path.take() {
                let _ = fs::remove_file(path);
            }
            if let Ok(mut inner) = self.inner.lock() {
                inner.source_video_file = None;
            }
            self.set_error(error);
            return;
        }
        if let Some(mut encoder) = mp4_encoder.take() {
            if let Err(error) = encoder.finish() {
                drop(encoder);
                if let Some(path) = mp4_path.take() {
                    let _ = fs::remove_file(path);
                }
                if let Ok(mut inner) = self.inner.lock() {
                    inner.source_video_file = None;
                }
                self.set_error(error);
                return;
            }
        }
        self.begin_preview(stop_reason);
    }

    fn begin_preview(&self, stop_reason: Option<String>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.status.paused = false;
        if inner.status.frame_count < 2 {
            inner.frames.clear();
            inner.bytes = 0;
            inner.status.phase = "error".into();
            inner.status.message = Some("录制时间太短，至少需要两帧".into());
            return;
        }

        // The editor previews captured frames directly. Encoding a lossless WebP
        // here duplicated work and delayed the transition to the editor.
        inner.status.preview_path = None;
        inner.status.phase = "preview".into();
        inner.status.message = Some(stop_reason.unwrap_or_else(|| "录制已完成".into()));
    }

    fn set_error(&self, error: String) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.frames.clear();
            inner.bytes = 0;
            inner.status.phase = "error".into();
            inner.status.message = Some(error);
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, RecordingInner>, String> {
        self.inner.lock().map_err(|_| "录制状态锁已损坏".into())
    }
}

fn is_busy(phase: &str) -> bool {
    matches!(
        phase,
        "selecting" | "recording" | "processing" | "exporting" | "cancelling"
    )
}

fn preview_dimensions(width: u32, height: u32) -> (u32, u32) {
    let scale = (PREVIEW_WIDTH as f64 / width as f64)
        .min(PREVIEW_HEIGHT as f64 / height as f64)
        .min(1.0);
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

fn make_preview_sample(
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    preview_width: u32,
    preview_height: u32,
) -> Option<Vec<u8>> {
    let image = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, pixels)?;
    Some(
        image::imageops::resize(&image, preview_width, preview_height, FilterType::Triangle)
            .into_raw(),
    )
}

fn temporary_recording_mp4_path() -> PathBuf {
    std::env::temp_dir().join(format!("AgileTilesRecording-{}.mp4", unique_suffix()))
}

#[cfg(windows)]
fn available_temp_disk_space() -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let directory: Vec<u16> = std::env::temp_dir()
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut available = 0u64;
    let succeeded = unsafe {
        GetDiskFreeSpaceExW(
            directory.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (succeeded != 0).then_some(available)
}

#[cfg(not(windows))]
fn available_temp_disk_space() -> Option<u64> {
    None
}

fn export_animation(
    format: AnimationFormat,
    path: &Path,
    frames: &[RecordedFrame],
    width: u32,
    height: u32,
    fps: u32,
    output_fps: u32,
    mp4_bitrate_mbps: u32,
    source_video: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let temp_path = sibling_temp_path(path);
    let result = match format {
        AnimationFormat::Webp => {
            if let Some(source_video) = source_video {
                #[cfg(windows)]
                {
                    encode_webp_from_mp4(
                        &temp_path,
                        source_video,
                        width,
                        height,
                        fps,
                        output_fps,
                        cancel,
                    )
                }
                #[cfg(not(windows))]
                {
                    let _ = source_video;
                    Err("从 MP4 导出 WebP 目前只支持 Windows".into())
                }
            } else {
                encode_webp_animation(frames, width, height, fps, output_fps, cancel)
                    .and_then(|bytes| fs::write(&temp_path, bytes).map_err(|e| e.to_string()))
            }
        }
        AnimationFormat::Gif => {
            if let Some(source_video) = source_video {
                #[cfg(windows)]
                {
                    encode_gif_from_mp4(
                        &temp_path,
                        source_video,
                        width,
                        height,
                        fps,
                        output_fps,
                        cancel,
                    )
                }
                #[cfg(not(windows))]
                {
                    let _ = source_video;
                    Err("从 MP4 导出 GIF 目前只支持 Windows".into())
                }
            } else {
                encode_gif(&temp_path, frames, width, height, fps, output_fps, cancel)
            }
        }
        AnimationFormat::Mp4 => {
            if let Some(source_video) = source_video {
                fs::copy(source_video, &temp_path)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                #[cfg(windows)]
                {
                recording_mp4::encode_mp4(
                    &temp_path,
                    frames,
                    width,
                    height,
                    fps,
                    output_fps,
                    mp4_bitrate_mbps,
                    cancel,
                )
                }
                #[cfg(not(windows))]
                {
                    Err("MP4 recording is currently supported only on Windows".into())
                }
            }
        }
    };
    if let Err(error) = result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    if cancel.load(Ordering::Acquire) {
        let _ = fs::remove_file(&temp_path);
        return Err("导出已取消".into());
    }
    replace_file(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        error
    })
}

fn encode_gif(
    path: &Path,
    frames: &[RecordedFrame],
    width: u32,
    height: u32,
    source_fps: u32,
    output_fps: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if frames.is_empty() || width > u16::MAX as u32 || height > u16::MAX as u32 {
        return Err("GIF 尺寸或帧数据无效".into());
    }
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    {
        let mut encoder = GifEncoder::new(&mut writer, width as u16, height as u16, &[])
            .map_err(|e| e.to_string())?;
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        for (frame_index, _, duration_ms) in frame_schedule(frames, source_fps, output_fps) {
            if cancel.load(Ordering::Acquire) {
                return Err("导出已取消".into());
            }
            let frame = &frames[frame_index];
            let mut pixels = frame.pixels.clone();
            let mut gif_frame =
                GifFrame::from_rgba_speed(width as u16, height as u16, &mut pixels, 10);
            gif_frame.delay = (duration_ms.saturating_add(5) / 10)
                .clamp(1, u32::from(u16::MAX)) as u16;
            encoder.write_frame(&gif_frame).map_err(|e| e.to_string())?;
        }
    }
    writer.flush().map_err(|e| e.to_string())?;
    writer.get_ref().sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(windows)]
fn encode_gif_from_mp4(
    path: &Path,
    source_video: &Path,
    width: u32,
    height: u32,
    source_fps: u32,
    output_fps: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if width == 0 || height == 0 || width > u16::MAX as u32 || height > u16::MAX as u32 {
        return Err("GIF 尺寸或帧数据无效".into());
    }
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    {
        let mut encoder = GifEncoder::new(&mut writer, width as u16, height as u16, &[])
            .map_err(|e| e.to_string())?;
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        recording_mp4::visit_mp4_frames(
            source_video,
            width,
            height,
            source_fps,
            output_fps,
            cancel,
            |_, duration_ms, pixels| {
                let mut pixels = pixels.to_vec();
                let mut frame = GifFrame::from_rgba_speed(
                    width as u16,
                    height as u16,
                    &mut pixels,
                    10,
                );
                frame.delay = (duration_ms.saturating_add(5) / 10)
                    .clamp(1, u64::from(u16::MAX)) as u16;
                encoder.write_frame(&frame).map_err(|e| e.to_string())
            },
        )?;
    }
    writer.flush().map_err(|e| e.to_string())?;
    writer.get_ref().sync_all().map_err(|e| e.to_string())
}

fn encode_webp_animation(
    frames: &[RecordedFrame],
    width: u32,
    height: u32,
    source_fps: u32,
    output_fps: u32,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    if frames.is_empty() || width == 0 || height == 0 {
        return Err("WebP 尺寸或帧数据无效".into());
    }
    let mut chunks = Vec::new();
    let mut vp8x = Vec::with_capacity(10);
    vp8x.push(0x02); // Animation feature flag.
    vp8x.extend_from_slice(&[0, 0, 0]);
    push_u24(&mut vp8x, width - 1)?;
    push_u24(&mut vp8x, height - 1)?;
    append_riff_chunk(&mut chunks, *b"VP8X", &vp8x)?;

    let mut anim = Vec::with_capacity(6);
    anim.extend_from_slice(&[0, 0, 0, 0]); // BGRA background color.
    anim.extend_from_slice(&0u16.to_le_bytes()); // Infinite loop.
    append_riff_chunk(&mut chunks, *b"ANIM", &anim)?;

    for (frame_index, _, duration_ms) in frame_schedule(frames, source_fps, output_fps) {
        if cancel.load(Ordering::Acquire) {
            return Err("导出已取消".into());
        }
        let frame = &frames[frame_index];
        let mut static_webp = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut static_webp)
            .encode(&frame.pixels, width, height, ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        let frame_chunks = extract_webp_frame_chunks(&static_webp)?;
        let mut payload = Vec::with_capacity(16 + frame_chunks.len());
        push_u24(&mut payload, 0)?;
        push_u24(&mut payload, 0)?;
        push_u24(&mut payload, width - 1)?;
        push_u24(&mut payload, height - 1)?;
            push_u24(&mut payload, duration_ms.min(WEBP_CHUNK_LIMIT as u32))?;
        payload.push(0x02); // Do not blend; every frame covers the full canvas.
        payload.extend_from_slice(&frame_chunks);
        append_riff_chunk(&mut chunks, *b"ANMF", &payload)?;
    }

    let riff_size = chunks
        .len()
        .checked_add(4)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "WebP 动画超过格式大小上限".to_string())?;
    let mut output = Vec::with_capacity(chunks.len() + 12);
    output.extend_from_slice(b"RIFF");
    output.extend_from_slice(&riff_size.to_le_bytes());
    output.extend_from_slice(b"WEBP");
    output.extend_from_slice(&chunks);
    Ok(output)
}

#[cfg(windows)]
fn encode_webp_from_mp4(
    path: &Path,
    source_video: &Path,
    width: u32,
    height: u32,
    source_fps: u32,
    output_fps: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("WebP 尺寸或帧数据无效".into());
    }
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"RIFF").map_err(|e| e.to_string())?;
    writer.write_all(&0u32.to_le_bytes()).map_err(|e| e.to_string())?;
    writer.write_all(b"WEBP").map_err(|e| e.to_string())?;
    let mut riff_size = 4u64;

    let mut vp8x = Vec::with_capacity(10);
    vp8x.push(0x02);
    vp8x.extend_from_slice(&[0, 0, 0]);
    push_u24(&mut vp8x, width - 1)?;
    push_u24(&mut vp8x, height - 1)?;
    write_riff_chunk(&mut writer, *b"VP8X", &vp8x, &mut riff_size)?;
    let mut anim = Vec::with_capacity(6);
    anim.extend_from_slice(&[0, 0, 0, 0]);
    anim.extend_from_slice(&0u16.to_le_bytes());
    write_riff_chunk(&mut writer, *b"ANIM", &anim, &mut riff_size)?;

    recording_mp4::visit_mp4_frames(
        source_video,
        width,
        height,
        source_fps,
        output_fps,
        cancel,
        |_, duration_ms, pixels| {
            let mut static_webp = Vec::new();
            image::codecs::webp::WebPEncoder::new_lossless(&mut static_webp)
                .encode(pixels, width, height, ExtendedColorType::Rgba8)
                .map_err(|e| e.to_string())?;
            let frame_chunks = extract_webp_frame_chunks(&static_webp)?;
            let mut payload = Vec::with_capacity(16 + frame_chunks.len());
            push_u24(&mut payload, 0)?;
            push_u24(&mut payload, 0)?;
            push_u24(&mut payload, width - 1)?;
            push_u24(&mut payload, height - 1)?;
            push_u24(&mut payload, duration_ms.min(WEBP_CHUNK_LIMIT as u64) as u32)?;
            payload.push(0x02);
            payload.extend_from_slice(&frame_chunks);
            write_riff_chunk(&mut writer, *b"ANMF", &payload, &mut riff_size)
        },
    )?;

    let riff_size = u32::try_from(riff_size)
        .map_err(|_| "WebP 动画超过格式大小上限".to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    writer.get_mut().seek(SeekFrom::Start(4)).map_err(|e| e.to_string())?;
    writer
        .write_all(&riff_size.to_le_bytes())
        .map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    writer.get_ref().sync_all().map_err(|e| e.to_string())
}

fn write_riff_chunk(
    writer: &mut impl Write,
    id: [u8; 4],
    data: &[u8],
    riff_size: &mut u64,
) -> Result<(), String> {
    if data.len() > WEBP_CHUNK_LIMIT || data.len() > u32::MAX as usize {
        return Err("WebP 动画区块过大".into());
    }
    writer.write_all(&id).map_err(|e| e.to_string())?;
    writer
        .write_all(&(data.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    writer.write_all(data).map_err(|e| e.to_string())?;
    if data.len() & 1 == 1 {
        writer.write_all(&[0]).map_err(|e| e.to_string())?;
    }
    *riff_size = riff_size
        .checked_add(8 + data.len() as u64 + (data.len() as u64 & 1))
        .ok_or_else(|| "WebP 动画超过格式大小上限".to_string())?;
    Ok(())
}

fn extract_webp_frame_chunks(webp: &[u8]) -> Result<Vec<u8>, String> {
    if webp.len() < 12 || &webp[..4] != b"RIFF" || &webp[8..12] != b"WEBP" {
        return Err("静态 WebP 编码结果无效".into());
    }
    let mut output = Vec::new();
    let mut cursor = 12usize;
    let mut has_image_data = false;
    while cursor + 8 <= webp.len() {
        let id = &webp[cursor..cursor + 4];
        let size = u32::from_le_bytes(webp[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let start = cursor + 8;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= webp.len())
            .ok_or_else(|| "静态 WebP 区块长度无效".to_string())?;
        let padded_end = end
            .checked_add(size & 1)
            .filter(|end| *end <= webp.len())
            .ok_or_else(|| "静态 WebP 区块缺少对齐字节".to_string())?;
        if matches!(id, b"VP8 " | b"VP8L" | b"ALPH") {
            let chunk = &webp[cursor..padded_end];
            output.extend_from_slice(chunk);
            has_image_data |= id == b"VP8 " || id == b"VP8L";
        }
        cursor = padded_end;
    }
    if !has_image_data {
        return Err("静态 WebP 中没有可用帧数据".into());
    }
    Ok(output)
}

fn append_riff_chunk(output: &mut Vec<u8>, id: [u8; 4], data: &[u8]) -> Result<(), String> {
    if data.len() > WEBP_CHUNK_LIMIT || data.len() > u32::MAX as usize {
        return Err("WebP 动画区块过大".into());
    }
    output.extend_from_slice(&id);
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output.extend_from_slice(data);
    if data.len() & 1 == 1 {
        output.push(0);
    }
    Ok(())
}

fn push_u24(output: &mut Vec<u8>, value: u32) -> Result<(), String> {
    if value > WEBP_CHUNK_LIMIT as u32 {
        return Err("WebP 画面尺寸超过格式上限".into());
    }
    output.extend_from_slice(&value.to_le_bytes()[..3]);
    Ok(())
}

fn frame_schedule(
    frames: &[RecordedFrame],
    source_fps: u32,
    output_fps: u32,
) -> Vec<(usize, u64, u32)> {
    let Some(last_frame) = frames.last() else {
        return Vec::new();
    };
    let source_fps = u64::from(source_fps.max(1));
    let output_fps = u64::from(output_fps.max(1));
    let final_frame_duration = 1000u64.saturating_add(source_fps - 1) / source_fps;
    let end_ms = last_frame
        .timestamp_ms
        .saturating_add(final_frame_duration.max(1));
    let mut schedule = Vec::new();
    let mut frame_index = 0usize;
    let mut output_index = 0u64;
    loop {
        let timestamp_ms = output_index.saturating_mul(1000) / output_fps;
        if timestamp_ms >= end_ms {
            break;
        }
        while frame_index + 1 < frames.len()
            && frames[frame_index + 1].timestamp_ms <= timestamp_ms
        {
            frame_index += 1;
        }
        let next_timestamp_ms = output_index.saturating_add(1).saturating_mul(1000) / output_fps;
        let duration_ms = next_timestamp_ms
            .saturating_sub(timestamp_ms)
            .max(1)
            .min(u64::from(u32::MAX)) as u32;
        schedule.push((frame_index, timestamp_ms, duration_ms));
        output_index = output_index.saturating_add(1);
    }
    schedule
}

fn unique_suffix() -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{stamp}-{}-{sequence}", std::process::id())
}

fn sibling_temp_path(path: &Path) -> PathBuf {
    let file_stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("recording");
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("tmp");
    path.with_file_name(format!(".{file_stem}.{}.tmp.{extension}", unique_suffix()))
}

fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};
    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let moved = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
