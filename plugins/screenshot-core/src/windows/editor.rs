use super::output::{finish_inner, hide_capture_windows, save_png_with_dialog};
use super::paint::{create_gdi_surface, create_pixel_surface, rgba_preview_bgra, GdiSurface};
use super::scroll_ui::{
    destroy_scroll_border, destroy_scroll_masks, destroy_scroll_preview, destroy_scroll_tip,
    stop_scroll_input,
};
use super::{clear_error, invalidate, set_error, update_status, wide, UiState, TIMER_SCROLL};
use crate::capture::ScreenCapture;
use crate::export::{compose_with_options, ComposeOptions};
use crate::model::{Annotation, ScreenPoint, ScreenRect};
use crate::session::CaptureSession;
use crate::toolbar::Tool;

use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::time::Instant;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, GetSystemMetrics, KillTimer, SetTimer,
    SetForegroundWindow, ShowWindow, SW_HIDE, SW_SHOW, WS_CAPTION, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlayerHit {
    Video,
    Timeline,
    Play,
    Quality(u8),
    Fps(usize),
    CopyFrame,
    ExportWebp,
    ExportGif,
    ExportMp4,
    CancelExport,
    OpenFolder,
}

pub(super) struct PlayerMetrics {
    pub(super) scale: f32,
    pub(super) header_height: i32,
    pub(super) footer_height: i32,
    pub(super) side_pad: i32,
    pub(super) timeline_y: i32,
    pub(super) timeline_left: i32,
    pub(super) timeline_right: i32,
    pub(super) quality_group_rect: (i32, i32, i32, i32),
    pub(super) quality_rects: [(i32, i32, i32, i32); 3],
    pub(super) fps_group_rect: (i32, i32, i32, i32),
    pub(super) fps_values: Vec<u32>,
    pub(super) fps_rects: Vec<(i32, i32, i32, i32)>,
    pub(super) play_rect: (i32, i32, i32, i32),
    pub(super) open_folder_rect: Option<(i32, i32, i32, i32)>,
    pub(super) copy_rect: (i32, i32, i32, i32),
    pub(super) webp_rect: (i32, i32, i32, i32),
    pub(super) gif_rect: (i32, i32, i32, i32),
    pub(super) mp4_rect: (i32, i32, i32, i32),
    pub(super) cancel_export_rect: (i32, i32, i32, i32),
    pub(super) status_left: i32,
    pub(super) status_right: i32,
}

pub(crate) struct EditorCanvas {
    pub(super) source_width: usize,
    pub(super) source_height: usize,
    pub(super) source_pixels: Vec<u8>,
    pub(super) display_rect: ScreenRect,
    pub(super) recording_preview: bool,
    pub(super) playing: bool,
    pub(super) seeking: bool,
    pub(super) frame_index: usize,
    pub(super) playback_started_at: Option<Instant>,
    pub(super) playback_start_frame: usize,
    pub(super) export_quality_preset: u8,
    pub(super) export_fps_preset: u32,
    pub(super) player_hover: Option<PlayerHit>,
    pub(super) canvas_surface: Option<GdiSurface>,
    pub(super) back_buffer: Option<GdiSurface>,
}

pub(super) const TIMER_RECORDING_PREVIEW: usize = 43;
pub(super) const TIMER_RECORDING_EXPORT_STATUS: usize = 44;
const RECORDING_PREVIEW_FPS: u32 = 15;

pub(super) fn open_recording_editor(state: &mut UiState) {
    if state.editor.is_some() {
        return;
    }
    let Some((width, height, pixels)) = state.recording.preview_frame(0) else {
        return;
    };
    let Some(bgra) = rgba_preview_bgra(
        &pixels,
        width as i32,
        height as i32,
        width as i32,
        height as i32,
    ) else {
        set_error(
            state,
            "\u{65e0}\u{6cd5}\u{51c6}\u{5907}\u{5f55}\u{5236}\u{9884}\u{89c8}".into(),
        );
        return;
    };
    let rect = ScreenRect {
        left: 0,
        top: 0,
        right: width.min(i32::MAX as u32) as i32,
        bottom: height.min(i32::MAX as u32) as i32,
    };
    state.screen = Some(ScreenCapture {
        x: 0,
        y: 0,
        width: rect.width() as i32,
        height: rect.height() as i32,
        pixels: bgra.clone(),
        dimmed_pixels: bgra,
    });
    let settings = state
        .settings
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    let mut capture = CaptureSession::new_with_settings(rect, &settings, false);
    capture.selection = Some(rect);
    state.capture = Some(capture);
    state.recording_editor_pending = true;
    open_editor(state);
}

