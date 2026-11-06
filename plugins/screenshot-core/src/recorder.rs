use crate::model::{ScreenRect, RECORDING_FPS_OPTIONS};
use color_quant::NeuQuant;
use gif::{DisposalMethod, Encoder as GifEncoder, Frame as GifFrame, Repeat};
use image::imageops::FilterType;
use image::{ImageBuffer, Rgba};
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
#[path = "recording_mp4.rs"]
mod recording_mp4;

const MAX_DURATION: Duration = Duration::from_secs(10 * 60);
const MAX_PREVIEW_FRAMES: usize = 48;
const PREVIEW_WIDTH: u32 = 640;
const PREVIEW_HEIGHT: u32 = 360;
const PREVIEW_SAMPLE_INTERVAL_MS: u32 = 250;
const MP4_MIN_FREE_SPACE_BYTES: u64 = 512 * 1024 * 1024;
const MP4_DISK_CHECK_INTERVAL: Duration = Duration::from_secs(5);
// GIF exports are derived from the shared temporary video so recording can
// stay sharp without retaining full-resolution RGBA frames in memory.
const MAX_GIF_OUTPUT_WIDTH: u32 = 960;
const MAX_GIF_OUTPUT_HEIGHT: u32 = 540;
const MAX_GIF_HIGH_FPS_OUTPUT_WIDTH: u32 = 800;
const MAX_GIF_HIGH_FPS_OUTPUT_HEIGHT: u32 = 450;
const MAX_GIF_LARGE_OUTPUT_WIDTH: u32 = 1920;
const MAX_GIF_LARGE_OUTPUT_HEIGHT: u32 = 1080;
const MAX_GIF_LARGE_HIGH_FPS_OUTPUT_WIDTH: u32 = 1280;
const MAX_GIF_LARGE_HIGH_FPS_OUTPUT_HEIGHT: u32 = 720;
const MAX_RECORDING_720P_WIDTH: u32 = 1280;
const MAX_RECORDING_720P_HEIGHT: u32 = 720;
const MAX_RECORDING_1080P_WIDTH: u32 = 1920;
const MAX_RECORDING_1080P_HEIGHT: u32 = 1080;
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
    pub progress_percent: u8,
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
            progress_percent: 0,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GifExportQuality {
    #[default]
    Recommended,
    High,
    Balanced,
    Small,
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
    export_progress: Arc<AtomicU32>,
    #[cfg(windows)]
    preview_decoder: Option<recording_mp4::PreviewDecoder>,
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
            export_progress: Arc::new(AtomicU32::new(0)),
            #[cfg(windows)]
            preview_decoder: None,
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
            return Err("录制帧率设置无效，请在详情设置页选择支持的帧率".into());
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
        #[cfg(windows)]
        let preview_decoder = self
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .preview_decoder
            .take();
        #[cfg(windows)]
        drop(preview_decoder);
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
        max_height: u32,
        bitrate_mbps: u32,
    ) -> Result<(), String> {
        if !RECORDING_FPS_OPTIONS.contains(&fps) {
            return Err("录制帧率设置无效，请在详情设置页选择支持的帧率".into());
        }
        let source_width = u32::try_from(rect.width()).map_err(|_| "录制区域尺寸无效")?;
        let source_height = u32::try_from(rect.height()).map_err(|_| "录制区域尺寸无效")?;
        let source_pixels = (source_width as usize)
            .checked_mul(source_height as usize)
            .ok_or_else(|| "录制区域过大".to_string())?;
        if source_pixels > MAX_SOURCE_PIXELS {
            return Err("录制区域过大，请选择不超过 3200 万像素的区域".into());
        }
        let (max_output_width, max_output_height) = if max_height <= 720 {
            (
                MAX_RECORDING_720P_WIDTH,
                MAX_RECORDING_720P_HEIGHT,
            )
        } else {
            (
                MAX_RECORDING_1080P_WIDTH,
                MAX_RECORDING_1080P_HEIGHT,
            )
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
            inner.status.max_duration_ms = MAX_DURATION.as_millis() as u64;
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
                let result = catch_unwind(AssertUnwindSafe(|| {
                    manager.capture_loop(
                        rect,
                        source_width,
                        source_height,
                        width,
                        height,
                        fps,
                        interval,
                        started_at,
                        Arc::clone(&control),
                        capture_cursor,
                        bitrate_mbps,
                    )
                }));
                if result.is_err() {
                    let stale_source = manager
                        .inner
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .source_video_file
                        .take();
                    if let Some(path) = stale_source {
                        let _ = fs::remove_file(path);
                    }
                    manager.set_error("录制过程中发生异常，录制资源已释放".into());
                }
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
        let mut inner = self.lock()?;
        if inner.status.phase == "processing" {
            return Ok(());
        }
        if inner.status.phase != "recording" {
            return Err("当前没有正在录制的内容".into());
        }
        let Some(control) = inner.capture_control.as_ref() else {
            return Err("当前没有正在录制的内容".into());
        };
        control.stop.store(true, Ordering::Release);
        inner.status.paused = false;
        inner.status.phase = "processing".into();
        inner.status.message = Some("正在完成录制，请稍候…".into());
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
        #[cfg(windows)]
        let preview_decoder = self
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .preview_decoder
            .take();
        #[cfg(windows)]
        drop(preview_decoder);
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
        gif_export_quality: GifExportQuality,
        path: PathBuf,
        output_fps: u32,
        mp4_bitrate_mbps: u32,
        gif_colors: u16,
        webp_quality: u8,
    ) -> Result<(), String> {
        if !RECORDING_FPS_OPTIONS.contains(&output_fps) {
            return Err("导出帧率无效".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (frames, width, height, fps, source_video_file, source_duration_ms, export_progress) = {
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
            let source_duration_ms = inner.status.elapsed_ms;
            inner.export_progress.store(0, Ordering::Release);
            let export_progress = Arc::clone(&inner.export_progress);
            inner.status.phase = "exporting".into();
            inner.status.message = Some(format!("正在导出 {}…", format.extension().to_uppercase()));
            inner.status.progress_percent = 0;
            inner.encode_cancel = Some(Arc::clone(&cancel));
            (
                frames,
                width,
                height,
                fps,
                source_video,
                source_duration_ms,
                export_progress,
            )
        };
        #[cfg(windows)]
        let preview_decoder = self
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .preview_decoder
            .take();
        #[cfg(windows)]
        drop(preview_decoder);

        let manager = self.clone();
        let frames = Arc::new(Mutex::new(Some(frames)));
        let worker_frames = Arc::clone(&frames);
        let start_gate = Arc::new(Barrier::new(2));
        let worker_gate = Arc::clone(&start_gate);
        let worker = thread::Builder::new()
            .name("screenshot-animation-export".into())
            .spawn(move || {
                worker_gate.wait();
                let mut frames = worker_frames
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .unwrap_or_default();
                let result = if cancel.load(Ordering::Acquire) {
                    Err("导出已取消".into())
                } else {
                    catch_unwind(AssertUnwindSafe(|| {
                        export_animation(
                            format,
                            &path,
                            &mut frames,
                            width,
                            height,
                            fps,
                            output_fps,
                            gif_export_quality,
                            mp4_bitrate_mbps,
                            gif_colors,
                            webp_quality,
                            source_video_file.as_deref(),
                            source_duration_ms,
                            &export_progress,
                            &cancel,
                        )
                    }))
                    .unwrap_or_else(|_| {
                        Err("导出过程中发生异常，请检查导出目录中的 .tmp.gif 文件".into())
                    })
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
                    inner.status.progress_percent = 0;
                    inner.export_progress.store(0, Ordering::Release);
                } else {
                    inner.frames = frames;
                    inner.bytes = inner.frames.iter().map(|frame| frame.pixels.len()).sum();
                    inner.status.phase = "preview".into();
                    inner.status.progress_percent =
                        inner.export_progress.load(Ordering::Acquire).min(100) as u8;
                    match result {
                        Ok(()) => {
                            inner.status.last_path = Some(path.to_string_lossy().into_owned());
                            inner.status.message = Some(format!("导出完成：{}", path.display()));
                        }
                        Err(error) => {
                            inner.status.phase = "preview".into();
                            inner.status.message = Some(format!("导出失败：{error}"));
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
                    inner.status.progress_percent =
                        inner.export_progress.load(Ordering::Acquire).min(100) as u8;
                    inner.status.message = Some(format!("导出失败：无法启动导出线程：{error}"));
                }
                error.to_string()
            })?;
        self.lock()?.encode_thread = Some(worker);
        start_gate.wait();
        Ok(())
    }

    pub fn snapshot(&self) -> RecordingStatus {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.status.phase == "exporting" {
            inner.status.progress_percent =
                inner.export_progress.load(Ordering::Acquire).min(100) as u8;
        }
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

    #[cfg(windows)]
    pub fn request_preview_frame(
        &self,
        index: usize,
    ) -> Option<(usize, u32, u32, Vec<u8>)> {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if inner.status.phase != "preview" || inner.frames.is_empty() {
            return None;
        }

        if let Some(path) = inner.source_video_file.clone() {
            if inner.preview_decoder.is_none() {
                let (width, height, preview_width, preview_height, fps) = (
                    inner.width,
                    inner.height,
                    inner.preview_width,
                    inner.preview_height,
                    inner.status.fps,
                );
                inner.preview_decoder = recording_mp4::PreviewDecoder::spawn(
                    path,
                    width,
                    height,
                    preview_width,
                    preview_height,
                    fps,
                );
            }
            if let Some(decoder) = inner.preview_decoder.as_ref() {
                if !decoder.failed() {
                    return decoder
                        .request_frame(index)
                        .map(|(frame_index, pixels)| {
                            (frame_index, inner.preview_width, inner.preview_height, pixels)
                        });
                }
            }
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
        Some((
            index,
            inner.preview_width,
            inner.preview_height,
            frame.pixels.clone(),
        ))
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
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
        fps: u32,
        interval: Duration,
        started_at: Instant,
        control: Arc<CaptureControl>,
        capture_cursor: bool,
        bitrate_mbps: u32,
    ) {
        let mut capture_error = None;
        let mut stop_reason = None;
        let mut next_disk_check = Instant::now();
        let path = temporary_recording_mp4_path();
        let mut source_cleanup = RecordingSourceCleanup {
            path: path.clone(),
            keep: false,
        };
        let mut capture = match crate::wgc::RecordingCapture::create(
            rect,
            source_width,
            source_height,
            width,
            height,
            fps,
            capture_cursor,
            &path,
            bitrate_mbps,
        ) {
            Ok(capture) => capture,
            Err(error) => {
                let _ = fs::remove_file(&path);
                self.set_error(error);
                return;
            }
        };
        let (width, height) = capture.output_dimensions();
        let (preview_width, preview_height) = preview_dimensions(width, height);
        if let Ok(mut inner) = self.inner.lock() {
            inner.width = width;
            inner.height = height;
            inner.preview_width = preview_width;
            inner.preview_height = preview_height;
            inner.source_video_file = Some(path.clone());
        }
        let frequency = match crate::wgc::qpc_frequency() {
            Ok(frequency) => frequency,
            Err(error) => {
                let _ = capture.finish();
                let _ = fs::remove_file(&path);
                if let Ok(mut inner) = self.inner.lock() {
                    inner.source_video_file = None;
                }
                self.set_error(error);
                return;
            }
        };
        let qpc_origin = match crate::wgc::qpc_now() {
            Ok(now) => now,
            Err(error) => {
                let _ = capture.finish();
                if let Ok(mut inner) = self.inner.lock() {
                    inner.source_video_file = None;
                }
                self.set_error(error);
                return;
            }
        };
        let capture_init_started = Instant::now();
        let mut frame_pacer = crate::wgc::FramePacer::default();
        let mut preview_interval_ms = PREVIEW_SAMPLE_INTERVAL_MS;
        let mut next_preview_ms = 0u64;
        let mut frame_count = 0usize;
        loop {
            if control.stop.load(Ordering::Acquire) {
                break;
            }
            if control.paused.load(Ordering::Acquire) {
                if let Err(error) = capture.poll() {
                    capture_error = Some(error);
                    break;
                }
                thread::sleep(interval.min(Duration::from_millis(25)));
                continue;
            }
            if control.active_elapsed(started_at) >= MAX_DURATION {
                stop_reason = Some("已达到 10 分钟录制上限，正在打开预览".into());
                break;
            }
            if Instant::now() >= next_disk_check {
                next_disk_check = Instant::now() + MP4_DISK_CHECK_INTERVAL;
                if available_temp_disk_space()
                    .is_some_and(|available| available < MP4_MIN_FREE_SPACE_BYTES)
                {
                    if frame_count == 0 {
                        capture_error = Some("临时磁盘剩余空间不足 512 MB，无法开始录制".into());
                    } else {
                        stop_reason = Some("临时磁盘剩余空间不足 512 MB，录制已停止并保存".into());
                    }
                    break;
                }
            }
            if let Err(error) = capture.poll() {
                capture_error = Some(error);
                break;
            }
            if !capture.ready() {
                if capture_init_started.elapsed() >= Duration::from_secs(5) {
                    capture_error = Some(
                        "显示器在 5 秒内没有返回首帧，请检查 Windows 屏幕录制权限".into(),
                    );
                    break;
                }
                thread::sleep(interval.min(Duration::from_millis(8)));
                continue;
            }
            let frame_time = active_qpc_time(&control, started_at, qpc_origin, frequency);
            if !frame_pacer.accept(frame_time, fps, frequency) {
                thread::sleep(interval.min(Duration::from_millis(8)));
                continue;
            }
            let encoded = match capture.encode_frame(frame_time, frequency) {
                Ok(encoded) => encoded,
                Err(error) => {
                    capture_error = Some(error);
                    break;
                }
            };
            if !encoded {
                thread::sleep(interval.min(Duration::from_millis(2)));
                continue;
            }
            let elapsed = control.active_elapsed(started_at);
            let elapsed_ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
            frame_count += 1;
            {
                let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                inner.status.frame_count = frame_count;
                inner.status.elapsed_ms = elapsed_ms;
            }
            if elapsed_ms >= next_preview_ms {
                let raw = match capture.preview_pixels() {
                    Ok(pixels) => pixels,
                    Err(error) => {
                        capture_error = Some(error);
                        break;
                    }
                };
                match make_preview_sample(width, height, raw, preview_width, preview_height) {
                    Some(pixels) => {
                        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                        inner.bytes = inner.bytes.saturating_add(pixels.len());
                        inner.frames.push(RecordedFrame {
                            timestamp_ms: elapsed_ms,
                            pixels,
                        });
                        inner.preview_width = preview_width;
                        inner.preview_height = preview_height;
                        next_preview_ms =
                            elapsed_ms.saturating_add(u64::from(preview_interval_ms));
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
        }

        if !control.discard.load(Ordering::Acquire) && capture_error.is_none() {
            if let Ok(mut inner) = self.inner.lock() {
                if inner.status.phase == "recording" {
                    inner.status.paused = false;
                    inner.status.phase = "processing".into();
                    inner.status.message = Some("正在完成录制，请稍候…".into());
                }
            }
        }

        let elapsed_ms = control
            .active_elapsed(started_at)
            .as_millis()
            .min(u64::MAX as u128) as u64;
        if let Ok(mut inner) = self.inner.lock() {
            inner.status.elapsed_ms = elapsed_ms;
        }
        let finish_error = capture.finish().err();

        if control.discard.load(Ordering::Acquire) {
            let _ = fs::remove_file(&path);
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
        if let Some(error) = capture_error.or(finish_error) {
            let _ = fs::remove_file(&path);
            if let Ok(mut inner) = self.inner.lock() {
                inner.source_video_file = None;
            }
            self.set_error(error);
            return;
        }
        let preview_ready = self
            .inner
            .lock()
            .map(|inner| inner.status.frame_count >= 2)
            .unwrap_or(false);
        source_cleanup.keep = preview_ready;
        self.begin_preview(stop_reason);
    }

    fn begin_preview(&self, stop_reason: Option<String>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.status.paused = false;
        if inner.status.frame_count < 2 {
            inner.frames.clear();
            inner.bytes = 0;
            inner.source_video_file = None;
            inner.status.paused = false;
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

fn active_qpc_time(
    control: &CaptureControl,
    started_at: Instant,
    qpc_origin: u64,
    frequency: u64,
) -> u64 {
    let elapsed = control.active_elapsed(started_at).as_nanos();
    let ticks = elapsed
        .saturating_mul(u128::from(frequency))
        .checked_div(1_000_000_000)
        .unwrap_or(u128::MAX)
        .min(u128::from(u64::MAX)) as u64;
    qpc_origin.saturating_add(ticks)
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

fn resize_rgba_pixels(
    pixels: &[u8],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let image = ImageBuffer::<Rgba<u8>, _>::from_raw(source_width, source_height, pixels.to_vec())
        .ok_or_else(|| "GIF 帧像素尺寸无效".to_string())?;
    Ok(image::imageops::resize(&image, width, height, FilterType::Triangle).into_raw())
}

fn temporary_recording_mp4_path() -> PathBuf {
    std::env::temp_dir().join(format!("AgileTilesRecording-{}.mp4", unique_suffix()))
}

struct RecordingSourceCleanup {
    path: PathBuf,
    keep: bool,
}

impl Drop for RecordingSourceCleanup {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
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
    frames: &mut Vec<RecordedFrame>,
    width: u32,
    height: u32,
    fps: u32,
    output_fps: u32,
    gif_export_quality: GifExportQuality,
    mp4_bitrate_mbps: u32,
    gif_colors: u16,
    webp_quality: u8,
    source_video: Option<&Path>,
    source_duration_ms: u64,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    progress.store(1, Ordering::Release);
    let temp_path = sibling_temp_path(path);
    let mut temp_output_cleanup = TempOutputCleanup {
        path: temp_path.clone(),
        armed: true,
    };
    let result = match format {
        AnimationFormat::Webp => {
            if let Some(source_video) = source_video {
                #[cfg(windows)]
                {
                    retain_preview_fallback(frames);
                    encode_webp_from_mp4(
                        &temp_path,
                        source_video,
                        width,
                        height,
                        fps,
                        output_fps,
                        webp_quality,
                        estimated_output_frames(source_duration_ms, fps, output_fps),
                        progress,
                        cancel,
                    )
                }
                #[cfg(not(windows))]
                {
                    let _ = source_video;
                    Err("从 MP4 导出 WebP 目前只支持 Windows".into())
                }
            } else {
                encode_webp_animation(
                    frames,
                    width,
                    height,
                    fps,
                    output_fps,
                    webp_quality,
                    progress,
                    cancel,
                )
                    .and_then(|bytes| fs::write(&temp_path, bytes).map_err(|e| e.to_string()))
            }
        }
        AnimationFormat::Gif => {
            if let Some(source_video) = source_video {
                #[cfg(windows)]
                {
                    let (gif_width, gif_height) =
                        gif_export_dimensions(width, height, output_fps, gif_export_quality);
                    encode_gif_from_mp4(
                        &temp_path,
                        source_video,
                        frames,
                        width,
                        height,
                        gif_width,
                        gif_height,
                        fps,
                        output_fps,
                        gif_colors,
                        gif_diff_tolerance(gif_export_quality),
                        estimated_output_frames(source_duration_ms, fps, output_fps),
                        progress,
                        cancel,
                    )
                }
                #[cfg(not(windows))]
                {
                    let _ = source_video;
                    Err("从 MP4 导出 GIF 目前只支持 Windows".into())
                }
            } else {
                let (gif_width, gif_height) =
                    gif_export_dimensions(width, height, output_fps, gif_export_quality);
                encode_gif(
                    &temp_path,
                    frames,
                    width,
                    height,
                    gif_width,
                    gif_height,
                    fps,
                    output_fps,
                    gif_colors,
                    gif_diff_tolerance(gif_export_quality),
                    progress,
                    cancel,
                )
            }
        }
        AnimationFormat::Mp4 => {
            if let Some(source_video) = source_video {
                retain_preview_fallback(frames);
                copy_file_with_progress(source_video, &temp_path, progress, cancel)
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
                    progress,
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
        return Err(cleanup_temp_output(&mut temp_output_cleanup, error));
    }
    if format == AnimationFormat::Gif {
        if let Err(error) = validate_gif_file(&temp_path) {
            return Err(cleanup_temp_output(&mut temp_output_cleanup, error));
        }
    }
    let output_size = match fs::metadata(&temp_path) {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return Err(cleanup_temp_output(
                &mut temp_output_cleanup,
                format!("无法读取导出文件：{error}"),
            ));
        }
    };
    if output_size == 0 {
        return Err(cleanup_temp_output(
            &mut temp_output_cleanup,
            "导出文件为空，原文件已保留".into(),
        ));
    }
    if cancel.load(Ordering::Acquire) {
        return Err(cleanup_temp_output(
            &mut temp_output_cleanup,
            "导出已取消".into(),
        ));
    }
    replace_file(&temp_path, path).map_err(|error| {
            cleanup_temp_output(
                &mut temp_output_cleanup,
                format!("无法替换目标文件：{error}"),
            )
        })?;
    progress.store(100, Ordering::Release);
    Ok(())
}

fn estimated_output_frames(source_duration_ms: u64, source_fps: u32, output_fps: u32) -> usize {
    let source_fps = u64::from(source_fps.max(1));
    let source_frame_ms = 1000u64.saturating_add(source_fps - 1) / source_fps;
    let duration_ms = source_duration_ms.saturating_add(source_frame_ms.max(1));
    let frames = (u128::from(duration_ms) * u128::from(output_fps.max(1))).div_ceil(1000);
    frames.max(1).min(usize::MAX as u128) as usize
}

fn update_export_progress(progress: &AtomicU32, completed: usize, total: usize) {
    let total = total.max(1) as u128;
    let completed = completed.min(total as usize) as u128;
    let percent = 5 + (completed * 90 / total) as u32;
    progress.store(percent.min(95), Ordering::Release);
}

fn copy_file_with_progress(
    source: &Path,
    destination: &Path,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut input = File::open(source).map_err(|error| format!("无法打开录制文件：{error}"))?;
    let total = input
        .metadata()
        .map_err(|error| format!("无法读取录制文件信息：{error}"))?
        .len();
    let output = File::create(destination).map_err(|error| format!("无法创建导出文件：{error}"))?;
    let mut output = BufWriter::new(output);
    let mut buffer = [0u8; 1024 * 1024];
    let mut copied = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("导出已取消".into());
        }
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("读取录制文件失败：{error}"))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("写入导出文件失败：{error}"))?;
        copied = copied.saturating_add(read as u64);
        if total > 0 {
            update_export_progress(
                progress,
                copied.min(usize::MAX as u64) as usize,
                total.min(usize::MAX as u64) as usize,
            );
        }
    }
    output.flush().map_err(|error| format!("刷新导出文件失败：{error}"))?;
    output
        .get_ref()
        .sync_all()
        .map_err(|error| format!("保存导出文件失败：{error}"))?;
    Ok(())
}

fn fit_recording_dimensions(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (1, 1);
    }
    let scale = (max_width as f64 / width as f64)
        .min(max_height as f64 / height as f64)
        .min(1.0);
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

fn gif_export_dimensions(
    width: u32,
    height: u32,
    fps: u32,
    quality: GifExportQuality,
) -> (u32, u32) {
    let (max_width, max_height) = match quality {
        GifExportQuality::Recommended => gif_max_output_dimensions(width, height, fps),
        GifExportQuality::High => (1920, 1080),
        GifExportQuality::Balanced => (1280, 720),
        GifExportQuality::Small => (960, 540),
    };
    fit_recording_dimensions(width, height, max_width, max_height)
}

fn gif_max_output_dimensions(width: u32, height: u32, fps: u32) -> (u32, u32) {
    if fps >= 30 {
        if width > MAX_GIF_HIGH_FPS_OUTPUT_WIDTH || height > MAX_GIF_HIGH_FPS_OUTPUT_HEIGHT {
            (
                MAX_GIF_LARGE_HIGH_FPS_OUTPUT_WIDTH,
                MAX_GIF_LARGE_HIGH_FPS_OUTPUT_HEIGHT,
            )
        } else {
            (
                MAX_GIF_HIGH_FPS_OUTPUT_WIDTH,
                MAX_GIF_HIGH_FPS_OUTPUT_HEIGHT,
            )
        }
    } else if width > MAX_GIF_OUTPUT_WIDTH || height > MAX_GIF_OUTPUT_HEIGHT {
        (MAX_GIF_LARGE_OUTPUT_WIDTH, MAX_GIF_LARGE_OUTPUT_HEIGHT)
    } else {
        (MAX_GIF_OUTPUT_WIDTH, MAX_GIF_OUTPUT_HEIGHT)
    }
}

fn gif_diff_tolerance(quality: GifExportQuality) -> i32 {
    match quality {
        GifExportQuality::High => 0,
        GifExportQuality::Recommended => 3,
        GifExportQuality::Balanced => 6,
        GifExportQuality::Small => 9,
    }
}

fn encode_gif(
    path: &Path,
    frames: &[RecordedFrame],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    source_fps: u32,
    output_fps: u32,
    color_count: u16,
    diff_tolerance: i32,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if frames.is_empty()
        || width == 0
        || height == 0
        || width > u16::MAX as u32
        || height > u16::MAX as u32
    {
        return Err("GIF 尺寸或帧数据无效".into());
    }
    let flush_interval = output_fps.max(1) as usize;
    let quantizer = GifQuantizer::from_frames(frames, color_count)?;
    let schedule = frame_schedule(frames, source_fps, output_fps);
    let total_frames = schedule.len();
    update_export_progress(progress, 0, total_frames);
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    {
        let mut encoder =
            GifEncoder::new(&mut writer, width as u16, height as u16, &quantizer.palette)
                .map_err(|e| e.to_string())?;
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        let mut processed_frames = 0usize;
        let mut encoded_frames = 0usize;
        let mut previous_pixels = None;
        let mut pending_frame = None;
        let mut final_timestamp_ms = 0u64;
        for (frame_index, timestamp_ms, duration_ms) in schedule {
            if cancel.load(Ordering::Acquire) {
                return Err("导出已取消".into());
            }
            let frame = &frames[frame_index];
            let resized = (source_width != width || source_height != height)
                .then(|| {
                    resize_rgba_pixels(&frame.pixels, source_width, source_height, width, height)
                })
                .transpose()?;
            let pixels = resized.as_deref().unwrap_or(&frame.pixels);
            if let Some(delta) = make_gif_delta_frame(
                pixels,
                previous_pixels.as_deref(),
                width,
                height,
                diff_tolerance,
                quantizer.transparent_index,
                &quantizer,
            )? {
                if let Some(previous) = pending_frame.take() {
                    encoded_frames += write_pending_gif_frame(
                        &mut encoder,
                        previous,
                        timestamp_ms,
                        quantizer.transparent_index,
                    )?;
                }
                pending_frame = Some(PendingGifFrame {
                    timestamp_ms,
                    delta,
                });
            }
            previous_pixels = Some(pixels.to_vec());
            final_timestamp_ms = timestamp_ms.saturating_add(u64::from(duration_ms));
            processed_frames += 1;
            update_export_progress(progress, processed_frames, total_frames);
            if processed_frames % flush_interval == 0 {
                encoder
                    .get_mut()
                    .flush()
                    .map_err(|error| error.to_string())?;
            }
        }
        if let Some(pending) = pending_frame {
            encoded_frames += write_pending_gif_frame(
                &mut encoder,
                pending,
                final_timestamp_ms,
                quantizer.transparent_index,
            )?;
        }
        if encoded_frames == 0 {
            return Err("GIF 没有可导出的画面帧".into());
        }
        encoder.into_inner().map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())?;
    writer.get_ref().sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

struct GifQuantizer {
    palette: Vec<u8>,
    quantizer: NeuQuant,
    transparent_index: u8,
}

impl GifQuantizer {
    fn from_frames(frames: &[RecordedFrame], color_count: u16) -> Result<Self, String> {
        const MAX_SAMPLE_FRAMES: usize = 24;
        const MAX_SAMPLES_PER_FRAME: usize = 4096;

        if ![64, 128, 256].contains(&color_count) {
            return Err("GIF 色彩数设置无效".into());
        }
        let valid_frames: Vec<_> = frames
            .iter()
            .filter(|frame| frame.pixels.len() >= 4)
            .collect();
        if valid_frames.is_empty() {
            return Err("GIF 调色板没有可用帧".into());
        }
        let frame_step = valid_frames.len().div_ceil(MAX_SAMPLE_FRAMES).max(1);
        let sample_frame_count = valid_frames.len().div_ceil(frame_step);
        let mut samples = Vec::with_capacity(sample_frame_count * MAX_SAMPLES_PER_FRAME * 4);
        for frame in valid_frames.iter().step_by(frame_step) {
            let pixels = frame.pixels.chunks_exact(4);
            let sample_step = pixels.len().div_ceil(MAX_SAMPLES_PER_FRAME).max(1);
            for pixel in pixels.step_by(sample_step).take(MAX_SAMPLES_PER_FRAME) {
                samples.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        if samples.is_empty() {
            return Err("GIF 调色板采样失败".into());
        }

        let transparent_index = u8::try_from(color_count - 1).map_err(|_| "GIF 透明色索引无效")?;
        let quantizer = NeuQuant::new(10, usize::from(color_count - 1), &samples);
        let mut palette = quantizer.color_map_rgb();
        palette.extend_from_slice(&[0, 0, 0]);
        if palette.len() != usize::from(color_count) * 3 {
            return Err("GIF 调色板生成失败".into());
        }
        Ok(Self {
            palette,
            quantizer,
            transparent_index,
        })
    }

    fn index_pixel(&self, rgba: &[u8]) -> Result<u8, String> {
        if rgba.len() < 4 {
            return Err("GIF 帧像素数据无效".into());
        }
        let index = self.quantizer.index_of(rgba);
        if index >= usize::from(self.transparent_index) {
            return Err("GIF 调色板索引无效".into());
        }
        u8::try_from(index).map_err(|_| "GIF 调色板索引无效".into())
    }
}

// Differential rectangles and transparent unchanged pixels follow the MIT-licensed
// GIF encoding approach used by Winshotx; this implementation stays sequential.
#[derive(Debug)]
struct GifDeltaFrame {
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    pixels: Vec<u8>,
    uses_transparency: bool,
}

struct PendingGifFrame {
    timestamp_ms: u64,
    delta: GifDeltaFrame,
}

fn make_gif_delta_frame(
    current: &[u8],
    previous: Option<&[u8]>,
    width: u32,
    height: u32,
    tolerance: i32,
    transparent_index: u8,
    quantizer: &GifQuantizer,
) -> Result<Option<GifDeltaFrame>, String> {
    let expected_length = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "GIF 帧尺寸过大".to_string())? as usize;
    if width == 0
        || height == 0
        || width > u16::MAX as u32
        || height > u16::MAX as u32
        || current.len() != expected_length
        || previous.is_some_and(|pixels| pixels.len() != expected_length)
    {
        return Err("GIF 帧像素数据无效".into());
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut changed = false;
    let row_bytes = width as usize * 4;
    for y in 0..height {
        let row_start = y as usize * row_bytes;
        let current_row = &current[row_start..row_start + row_bytes];
        let previous_row = previous.map(|pixels| &pixels[row_start..row_start + row_bytes]);
        if previous_row.is_some_and(|pixels| pixels == current_row) {
            continue;
        }
        for (x, current_pixel) in current_row.chunks_exact(4).enumerate() {
            if previous_row.is_none_or(|pixels| {
                let pixel_start = x * 4;
                gif_pixel_changed(
                    current_pixel,
                    &pixels[pixel_start..pixel_start + 4],
                    tolerance,
                )
            }) {
                changed = true;
                min_x = min_x.min(x as u32);
                min_y = min_y.min(y);
                max_x = max_x.max(x as u32);
                max_y = max_y.max(y);
            }
        }
    }
    if !changed {
        return Ok(None);
    }

    let delta_width = max_x - min_x + 1;
    let delta_height = max_y - min_y + 1;
    let delta_length = (delta_width as usize)
        .checked_mul(delta_height as usize)
        .ok_or_else(|| "GIF 差分区域过大".to_string())?;
    let mut delta_pixels = vec![transparent_index; delta_length];
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let source_offset = (y as usize * width as usize + x as usize) * 4;
            let current_pixel = &current[source_offset..source_offset + 4];
            let is_changed = previous.is_none_or(|pixels| {
                gif_pixel_changed(
                    current_pixel,
                    &pixels[source_offset..source_offset + 4],
                    tolerance,
                )
            });
            if is_changed {
                let target_offset = ((y - min_y) * delta_width + (x - min_x)) as usize;
                delta_pixels[target_offset] = quantizer.index_pixel(current_pixel)?;
            }
        }
    }

    Ok(Some(GifDeltaFrame {
        left: u16::try_from(min_x).map_err(|_| "GIF 差分位置无效")?,
        top: u16::try_from(min_y).map_err(|_| "GIF 差分位置无效")?,
        width: u16::try_from(delta_width).map_err(|_| "GIF 差分宽度无效")?,
        height: u16::try_from(delta_height).map_err(|_| "GIF 差分高度无效")?,
        pixels: delta_pixels,
        uses_transparency: previous.is_some(),
    }))
}

fn gif_pixel_changed(current: &[u8], previous: &[u8], tolerance: i32) -> bool {
    let rgb_difference = (i32::from(current[0]) - i32::from(previous[0])).abs()
        + (i32::from(current[1]) - i32::from(previous[1])).abs()
        + (i32::from(current[2]) - i32::from(previous[2])).abs();
    rgb_difference > tolerance
}

fn gif_delay_centiseconds(start_ms: u64, end_ms: u64) -> u64 {
    let start_cs = start_ms.saturating_add(5) / 10;
    let end_cs = end_ms.saturating_add(5) / 10;
    end_cs.saturating_sub(start_cs).max(2)
}

fn write_pending_gif_frame<W: Write>(
    encoder: &mut GifEncoder<W>,
    pending: PendingGifFrame,
    end_timestamp_ms: u64,
    transparent_index: u8,
) -> Result<usize, String> {
    let mut delay_cs = gif_delay_centiseconds(pending.timestamp_ms, end_timestamp_ms);
    let mut frame = Some(GifFrame::from_indexed_pixels(
        pending.delta.width,
        pending.delta.height,
        pending.delta.pixels,
        None,
    ));
    if let Some(frame) = frame.as_mut() {
        frame.left = pending.delta.left;
        frame.top = pending.delta.top;
        frame.transparent = pending.delta.uses_transparency.then_some(transparent_index);
        frame.dispose = DisposalMethod::Keep;
    }

    let mut written = 0usize;
    while delay_cs != 0 {
        let delay = delay_cs.min(u64::from(u16::MAX)) as u16;
        if let Some(mut frame) = frame.take() {
            frame.delay = delay;
            encoder
                .write_frame(&frame)
                .map_err(|error| error.to_string())?;
        } else {
            write_gif_hold_frame(encoder, delay, transparent_index)?;
        }
        written += 1;
        delay_cs -= u64::from(delay);
    }
    Ok(written)
}

fn write_gif_hold_frame<W: Write>(
    encoder: &mut GifEncoder<W>,
    delay: u16,
    transparent_index: u8,
) -> Result<(), String> {
    let mut frame = GifFrame::from_indexed_pixels(1, 1, vec![transparent_index], None);
    frame.delay = delay;
    frame.transparent = Some(transparent_index);
    frame.dispose = DisposalMethod::Keep;
    encoder
        .write_frame(&frame)
        .map_err(|error| error.to_string())
}

fn retain_preview_fallback(frames: &mut Vec<RecordedFrame>) {
    frames.truncate(1);
    frames.shrink_to_fit();
}

#[cfg(windows)]
fn encode_gif_from_mp4(
    path: &Path,
    source_video: &Path,
    palette_frames: &mut Vec<RecordedFrame>,
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    source_fps: u32,
    output_fps: u32,
    color_count: u16,
    diff_tolerance: i32,
    expected_output_frames: usize,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if width == 0 || height == 0 || width > u16::MAX as u32 || height > u16::MAX as u32 {
        return Err("GIF 尺寸或帧数据无效".into());
    }
    let flush_interval = output_fps.max(1) as usize;
    let quantizer = GifQuantizer::from_frames(palette_frames, color_count)?;
    retain_preview_fallback(palette_frames);
    update_export_progress(progress, 0, expected_output_frames);
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    {
        let mut encoder =
            GifEncoder::new(&mut writer, width as u16, height as u16, &quantizer.palette)
                .map_err(|e| e.to_string())?;
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        let mut processed_frames = 0usize;
        let mut encoded_frames = 0usize;
        let mut previous_pixels = None;
        let mut pending_frame = None;
        let mut final_timestamp_ms = 0u64;
        recording_mp4::visit_mp4_frames(
            source_video,
            source_width,
            source_height,
            source_fps,
            output_fps,
            cancel,
            |timestamp_ms, duration_ms, pixels| {
                let resized = (source_width != width || source_height != height)
                    .then(|| resize_rgba_pixels(pixels, source_width, source_height, width, height))
                    .transpose()?;
                let pixels = resized.as_deref().unwrap_or(pixels);
                if let Some(delta) = make_gif_delta_frame(
                    pixels,
                    previous_pixels.as_deref(),
                    width,
                    height,
                    diff_tolerance,
                    quantizer.transparent_index,
                    &quantizer,
                )? {
                    if let Some(previous) = pending_frame.take() {
                        encoded_frames += write_pending_gif_frame(
                            &mut encoder,
                            previous,
                            timestamp_ms,
                            quantizer.transparent_index,
                        )?;
                    }
                    pending_frame = Some(PendingGifFrame {
                        timestamp_ms,
                        delta,
                    });
                }
                previous_pixels = Some(pixels.to_vec());
                final_timestamp_ms = timestamp_ms.saturating_add(duration_ms);
                processed_frames += 1;
                update_export_progress(progress, processed_frames, expected_output_frames);
                if processed_frames % flush_interval == 0 {
                    encoder
                        .get_mut()
                        .flush()
                        .map_err(|error| error.to_string())?;
                }
                Ok(())
            },
        )?;
        if let Some(pending) = pending_frame {
            encoded_frames += write_pending_gif_frame(
                &mut encoder,
                pending,
                final_timestamp_ms,
                quantizer.transparent_index,
            )?;
        }
        if encoded_frames == 0 {
            return Err("GIF 没有可导出的画面帧".into());
        }
        encoder.into_inner().map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())?;
    writer.get_ref().sync_all().map_err(|e| e.to_string())
}

fn encode_webp_frame_chunks(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Result<Vec<u8>, String> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "WebP 帧尺寸过大".to_string())? as usize;
    if width == 0 || height == 0 || pixels.len() != expected || !(50..=95).contains(&quality) {
        return Err("WebP 画质或帧数据无效".into());
    }
    let image = webp_rust::ImageBuffer {
        width: width as usize,
        height: height as usize,
        rgba: pixels.to_vec(),
    };
    let config = webp_rust::LossyEncodingConfig {
        quality: f32::from(quality),
        method: 4,
        ..Default::default()
    };
    let encoded = webp_rust::encode_lossy_with_config(&image, &config, None)
        .map_err(|_| "WebP 帧编码失败".to_string())?;
    if encoded.is_empty() {
        return Err("WebP 帧编码结果为空".into());
    }
    extract_webp_frame_chunks(&encoded)
}

fn encode_webp_animation(
    frames: &[RecordedFrame],
    width: u32,
    height: u32,
    source_fps: u32,
    output_fps: u32,
    quality: u8,
    progress: &AtomicU32,
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

    let schedule = frame_schedule(frames, source_fps, output_fps);
    let total_frames = schedule.len();
    update_export_progress(progress, 0, total_frames);
    let mut written_frames = 0usize;
    for (frame_index, _, duration_ms) in schedule {
        if cancel.load(Ordering::Acquire) {
            return Err("导出已取消".into());
        }
        let frame = &frames[frame_index];
        let frame_chunks = encode_webp_frame_chunks(&frame.pixels, width, height, quality)?;
        let mut payload = Vec::with_capacity(16 + frame_chunks.len());
        push_u24(&mut payload, 0)?;
        push_u24(&mut payload, 0)?;
        push_u24(&mut payload, width - 1)?;
        push_u24(&mut payload, height - 1)?;
        push_u24(&mut payload, duration_ms.min(WEBP_CHUNK_LIMIT as u32))?;
        payload.push(0x02); // Do not blend; every frame covers the full canvas.
        payload.extend_from_slice(&frame_chunks);
        append_riff_chunk(&mut chunks, *b"ANMF", &payload)?;
        written_frames += 1;
        update_export_progress(progress, written_frames, total_frames);
    }
    if written_frames == 0 {
        return Err("WebP 没有可导出的画面帧".into());
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
    quality: u8,
    expected_output_frames: usize,
    progress: &AtomicU32,
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

    let mut written_frames = 0usize;
    recording_mp4::visit_mp4_frames(
        source_video,
        width,
        height,
        source_fps,
        output_fps,
        cancel,
        |_, duration_ms, pixels| {
            let frame_chunks = encode_webp_frame_chunks(pixels, width, height, quality)?;
            let mut payload = Vec::with_capacity(16 + frame_chunks.len());
            push_u24(&mut payload, 0)?;
            push_u24(&mut payload, 0)?;
            push_u24(&mut payload, width - 1)?;
            push_u24(&mut payload, height - 1)?;
            push_u24(&mut payload, duration_ms.min(WEBP_CHUNK_LIMIT as u64) as u32)?;
            payload.push(0x02);
            payload.extend_from_slice(&frame_chunks);
            write_riff_chunk(&mut writer, *b"ANMF", &payload, &mut riff_size)?;
            written_frames += 1;
            update_export_progress(progress, written_frames, expected_output_frames);
            Ok(())
        },
    )?;
    if written_frames == 0 {
        return Err("WebP 没有可导出的画面帧".into());
    }

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

struct TempOutputCleanup {
    path: PathBuf,
    armed: bool,
}

impl TempOutputCleanup {
    fn cleanup(&mut self) -> Result<(), String> {
        let result = remove_temp_output(&self.path);
        if result.is_ok() {
            self.armed = false;
        }
        result
    }
}

impl Drop for TempOutputCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = remove_temp_output(&self.path);
        }
    }
}

fn cleanup_temp_output(cleanup: &mut TempOutputCleanup, primary_error: String) -> String {
    match cleanup.cleanup() {
        Ok(()) => primary_error,
        Err(cleanup_error) => format!("{primary_error}；{cleanup_error}"),
    }
}

fn remove_temp_output(path: &Path) -> Result<(), String> {
    let mut last_error = None;
    for attempt in 0..3 {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt < 2 {
                    thread::sleep(Duration::from_millis(25));
                }
            }
        }
    }
    Err(format!(
        "无法清理临时文件 {}：{}",
        path.display(),
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "未知错误".into())
    ))
}

fn validate_gif_file(path: &Path) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| format!("无法打开临时 GIF：{error}"))?;
    let file_len = file
        .metadata()
        .map_err(|error| format!("无法读取临时 GIF：{error}"))?
        .len();
    if file_len < 14 {
        return Err("GIF 导出文件过短，原文件已保留".into());
    }

    let mut header = [0u8; 13];
    file.read_exact(&mut header)
        .map_err(|error| format!("GIF 文件头不完整：{error}"))?;
    if &header[..6] != b"GIF87a" && &header[..6] != b"GIF89a" {
        return Err("GIF 文件头无效，原文件已保留".into());
    }

    let global_table_size = if header[10] & 0x80 != 0 {
        3u64 << (u32::from(header[10] & 0x07) + 1)
    } else {
        0
    };
    skip_gif_bytes(&mut file, global_table_size, file_len)?;

    let mut frame_count = 0usize;
    loop {
        let mut marker = [0u8; 1];
        file.read_exact(&mut marker)
            .map_err(|error| format!("GIF 文件块不完整：{error}"))?;
        match marker[0] {
            0x3b => {
                if frame_count == 0 {
                    return Err("GIF 中没有可播放画面，原文件已保留".into());
                }
                if file
                    .stream_position()
                    .map_err(|error| error.to_string())?
                    != file_len
                {
                    return Err("GIF 结束标记后仍有无效数据，原文件已保留".into());
                }
                return Ok(());
            }
            0x21 => {
                let mut _label = [0u8; 1];
                file.read_exact(&mut _label)
                    .map_err(|error| format!("GIF 扩展块不完整：{error}"))?;
                skip_gif_sub_blocks(&mut file, file_len)?;
            }
            0x2c => {
                let mut descriptor = [0u8; 9];
                file.read_exact(&mut descriptor)
                    .map_err(|error| format!("GIF 画面描述不完整：{error}"))?;
                if descriptor[8] & 0x80 != 0 {
                    let local_table_size = 3u64 << (u32::from(descriptor[8] & 0x07) + 1);
                    skip_gif_bytes(&mut file, local_table_size, file_len)?;
                }
                let mut lzw_min_code_size = [0u8; 1];
                file.read_exact(&mut lzw_min_code_size)
                    .map_err(|error| format!("GIF 图像数据不完整：{error}"))?;
                skip_gif_sub_blocks(&mut file, file_len)?;
                frame_count = frame_count.saturating_add(1);
            }
            _ => return Err("GIF 文件块标记无效，原文件已保留".into()),
        }
    }
}

fn skip_gif_sub_blocks(file: &mut File, file_len: u64) -> Result<(), String> {
    loop {
        let mut block_len = [0u8; 1];
        file.read_exact(&mut block_len)
            .map_err(|error| format!("GIF 数据块不完整：{error}"))?;
        if block_len[0] == 0 {
            return Ok(());
        }
        skip_gif_bytes(file, u64::from(block_len[0]), file_len)?;
    }
}

fn skip_gif_bytes(file: &mut File, count: u64, file_len: u64) -> Result<(), String> {
    let current = file.stream_position().map_err(|error| error.to_string())?;
    let next = current
        .checked_add(count)
        .filter(|next| *next <= file_len)
        .ok_or_else(|| "GIF 数据超出文件长度，原文件已保留".to_string())?;
    file.seek(SeekFrom::Start(next))
        .map_err(|error| format!("读取 GIF 数据失败：{error}"))?;
    Ok(())
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

#[cfg(test)]
mod gif_export_tests {
    use super::*;

    #[test]
    fn ten_second_desktop_gif_is_smaller_and_faster_than_legacy_encoding() {
        const WIDTH: u32 = 640;
        const HEIGHT: u32 = 360;
        const FPS: u32 = 15;
        const FRAME_COUNT: usize = 150;

        assert_eq!(
            gif_export_dimensions(1920, 1080, FPS, GifExportQuality::Recommended),
            (1920, 1080)
        );
        assert_eq!(
            gif_export_dimensions(1920, 1080, 30, GifExportQuality::Recommended),
            (1280, 720)
        );

        const SCROLL_STEP: u32 = 8;
        let document_height = HEIGHT + FRAME_COUNT as u32 * SCROLL_STEP;
        let mut document = vec![0; (WIDTH * document_height * 4) as usize];
        for y in 0..document_height {
            for x in 0..WIDTH {
                let card_left = 22;
                let card_right = WIDTH - 22;
                let row_in_section = y % 180;
                let color = if x < card_left || x >= card_right {
                    [238, 240, 243, 255]
                } else if row_in_section < 28 {
                    [231, 239, 250, 255]
                } else if (8..17).contains(&(row_in_section % 28))
                    && x < 42 + (y.wrapping_mul(37) % (WIDTH - 70))
                {
                    let shade = 35 + ((x * 13 + y * 7) % 130) as u8;
                    [shade, shade, shade.saturating_add(4), 255]
                } else if (64..122).contains(&row_in_section)
                    && (76..WIDTH - 76).contains(&x)
                {
                    let red = (x * 255 / WIDTH) as u8;
                    let green = (y.wrapping_mul(5) % 256) as u8;
                    let blue = red.wrapping_add(green / 2);
                    [red, green, blue, 255]
                } else {
                    [255, 255, 255, 255]
                };
                let offset = ((y * WIDTH + x) * 4) as usize;
                document[offset..offset + 4].copy_from_slice(&color);
            }
        }
        let frames: Vec<_> = (0..FRAME_COUNT)
            .map(|index| {
                let scroll_top = index as u32 * SCROLL_STEP;
                let mut pixels = vec![0; (WIDTH * HEIGHT * 4) as usize];
                for y in 0..HEIGHT {
                    let source_start = ((scroll_top + y) * WIDTH * 4) as usize;
                    let target_start = (y * WIDTH * 4) as usize;
                    let row_len = (WIDTH * 4) as usize;
                    pixels[target_start..target_start + row_len]
                        .copy_from_slice(&document[source_start..source_start + row_len]);
                }
                RecordedFrame {
                    timestamp_ms: index as u64 * 1000 / u64::from(FPS),
                    pixels,
                }
            })
            .collect();
        assert_eq!(frame_schedule(&frames, FPS, FPS).len(), FRAME_COUNT);

        let suffix = unique_suffix();
        let legacy_path = std::env::temp_dir().join(format!("sideweave-gif-legacy-{suffix}.gif"));
        let optimized_path =
            std::env::temp_dir().join(format!("sideweave-gif-optimized-{suffix}.gif"));
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);

        let legacy_started = Instant::now();
        encode_legacy_gif_for_comparison(&legacy_path, &frames, WIDTH, HEIGHT, FPS, &cancel);
        let legacy_elapsed = legacy_started.elapsed();

        let optimized_started = Instant::now();
        encode_gif(
            &optimized_path,
            &frames,
            WIDTH,
            HEIGHT,
            WIDTH,
            HEIGHT,
            FPS,
            FPS,
            256,
            0,
            &progress,
            &cancel,
        )
        .unwrap();
        let optimized_elapsed = optimized_started.elapsed();

        let legacy_size = fs::metadata(&legacy_path).unwrap().len();
        let optimized_size = fs::metadata(&optimized_path).unwrap().len();
        let _ = fs::remove_file(&legacy_path);
        let _ = fs::remove_file(&optimized_path);

        eprintln!(
            "10s @ 15fps scrolling desktop fixture: legacy {legacy_size} bytes / {legacy_elapsed:?}; optimized {optimized_size} bytes / {optimized_elapsed:?}"
        );
        assert!(
            optimized_size < legacy_size,
            "optimized GIF ({optimized_size} bytes) should be smaller than the legacy size ({legacy_size} bytes)"
        );
        assert!(
            optimized_elapsed < legacy_elapsed,
            "optimized export ({optimized_elapsed:?}) should be faster than the legacy export ({legacy_elapsed:?})"
        );
    }

    #[test]
    fn ten_second_high_motion_gif_stays_below_one_hundred_megabytes() {
        const WIDTH: u32 = 800;
        const HEIGHT: u32 = 450;
        const FPS: u32 = 15;
        const FRAME_COUNT: usize = 150;

        let mut random = 0x7f4a_7c15u32;
        let frames: Vec<_> = (0..FRAME_COUNT)
            .map(|index| {
                let mut pixels = vec![0; (WIDTH * HEIGHT * 4) as usize];
                for pixel in pixels.chunks_exact_mut(4) {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    pixel.copy_from_slice(&[
                        random as u8,
                        (random >> 8) as u8,
                        (random >> 16) as u8,
                        255,
                    ]);
                }
                RecordedFrame {
                    timestamp_ms: index as u64 * 1000 / u64::from(FPS),
                    pixels,
                }
            })
            .collect();

        let path = std::env::temp_dir().join(format!("sideweave-gif-high-motion-{}.gif", unique_suffix()));
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);
        let started = Instant::now();
        encode_gif(
            &path,
            &frames,
            WIDTH,
            HEIGHT,
            WIDTH,
            HEIGHT,
            FPS,
            FPS,
            256,
            0,
            &progress,
            &cancel,
        )
        .unwrap();
        let elapsed = started.elapsed();
        let size = fs::metadata(&path).unwrap().len();
        let _ = fs::remove_file(path);

        eprintln!("10s @ 15fps high-motion GIF: {size} bytes / {elapsed:?}");
        assert!(
            size < 90 * 1024 * 1024,
            "high-motion GIF ({size} bytes) exceeded the 90 MiB size budget"
        );
    }

    fn encode_legacy_gif_for_comparison(
        path: &Path,
        frames: &[RecordedFrame],
        width: u32,
        height: u32,
        fps: u32,
        cancel: &AtomicBool,
    ) {
        let file = File::create(path).unwrap();
        let mut writer = BufWriter::new(file);
        {
            let mut encoder =
                GifEncoder::new(&mut writer, width as u16, height as u16, &[]).unwrap();
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for (frame_index, _, duration_ms) in frame_schedule(frames, fps, fps) {
                assert!(!cancel.load(Ordering::Acquire));
                let mut rgba = frames[frame_index].pixels.clone();
                let mut frame = GifFrame::from_rgba_speed(width as u16, height as u16, &mut rgba, 10);
                frame.delay = (duration_ms.saturating_add(5) / 10)
                    .clamp(1, u32::from(u16::MAX)) as u16;
                encoder.write_frame(&frame).unwrap();
            }
        }
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
    }
}