pub(super) fn open_editor(state: &mut UiState) {
    if state.editor.is_some() {
        return;
    }
    let Some(mut capture) = state.capture.take() else {
        return;
    };
    let Some(selection) = capture.selection else {
        state.capture = Some(capture);
        return;
    };
    let recording_preview = std::mem::take(&mut state.recording_editor_pending);

    stop_scroll_input(state);
    let scroll_mode = capture.scroll.is_some();
    let assembled = capture.scroll.as_mut().and_then(|scroll| {
        scroll.finish();
        scroll.assembled.take()
    });
    let screen = state.screen.take();
    let (source_width, source_height, source_pixels) = assembled.unwrap_or_else(|| {
        (
            selection.width(),
            selection.height(),
            screen
                .as_ref()
                .map(|value| value.crop(selection))
                .unwrap_or_default(),
        )
    });
    if source_width == 0
        || source_height == 0
        || source_pixels.len() != source_width * source_height * 4
    {
        state.capture = Some(capture);
        state.screen = screen;
        set_error(state, "无法打开图片编辑窗口：截图数据无效".into());
        return;
    }

    state.render = None;
    state.last_hover_query = None;
    if !state.overlay.is_null() {
        unsafe {
            KillTimer(state.overlay, TIMER_SCROLL);
            ShowWindow(state.overlay, SW_HIDE);
            DestroyWindow(state.overlay);
        }
        state.overlay = null_mut();
    }
    if !state.scroll_control.is_null() {
        unsafe { DestroyWindow(state.scroll_control) };
        state.scroll_control = null_mut();
    }
    destroy_scroll_preview(state);
    destroy_scroll_tip(state);
    destroy_scroll_masks(state);
    destroy_scroll_border(state);

    let dpi_scale = crate::theme::system_dpi_scale();
    let (window_width, window_height, window_x, window_y) = unsafe {
        let screen_width = GetSystemMetrics(0).max(800);
        let screen_height = GetSystemMetrics(1).max(600);
        let (min_width, max_width, min_height, max_height) = if recording_preview {
            (
                (920.0 * dpi_scale).round() as i32,
                (1180.0 * dpi_scale).round() as i32,
                (640.0 * dpi_scale).round() as i32,
                (860.0 * dpi_scale).round() as i32,
            )
        } else {
            (720, 1480, 560, 1000)
        };
        let avail_w = (screen_width - 48).max(640);
        let avail_h = (screen_height - 64).max(480);
        let width = avail_w.min(max_width).max(min_width.min(avail_w));
        let height = avail_h.min(max_height).max(min_height.min(avail_h));
        (
            width,
            height,
            (screen_width - width) / 2,
            (screen_height - height) / 2,
        )
    };
    unsafe {
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let title = wide(if recording_preview { "录制预览与导出" } else { "图片编辑" });
        let instance = GetModuleHandleW(null());
        let extended_style = if recording_preview {
            WS_EX_APPWINDOW
        } else {
            WS_EX_TOOLWINDOW
        };
        let editor_hwnd = CreateWindowExW(
            extended_style,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            window_x,
            window_y,
            window_width,
            window_height,
            null_mut(),
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        if editor_hwnd.is_null() {
            state.last_size = Some((source_width, source_height));
            state.last_image = Some((source_width, source_height, source_pixels.clone()));
            if let Ok(mut image) = state.image.lock() {
                *image = Some((source_width, source_height, source_pixels));
            }
            update_status(state, false);
            set_error(state, "无法创建图片编辑窗口".into());
            return;
        }

        let theme = state.current_theme();
        {
            use windows::Win32::Foundation::HWND as WinHwnd;
            use windows::Win32::Graphics::Dwm::{
                DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
            };
            let dark: i32 = if theme.is_dark { 1 } else { 0 };
            let _ = DwmSetWindowAttribute(
                WinHwnd(editor_hwnd as _),
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }

        state.overlay = editor_hwnd;
        let mut client: RECT = zeroed();
        GetClientRect(editor_hwnd, &mut client);
        let client_width = (client.right - client.left).max(1);
        let client_height = (client.bottom - client.top).max(1);
        let canvas_rect = ScreenRect {
            left: 0,
            top: 0,
            right: client_width,
            bottom: client_height,
        };
        let layout = super::toolbar_paint::toolbar_layout_scaled(
            capture.toolbar_button_count(),
            canvas_rect,
            capture.ui_scale,
        );
        let source_fps = state.recording.snapshot().fps.max(1);
        let player_metrics = recording_preview.then(|| {
            compute_player_metrics(client_width, client_height, source_fps, false)
        });
        let toolbar_origin = if recording_preview {
            ScreenPoint {
                x: 8,
                y: client_height - 78,
            }
        } else {
            ScreenPoint {
                x: (client_width - layout.width) / 2,
                y: 8,
            }
        };
        let image_area_top = if let Some(ref m) = player_metrics {
            m.header_height + (16.0 * m.scale).round() as i32
        } else {
            toolbar_origin.y + layout.height + 22
        };
        let image_area_width = if let Some(ref m) = player_metrics {
            (client_width - (64.0 * m.scale).round() as i32).max(1)
        } else {
            (client_width - 64).max(1)
        };
        let image_area_bottom_padding = if let Some(ref m) = player_metrics {
            m.footer_height + (38.0 * m.scale).round() as i32
        } else {
            32
        };
        let image_area_height = (client_height - image_area_top - image_area_bottom_padding).max(1);
        let scale = (image_area_width as f64 / source_width as f64)
            .min(image_area_height as f64 / source_height as f64)
            .min(1.0);
        let display_width = ((source_width as f64 * scale).round() as i32).max(1);
        let display_height = ((source_height as f64 * scale).round() as i32).max(1);
        let display_rect = ScreenRect {
            left: (client_width - display_width) / 2,
            top: image_area_top + (image_area_height - display_height) / 2,
            right: (client_width - display_width) / 2 + display_width,
            bottom: image_area_top + (image_area_height - display_height) / 2 + display_height,
        };
        let editor_scale = ((display_width as f64 / source_width as f64)
            + (display_height as f64 / source_height as f64))
            / 2.0;
        let display_size = |size: u32| ((size as f64 * editor_scale).round() as u32).max(1);
        let Some(scaled_bgra) = rgba_preview_bgra(
            &source_pixels,
            source_width as i32,
            source_height as i32,
            display_width,
            display_height,
        ) else {
            DestroyWindow(editor_hwnd);
            state.overlay = null_mut();
            state.last_size = Some((source_width, source_height));
            state.last_image = Some((source_width, source_height, source_pixels.clone()));
            if let Ok(mut image) = state.image.lock() {
                *image = Some((source_width, source_height, source_pixels));
            }
            update_status(state, false);
            set_error(state, "无法准备图片编辑画布".into());
            return;
        };

        let canvas_len = (client_width as usize)
            .saturating_mul(client_height as usize)
            .saturating_mul(4);
        // Theme-aware studio backdrop in BGRA order
        let bg_pixel: [u8; 4] = if recording_preview {
            if theme.is_dark {
                [0x17, 0x14, 0x14, 0xFF]
            } else {
                [0xF2, 0xEE, 0xEC, 0xFF]
            }
        } else if theme.is_dark {
            [0x22, 0x1E, 0x1E, 0xFF]
        } else {
            [0xF5, 0xF5, 0xF5, 0xFF]
        };
        let mut canvas = vec![0u8; canvas_len];
        for px in canvas.chunks_exact_mut(4) {
            px.copy_from_slice(&bg_pixel);
        }
        for row in 0..display_height as usize {
            let source = row * display_width as usize * 4;
            let target = (((display_rect.top as usize + row) * client_width as usize)
                + display_rect.left as usize)
                * 4;
            let row_len = display_width as usize * 4;
            canvas[target..target + row_len]
                .copy_from_slice(&scaled_bgra[source..source + row_len]);
        }

        // Keep the existing CaptureSession and its annotation handlers as the
        // editor. Only replace its desktop background with a fitted image canvas.
        let settings = state
            .settings
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        let mut editor_capture = CaptureSession::new_with_settings(canvas_rect, &settings, false);
        editor_capture.annotation_color = capture.annotation_color;
        editor_capture.selection = Some(display_rect);
        editor_capture.tool = if recording_preview {
            Tool::Select
        } else {
            Tool::Rectangle
        };
        editor_capture.toolbar_origin = Some(toolbar_origin);
        editor_capture.stroke_width = display_size(capture.stroke_width);
        editor_capture.brush_size = display_size(capture.brush_size);
        editor_capture.text_size = display_size(capture.text_size);
        editor_capture.stroke_style = capture.stroke_style;
        editor_capture.corner_radius = capture.corner_radius;
        editor_capture.drop_shadow = capture.drop_shadow;
        editor_capture.shape_filled = capture.shape_filled;
        editor_capture.shape_rounded = capture.shape_rounded;
        editor_capture.watermark_text = capture.watermark_text.clone();
        if !scroll_mode {
            editor_capture.annotations = capture
                .annotations
                .iter()
                .map(|annotation| map_annotation(annotation, selection, display_rect))
                .collect();
        }
        editor_capture.number_next = capture.number_next;

        state.screen = Some(ScreenCapture {
            x: 0,
            y: 0,
            width: client_width,
            height: client_height,
            pixels: canvas.clone(),
            dimmed_pixels: canvas,
        });
        let (canvas_surface, back_buffer) = if let Some(screen) = state.screen.as_ref() {
            let reference = GetDC(editor_hwnd);
            if reference.is_null() {
                (None, None)
            } else {
                let canvas_surface = (!recording_preview)
                    .then(|| {
                        create_pixel_surface(reference, client_width, client_height, &screen.pixels)
                    })
                    .flatten();
                let back_buffer = create_gdi_surface(reference, client_width, client_height);
                ReleaseDC(editor_hwnd, reference);
                (canvas_surface, back_buffer)
            }
        } else {
            (None, None)
        };
        let default_fps_preset = default_export_fps_preset(source_fps);
        state.capture = Some(editor_capture);
        state.editor = Some(EditorCanvas {
            source_width,
            source_height,
            source_pixels: source_pixels.clone(),
            display_rect,
            recording_preview,
            playing: false,
            seeking: false,
            frame_index: 0,
            playback_started_at: None,
            playback_start_frame: 0,
            export_quality_preset: 1, // 推荐 720p
            export_fps_preset: default_fps_preset,
            player_hover: None,
            canvas_surface,
            back_buffer,
        });
        state.last_size = Some((source_width, source_height));
        state.last_image = Some((source_width, source_height, source_pixels.clone()));
        if let Ok(mut image) = state.image.lock() {
            *image = Some((source_width, source_height, source_pixels));
        }
        state.paint_pending = false;
        ShowWindow(editor_hwnd, SW_SHOW);
        SetForegroundWindow(editor_hwnd);
        SetFocus(editor_hwnd);
    }
    update_status(state, true);
    clear_error(state);
}

pub(super) fn advance_recording_preview(state: &mut UiState) {
    let Some((
        frame_index,
        display_rect,
        playing,
        seeking,
        playback_started_at,
        playback_start_frame,
        fps,
    )) = state
        .editor
        .as_ref()
        .filter(|editor| editor.recording_preview)
        .and_then(|editor| {
            state.screen.as_ref()?;
            Some((
                editor.frame_index,
                editor.display_rect,
                editor.playing,
                editor.seeking,
                editor.playback_started_at,
                editor.playback_start_frame,
                state.recording.snapshot().fps.max(1),
            ))
        })
    else {
        return;
    };
    if !playing || seeking {
        return;
    }
    let frame_count = state.recording.snapshot().timeline_frame_count();
    if frame_count == 0 {
        return;
    }
    let elapsed_ms = playback_started_at
        .map(|started| started.elapsed().as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0);
    let elapsed_frames = (u128::from(elapsed_ms) * u128::from(fps) / 1000)
        .min(usize::MAX as u128) as usize;
    let target_frame = playback_start_frame.saturating_add(elapsed_frames);
    let reached_end = target_frame >= frame_count.saturating_sub(1);
    let target_frame = target_frame.min(frame_count.saturating_sub(1));
    if target_frame == frame_index && !reached_end {
        return;
    }
    let Some((decoded_frame, width, height, pixels)) =
        state.recording.request_preview_frame(target_frame)
    else {
        return;
    };
    let playback_finished =
        reached_end && decoded_frame.saturating_add(1) >= frame_count.saturating_sub(1);
    if decoded_frame == frame_index {
        if playback_finished {
            if let Some(editor) = state.editor.as_mut() {
                editor.playing = false;
                editor.playback_started_at = None;
            }
            unsafe { KillTimer(state.overlay, TIMER_RECORDING_PREVIEW) };
            super::invalidate(state);
        }
        return;
    }
    let Some(screen) = state.screen.as_mut() else {
        return;
    };
    if !recording_canvas(
        &pixels,
        width as i32,
        height as i32,
        screen.width,
        screen.height,
        display_rect,
        &mut screen.pixels,
    ) {
        return;
    }
    if let Some(editor) = state.editor.as_mut() {
        editor.source_width = width as usize;
        editor.source_height = height as usize;
        editor.source_pixels = pixels;
        editor.frame_index = decoded_frame;
        if playback_finished {
            editor.playing = false;
            editor.playback_started_at = None;
        }
    }
    if playback_finished {
        unsafe { KillTimer(state.overlay, TIMER_RECORDING_PREVIEW) };
    }
    super::invalidate(state);
}

pub(super) fn is_recording_preview(state: &UiState) -> bool {
    state
        .editor
        .as_ref()
        .is_some_and(|editor| editor.recording_preview)
}

pub(super) fn begin_recording_preview_seek(state: &mut UiState, point: ScreenPoint) -> bool {
    if !is_recording_preview(state) || !near_recording_timeline(state, point) {
        return false;
    }
    if let Some(editor) = state.editor.as_mut() {
        editor.seeking = true;
        editor.playing = false;
        editor.playback_started_at = None;
    }
    unsafe { KillTimer(state.overlay, TIMER_RECORDING_PREVIEW) };
    seek_timeline_to_x(state, point.x);
    true
}

pub(super) fn recording_fps_presets(source_fps: u32) -> Vec<u32> {
    let source = source_fps.max(1);
    let mut presets: Vec<u32> = if source <= 10 {
        vec![5, 8, source]
    } else if source <= 15 {
        vec![10, 12, source]
    } else {
        vec![10, 15, source]
    };
    presets.retain(|f| *f <= source && crate::model::RECORDING_FPS_OPTIONS.contains(f));
    presets.sort_unstable();
    presets.dedup();
    if presets.is_empty() {
        presets.push(source);
    }
    presets
}

fn default_export_fps_preset(source_fps: u32) -> u32 {
    let presets = recording_fps_presets(source_fps);
    // Prefer 12 fps (or 15 fps if source > 15) for fast & smooth GIF/WebP export
    if presets.contains(&12) {
        12
    } else if presets.contains(&15) {
        15
    } else {
        *presets.last().unwrap_or(&source_fps.max(1))
    }
}

pub(super) fn compute_player_metrics(
    width: i32,
    height: i32,
    source_fps: u32,
    show_open_folder: bool,
) -> PlayerMetrics {
    let s = crate::theme::system_dpi_scale();
    let px = |dp: f32| (dp * s).round() as i32;

    let header_height = px(62.0);
    let footer_height = px(74.0);
    let side_pad = px(24.0);
    let footer_top = height - footer_height;
    let timeline_y = footer_top - px(16.0);
    let timeline_left = side_pad;
    let timeline_right = (width - side_pad).max(timeline_left + 1);

    // Header right segmented controls: [ 540p | 720p | 1080p ]  [ 10fps | 12fps | 15fps ]
    let pill_h = px(30.0);
    let seg_pad = px(3.0);
    let group_h = pill_h + seg_pad * 2;
    let group_top = (header_height - group_h) / 2;
    let group_bottom = group_top + group_h;

    let fps_values = recording_fps_presets(source_fps);
    let fps_item_w = px(52.0);
    let fps_group_w = fps_item_w * fps_values.len() as i32 + seg_pad * 2;
    let fps_group_right = width - side_pad;
    let fps_group_left = (fps_group_right - fps_group_w).max(side_pad);
    let fps_group_rect = (fps_group_left, group_top, fps_group_right, group_bottom);
    let mut fps_rects = Vec::with_capacity(fps_values.len());
    for i in 0..fps_values.len() {
        let l = fps_group_left + seg_pad + i as i32 * fps_item_w;
        fps_rects.push((l, group_top + seg_pad, l + fps_item_w, group_bottom - seg_pad));
    }

    let q_item_w = px(66.0);
    let q_group_w = q_item_w * 3 + seg_pad * 2;
    let q_group_right = fps_group_left - px(10.0);
    let q_group_left = (q_group_right - q_group_w).max(side_pad);
    let quality_group_rect = (q_group_left, group_top, q_group_right, group_bottom);
    let quality_rects = [
        (q_group_left + seg_pad, group_top + seg_pad, q_group_left + seg_pad + q_item_w, group_bottom - seg_pad),
        (q_group_left + seg_pad + q_item_w, group_top + seg_pad, q_group_left + seg_pad + q_item_w * 2, group_bottom - seg_pad),
        (q_group_left + seg_pad + q_item_w * 2, group_top + seg_pad, q_group_left + seg_pad + q_item_w * 3, group_bottom - seg_pad),
    ];

    // Footer action dock
    let btn_h = px(40.0);
    let btn_top = footer_top + (footer_height - btn_h) / 2;
    let btn_bottom = btn_top + btn_h;
    let btn_gap = px(10.0);

    let play_size = btn_h;
    let play_rect = (side_pad, btn_top, side_pad + play_size, btn_bottom);

    let mp4_w = px(98.0);
    let gif_w = px(90.0);
    let webp_w = px(98.0);
    let copy_w = px(78.0);
    let cancel_w = px(100.0);

    let mp4_right = width - side_pad;
    let mp4_left = mp4_right - mp4_w;
    let mp4_rect = (mp4_left, btn_top, mp4_right, btn_bottom);

    let gif_right = mp4_left - btn_gap;
    let gif_left = gif_right - gif_w;
    let gif_rect = (gif_left, btn_top, gif_right, btn_bottom);

    let webp_right = gif_left - btn_gap;
    let webp_left = webp_right - webp_w;
    let webp_rect = (webp_left, btn_top, webp_right, btn_bottom);

    let copy_right = webp_left - btn_gap;
    let copy_left = copy_right - copy_w;
    let copy_rect = (copy_left, btn_top, copy_right, btn_bottom);

    let cancel_export_rect = (width - side_pad - cancel_w, btn_top, width - side_pad, btn_bottom);

    let status_left = play_rect.2 + px(14.0);
    let open_folder_rect = if show_open_folder && (copy_left - status_left) > px(190.0) {
        let f_w = px(82.0);
        let f_h = px(28.0);
        let f_top = footer_top + (footer_height - f_h) / 2;
        let f_right = copy_left - px(14.0);
        Some((f_right - f_w, f_top, f_right, f_top + f_h))
    } else {
        None
    };
    let status_right = open_folder_rect
        .map(|r| r.0 - px(10.0))
        .unwrap_or(copy_left - px(14.0));

    PlayerMetrics {
        scale: s,
        header_height,
        footer_height,
        side_pad,
        timeline_y,
        timeline_left,
        timeline_right,
        quality_group_rect,
        quality_rects,
        fps_group_rect,
        fps_values,
        fps_rects,
        play_rect,
        open_folder_rect,
        copy_rect,
        webp_rect,
        gif_rect,
        mp4_rect,
        cancel_export_rect,
        status_left,
        status_right,
    }
}

pub(super) fn move_recording_preview_seek(
    state: &mut UiState,
    point: ScreenPoint,
    held: bool,
) {
    if held
        && state
            .editor
            .as_ref()
            .is_some_and(|editor| editor.recording_preview && editor.seeking)
    {
        seek_timeline_to_x(state, point.x);
        return;
    }
    update_recording_preview_hover(state, point);
}

pub(super) fn update_recording_preview_hover(state: &mut UiState, point: ScreenPoint) {
    if !is_recording_preview(state) {
        return;
    }
    let next_hover = hit_test_player(state, point);
    let changed = state
        .editor
        .as_mut()
        .is_some_and(|editor| {
            if editor.player_hover != next_hover {
                editor.player_hover = next_hover;
                true
            } else {
                false
            }
        });
    if changed {
        super::invalidate(state);
    }
}

pub(super) fn is_recording_preview_interactive_hover(state: &UiState) -> bool {
    state
        .editor
        .as_ref()
        .is_some_and(|e| e.recording_preview && e.player_hover.is_some())
}

pub(super) fn end_recording_preview_seek(state: &mut UiState, point: ScreenPoint) -> bool {
    let seeking = state
        .editor
        .as_ref()
        .is_some_and(|editor| editor.recording_preview && editor.seeking);
    if !seeking {
        return false;
    }
    if let Some(editor) = state.editor.as_mut() {
        editor.seeking = false;
    }
    seek_timeline_to_x(state, point.x);
    true
}

fn hit_test_player(state: &UiState, point: ScreenPoint) -> Option<PlayerHit> {
    let editor = state.editor.as_ref().filter(|e| e.recording_preview)?;
    let mut client: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(state.overlay, &mut client) };
    let width = client.right - client.left;
    let height = client.bottom - client.top;
    let status = state.recording.snapshot();
    let show_open_folder = status.phase != "exporting" && status.last_path.is_some();
    let m = compute_player_metrics(width, height, status.fps.max(1), show_open_folder);

    if status.phase == "exporting" {
        if inside(point, m.cancel_export_rect) {
            return Some(PlayerHit::CancelExport);
        }
        return None;
    }

    let tl_hit_half = (12.0 * m.scale).round() as i32;
    if point.y >= m.timeline_y - tl_hit_half
        && point.y <= m.timeline_y + tl_hit_half
        && point.x >= m.timeline_left - 6
        && point.x <= m.timeline_right + 6
    {
        return Some(PlayerHit::Timeline);
    }

    if inside(point, m.play_rect) {
        return Some(PlayerHit::Play);
    }
    if inside(point, m.mp4_rect) {
        return Some(PlayerHit::ExportMp4);
    }
    if inside(point, m.gif_rect) {
        return Some(PlayerHit::ExportGif);
    }
    if inside(point, m.webp_rect) {
        return Some(PlayerHit::ExportWebp);
    }
    if inside(point, m.copy_rect) {
        return Some(PlayerHit::CopyFrame);
    }
    if let Some(folder_rect) = m.open_folder_rect {
        if inside(point, folder_rect) {
            return Some(PlayerHit::OpenFolder);
        }
    }
    for (idx, &rect) in m.quality_rects.iter().enumerate() {
        if inside(point, rect) {
            return Some(PlayerHit::Quality(idx as u8));
        }
    }
    for (idx, &rect) in m.fps_rects.iter().enumerate() {
        if inside(point, rect) {
            return Some(PlayerHit::Fps(idx));
        }
    }
    if editor.display_rect.contains(point) {
        return Some(PlayerHit::Video);
    }
    None
}

fn near_recording_timeline(state: &UiState, point: ScreenPoint) -> bool {
    matches!(hit_test_player(state, point), Some(PlayerHit::Timeline))
}

fn seek_timeline_to_x(state: &mut UiState, x: i32) {
    let mut client: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(state.overlay, &mut client) };
    let status = state.recording.snapshot();
    let m = compute_player_metrics(
        client.right - client.left,
        client.bottom - client.top,
        status.fps.max(1),
        false,
    );
    let left = m.timeline_left;
    let right = m.timeline_right.max(left + 1);
    let progress = (x.clamp(left, right) - left) as f64 / (right - left) as f64;
    let last_frame = status.timeline_frame_count().saturating_sub(1);
    seek_recording_preview(state, (progress * last_frame as f64).round() as usize);
}

pub(super) fn handle_recording_preview_click(state: &mut UiState, point: ScreenPoint) -> bool {
    if !is_recording_preview(state) {
        return false;
    }
    let Some(hit) = hit_test_player(state, point) else {
        return true;
    };
    let status = state.recording.snapshot();
    match hit {
        PlayerHit::CancelExport => {
            state.recording.cancel_export();
            super::invalidate(state);
        }
        PlayerHit::Timeline => {
            seek_timeline_to_x(state, point.x);
        }
        PlayerHit::Play | PlayerHit::Video => {
            toggle_recording_preview(state);
        }
        PlayerHit::Quality(q) => {
            if let Some(editor) = state.editor.as_mut() {
                editor.export_quality_preset = q.min(2);
            }
            super::invalidate(state);
        }
        PlayerHit::Fps(idx) => {
            let presets = recording_fps_presets(status.fps.max(1));
            if let Some(&fps) = presets.get(idx) {
                if let Some(editor) = state.editor.as_mut() {
                    editor.export_fps_preset = fps;
                }
                super::invalidate(state);
            }
        }
        PlayerHit::CopyFrame => {
            copy_recording_preview_frame(state);
        }
        PlayerHit::OpenFolder => {
            if let Some(path) = status.last_path.as_deref() {
                super::recording_ui::reveal_exported_file(path);
            }
        }
        PlayerHit::ExportMp4 => {
            super::recording_ui::export_recording_direct(
                state,
                crate::recorder::AnimationFormat::Mp4,
                crate::recorder::GifExportQuality::Balanced,
                crate::recorder::WebpExportSize::Balanced,
                status.fps.max(1),
            );
        }
        PlayerHit::ExportGif => {
            let (q_preset, fps) = state
                .editor
                .as_ref()
                .map(|e| (e.export_quality_preset, e.export_fps_preset))
                .unwrap_or((1, status.fps.max(1)));
            let gif_quality = match q_preset {
                0 => crate::recorder::GifExportQuality::Small,
                2 => crate::recorder::GifExportQuality::High,
                _ => crate::recorder::GifExportQuality::Balanced,
            };
            super::recording_ui::export_recording_direct(
                state,
                crate::recorder::AnimationFormat::Gif,
                gif_quality,
                crate::recorder::WebpExportSize::Balanced,
                fps,
            );
        }
        PlayerHit::ExportWebp => {
            let (q_preset, fps) = state
                .editor
                .as_ref()
                .map(|e| (e.export_quality_preset, e.export_fps_preset))
                .unwrap_or((1, status.fps.max(1)));
            let webp_size = match q_preset {
                0 => crate::recorder::WebpExportSize::Small,
                2 => crate::recorder::WebpExportSize::High,
                _ => crate::recorder::WebpExportSize::Balanced,
            };
            super::recording_ui::export_recording_direct(
                state,
                crate::recorder::AnimationFormat::Webp,
                crate::recorder::GifExportQuality::Balanced,
                webp_size,
                fps,
            );
        }
    }
    true
}

fn inside(point: ScreenPoint, rect: (i32, i32, i32, i32)) -> bool {
    point.x >= rect.0 && point.x < rect.2 && point.y >= rect.1 && point.y < rect.3
}

pub(super) fn toggle_recording_preview(state: &mut UiState) {
    let Some(playing) = state
        .editor
        .as_ref()
        .filter(|editor| editor.recording_preview)
        .map(|editor| editor.playing)
    else {
        return;
    };
    if playing {
        if let Some(editor) = state.editor.as_mut() {
            editor.playing = false;
            editor.playback_started_at = None;
        }
        unsafe { KillTimer(state.overlay, TIMER_RECORDING_PREVIEW) };
    } else {
        let last_frame = state
            .recording
            .snapshot()
            .timeline_frame_count()
            .saturating_sub(1);
        let frame_index = state.editor.as_ref().map_or(0, |editor| editor.frame_index);
        if frame_index >= last_frame {
            seek_recording_preview(state, 0);
        }
        if let Some(editor) = state.editor.as_mut() {
            editor.playing = true;
            editor.playback_started_at = Some(Instant::now());
            editor.playback_start_frame = editor.frame_index;
        }
        let fps = state
            .recording
            .snapshot()
            .fps
            .max(1)
            .min(RECORDING_PREVIEW_FPS);
        unsafe {
            SetTimer(
                state.overlay,
                TIMER_RECORDING_PREVIEW,
                (1000 / fps).max(1),
                None,
            );
        }
    }
    super::invalidate(state);
}

fn seek_recording_preview(state: &mut UiState, frame_index: usize) {
    let Some((width, height, pixels)) = state.recording.preview_frame(frame_index) else {
        return;
    };
    let Some((canvas_width, canvas_height, display_rect)) = state
        .editor
        .as_ref()
        .filter(|editor| editor.recording_preview)
        .and_then(|editor| {
            let screen = state.screen.as_ref()?;
            Some((screen.width, screen.height, editor.display_rect))
        })
    else {
        return;
    };
    let Some(screen) = state.screen.as_mut() else {
        return;
    };
    if !recording_canvas(
        &pixels,
        width as i32,
        height as i32,
        canvas_width,
        canvas_height,
        display_rect,
        &mut screen.pixels,
    ) {
        return;
    }
    if let Some(editor) = state.editor.as_mut() {
        editor.source_width = width as usize;
        editor.source_height = height as usize;
        editor.source_pixels = pixels;
        editor.frame_index = frame_index;
    }
    super::invalidate(state);
}

fn copy_recording_preview_frame(state: &mut UiState) {
    let Some((width, height, pixels)) = state
        .editor
        .as_ref()
        .filter(|editor| editor.recording_preview)
        .map(|editor| {
            (
                editor.source_width,
                editor.source_height,
                editor.source_pixels.clone(),
            )
        })
    else {
        return;
    };
    match crate::export::copy_to_clipboard(width, height, &pixels) {
        Ok(()) => super::clear_error(state),
        Err(error) => super::set_error(state, format!("复制当前帧失败：{error}")),
    }
}

fn recording_canvas(
    pixels: &[u8],
    source_width: i32,
    source_height: i32,
    canvas_width: i32,
    canvas_height: i32,
    display_rect: ScreenRect,
    canvas: &mut [u8],
) -> bool {
    let (Ok(source_width), Ok(source_height), Ok(target_width), Ok(target_height)) = (
        usize::try_from(source_width),
        usize::try_from(source_height),
        usize::try_from(display_rect.width()),
        usize::try_from(display_rect.height()),
    ) else {
        return false;
    };
    let (Ok(canvas_width), Ok(canvas_height), Ok(left), Ok(top)) = (
        usize::try_from(canvas_width),
        usize::try_from(canvas_height),
        usize::try_from(display_rect.left),
        usize::try_from(display_rect.top),
    ) else {
        return false;
    };
    let Some(source_len) = source_width
        .checked_mul(source_height)
        .and_then(|size| size.checked_mul(4))
    else {
        return false;
    };
    let Some(canvas_len) = canvas_width
        .checked_mul(canvas_height)
        .and_then(|size| size.checked_mul(4))
    else {
        return false;
    };
    if source_width == 0
        || source_height == 0
        || target_width == 0
        || target_height == 0
        || pixels.len() != source_len
        || canvas.len() != canvas_len
        || left + target_width > canvas_width
        || top + target_height > canvas_height
    {
        return false;
    }
    for y in 0..target_height {
        let source_y = y * source_height / target_height;
        for x in 0..target_width {
            let source_x = x * source_width / target_width;
            let source = (source_y * source_width + source_x) * 4;
            let target = ((top + y) * canvas_width + left + x) * 4;
            canvas[target] = pixels[source + 2];
            canvas[target + 1] = pixels[source + 1];
            canvas[target + 2] = pixels[source];
            canvas[target + 3] = 255;
        }
    }
    true
}

pub(super) fn map_annotation(
    annotation: &Annotation,
    from: ScreenRect,
    to: ScreenRect,
) -> Annotation {
    let sx = to.width() as f64 / from.width() as f64;
    let sy = to.height() as f64 / from.height() as f64;
    let map_point = |point: ScreenPoint| ScreenPoint {
        x: to.left + ((point.x - from.left) as f64 * sx).round() as i32,
        y: to.top + ((point.y - from.top) as f64 * sy).round() as i32,
    };
    let map_rect = move |rect: ScreenRect| {
        let top_left = map_point(ScreenPoint {
            x: rect.left,
            y: rect.top,
        });
        let bottom_right = map_point(ScreenPoint {
            x: rect.right,
            y: rect.bottom,
        });
        ScreenRect {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x,
            bottom: bottom_right.y,
        }
    };
    let size_scale = ((sx + sy) / 2.0).max(0.01);
    let map_size = |value: u32| ((value as f64 * size_scale).round() as u32).max(1);
    match annotation {
        Annotation::Rect {
            rect,
            color,
            width,
            style,
            filled,
            rounded,
        } => Annotation::Rect {
            rect: map_rect(*rect),
            color: *color,
            width: map_size(*width),
            style: *style,
            filled: *filled,
            rounded: *rounded,
        },
        Annotation::Ellipse {
            rect,
            color,
            width,
            style,
            filled,
        } => Annotation::Ellipse {
            rect: map_rect(*rect),
            color: *color,
            width: map_size(*width),
            style: *style,
            filled: *filled,
        },
        Annotation::Triangle {
            rect,
            color,
            width,
            style,
            filled,
        } => Annotation::Triangle {
            rect: map_rect(*rect),
            color: *color,
            width: map_size(*width),
            style: *style,
            filled: *filled,
        },
        Annotation::Star {
            rect,
            color,
            width,
            style,
            filled,
        } => Annotation::Star {
            rect: map_rect(*rect),
            color: *color,
            width: map_size(*width),
            style: *style,
            filled: *filled,
        },
        Annotation::Bubble {
            rect,
            color,
            width,
            style,
        } => Annotation::Bubble {
            rect: map_rect(*rect),
            color: *color,
            width: map_size(*width),
            style: *style,
        },
        Annotation::Burst {
            rect,
            color,
            width,
            style,
        } => Annotation::Burst {
            rect: map_rect(*rect),
            color: *color,
            width: map_size(*width),
            style: *style,
        },
        Annotation::Arrow {
            start,
            end,
            color,
            width,
            style,
            variant,
        } => Annotation::Arrow {
            start: map_point(*start),
            end: map_point(*end),
            color: *color,
            width: map_size(*width),
            style: *style,
            variant: *variant,
        },
        Annotation::Line(start, end, color, size, style) => Annotation::Line(
            map_point(*start),
            map_point(*end),
            *color,
            map_size(*size),
            *style,
        ),
        Annotation::Dimension {
            start,
            end,
            color,
            width,
        } => Annotation::Dimension {
            start: map_point(*start),
            end: map_point(*end),
            color: *color,
            width: map_size(*width),
        },
        Annotation::Angle {
            start,
            end,
            color,
            width,
        } => Annotation::Angle {
            start: map_point(*start),
            end: map_point(*end),
            color: *color,
            width: map_size(*width),
        },
        Annotation::Wave {
            start,
            end,
            color,
            width,
        } => Annotation::Wave {
            start: map_point(*start),
            end: map_point(*end),
            color: *color,
            width: map_size(*width),
        },
        Annotation::Highlight(start, end, color, size) => {
            Annotation::Highlight(map_point(*start), map_point(*end), *color, map_size(*size))
        }
        Annotation::Spotlight(rect) => Annotation::Spotlight(map_rect(*rect)),
        Annotation::Pen(points, color, size, style) => Annotation::Pen(
            points.iter().copied().map(map_point).collect(),
            *color,
            map_size(*size),
            *style,
        ),
        Annotation::FadePen {
            points,
            color,
            width,
            created_ms,
        } => Annotation::FadePen {
            points: points.iter().copied().map(map_point).collect(),
            color: *color,
            width: map_size(*width),
            created_ms: *created_ms,
        },
        Annotation::Mosaic(points, size) => Annotation::Mosaic(
            points.iter().copied().map(map_point).collect(),
            map_size(*size),
        ),
        Annotation::Patina { rect, intensity } => Annotation::Patina {
            rect: map_rect(*rect),
            intensity: *intensity,
        },
        Annotation::Watermark { rect, text, color } => Annotation::Watermark {
            rect: map_rect(*rect),
            text: text.clone(),
            color: *color,
        },
        Annotation::Text(point, text, size, color) => {
            Annotation::Text(map_point(*point), text.clone(), map_size(*size), *color)
        }
        Annotation::Number(point, number, color) => {
            Annotation::Number(map_point(*point), *number, *color)
        }
    }
}

pub(super) fn finish_editor(
    state: &mut UiState,
    show_save_dialog: bool,
    copy_only: bool,
    already_saved_path: Option<String>,
) {
    if show_save_dialog && already_saved_path.is_none() {
        let export = state.editor.as_ref().and_then(|editor| {
            let capture = state.capture.as_ref()?;
            let source_rect = ScreenRect {
                left: 0,
                top: 0,
                right: editor.source_width.min(i32::MAX as usize) as i32,
                bottom: editor.source_height.min(i32::MAX as usize) as i32,
            };
            let annotations = capture
                .annotations
                .iter()
                .map(|annotation| map_annotation(annotation, editor.display_rect, source_rect))
                .collect::<Vec<_>>();
            let settings = state
                .settings
                .lock()
                .map(|settings| settings.clone())
                .unwrap_or_default();
            let (out_w, out_h, pixels) = compose_with_options(
                &settings,
                editor.source_width,
                editor.source_height,
                editor.source_pixels.clone(),
                annotations,
                source_rect,
                ComposeOptions {
                    corner_radius: capture.corner_radius,
                    drop_shadow: capture.drop_shadow,
                },
            );
            Some((settings, out_w, out_h, pixels))
        });
        let Some((settings, width, height, pixels)) = export else {
            set_error(state, "无法准备图片保存内容".into());
            invalidate(state);
            return;
        };
        hide_capture_windows(state);
        match save_png_with_dialog(state.hwnd, &settings, &pixels, width, height) {
            Ok(Some(path)) => {
                finish_editor(state, false, copy_only, Some(path));
                return;
            }
            Ok(None) => {
                super::cancel(state);
                return;
            }
            Err(error) => {
                set_error(state, error);
                super::cancel(state);
                return;
            }
        }
    }
    let Some(editor) = state.editor.take() else {
        return;
    };
    let Some(capture) = state.capture.as_mut() else {
        state.editor = Some(editor);
        return;
    };
    let source_rect = ScreenRect {
        left: 0,
        top: 0,
        right: editor.source_width.min(i32::MAX as usize) as i32,
        bottom: editor.source_height.min(i32::MAX as usize) as i32,
    };
    capture.annotations = capture
        .annotations
        .iter()
        .map(|annotation| map_annotation(annotation, editor.display_rect, source_rect))
        .collect();
    capture.screen_rect = source_rect;
    capture.selection = Some(source_rect);
    capture.scroll = None;
    let bgra = rgba_preview_bgra(
        &editor.source_pixels,
        source_rect.width() as i32,
        source_rect.height() as i32,
        source_rect.width() as i32,
        source_rect.height() as i32,
    )
    .unwrap_or_default();
    state.screen = Some(ScreenCapture {
        x: 0,
        y: 0,
        width: source_rect.width() as i32,
        height: source_rect.height() as i32,
        dimmed_pixels: bgra.clone(),
        pixels: bgra,
    });
    state.render = None;
    finish_inner(state, false, copy_only, already_saved_path);
}
