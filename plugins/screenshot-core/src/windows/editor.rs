use super::output::{finish_inner, hide_capture_windows, save_png_with_dialog};
use super::paint::{create_gdi_surface, create_pixel_surface, rgba_preview_bgra, GdiSurface};
use super::scroll_ui::{
    destroy_scroll_border, destroy_scroll_masks, destroy_scroll_preview, destroy_scroll_tip,
    stop_scroll_input,
};
use super::toolbar_paint::toolbar_layout;
use super::{clear_error, invalidate, set_error, update_status, wide, UiState, TIMER_SCROLL};
use crate::capture::ScreenCapture;
use crate::export::compose as compose_export;
use crate::model::{Annotation, ScreenPoint, ScreenRect};
use crate::session::CaptureSession;
use crate::toolbar::Tool;

use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, GetSystemMetrics, KillTimer, SetTimer,
    SetForegroundWindow, ShowWindow, SW_HIDE, SW_SHOW, WS_CAPTION, WS_EX_TOOLWINDOW,
    WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
};

pub(crate) struct EditorCanvas {
    pub(super) source_width: usize,
    pub(super) source_height: usize,
    pub(super) source_pixels: Vec<u8>,
    pub(super) display_rect: ScreenRect,
    pub(super) recording_preview: bool,
    pub(super) playing: bool,
    pub(super) seeking: bool,
    pub(super) frame_index: usize,
    pub(super) canvas_surface: Option<GdiSurface>,
    pub(super) back_buffer: Option<GdiSurface>,
}

pub(super) const TIMER_RECORDING_PREVIEW: usize = 43;

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
    let mut capture = CaptureSession::new(rect, [255, 58, 72, 255], 18, false);
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

    let (window_width, window_height, window_x, window_y) = unsafe {
        let screen_width = GetSystemMetrics(0).max(800);
        let screen_height = GetSystemMetrics(1).max(600);
        let width = (screen_width - 64).clamp(720, 1480);
        let height = (screen_height - 80).clamp(560, 1000);
        (
            width,
            height,
            (screen_width - width) / 2,
            (screen_height - height) / 2,
        )
    };
    unsafe {
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let title = wide(if recording_preview { "编辑录屏" } else { "图片编辑" });
        let instance = GetModuleHandleW(null());
        let editor_hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
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
        let layout = toolbar_layout(canvas_rect);
        let toolbar_origin = if recording_preview {
            ScreenPoint {
                x: (client_width - 286).max(8),
                y: client_height - 78,
            }
        } else {
            ScreenPoint {
                x: (client_width - layout.width) / 2,
                y: 8,
            }
        };
        let image_area_top = if recording_preview {
            20
        } else {
            toolbar_origin.y + layout.height + 22
        };
        let image_area_width = (client_width - if recording_preview { 96 } else { 64 }).max(1);
        let image_area_bottom_padding = if recording_preview { 104 } else { 32 };
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
        let canvas_color = if recording_preview { 0x1d } else { 0xf5 };
        let mut canvas = vec![canvas_color; canvas_len];
        for alpha in canvas[3..].iter_mut().step_by(4) {
            *alpha = 255;
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
        let mut editor_capture = CaptureSession::new(
            canvas_rect,
            capture.annotation_color,
            capture.text_size,
            false,
        );
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
        state.capture = Some(editor_capture);
        state.editor = Some(EditorCanvas {
            source_width,
            source_height,
            source_pixels: source_pixels.clone(),
            display_rect,
            recording_preview,
            playing: recording_preview,
            seeking: false,
            frame_index: 0,
            canvas_surface,
            back_buffer,
        });
        state.last_size = Some((source_width, source_height));
        state.last_image = Some((source_width, source_height, source_pixels.clone()));
        if let Ok(mut image) = state.image.lock() {
            *image = Some((source_width, source_height, source_pixels));
        }
        state.paint_pending = false;
        if recording_preview {
            let fps = state.recording.snapshot().fps.max(1);
            windows_sys::Win32::UI::WindowsAndMessaging::SetTimer(
                editor_hwnd,
                TIMER_RECORDING_PREVIEW,
                (1000 / fps).max(1),
                None,
            );
        }
        ShowWindow(editor_hwnd, SW_SHOW);
        SetForegroundWindow(editor_hwnd);
        SetFocus(editor_hwnd);
    }
    update_status(state, true);
    clear_error(state);
}

pub(super) fn advance_recording_preview(state: &mut UiState) {
    let Some((frame_index, display_rect, playing)) = state
        .editor
        .as_ref()
        .filter(|editor| editor.recording_preview)
        .and_then(|editor| {
            state.screen.as_ref()?;
            Some((editor.frame_index + 1, editor.display_rect, editor.playing))
        })
    else {
        return;
    };
    if !playing {
        return;
    }
    let frame_count = state.recording.snapshot().timeline_frame_count();
    if frame_index >= frame_count {
        if let Some(editor) = state.editor.as_mut() {
            editor.playing = false;
        }
        unsafe { KillTimer(state.overlay, TIMER_RECORDING_PREVIEW) };
        super::invalidate(state);
        return;
    }
    let Some((width, height, pixels)) = state.recording.preview_frame(frame_index) else {
        return;
    };
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
        editor.frame_index = frame_index;
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
    }
    seek_timeline_to_x(state, point.x);
    true
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
    }
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

fn near_recording_timeline(state: &UiState, point: ScreenPoint) -> bool {
    let mut client: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(state.overlay, &mut client) };
    let footer_top = client.bottom - PLAYER_FOOTER_HEIGHT;
    let timeline_y = footer_top - PLAYER_TIMELINE_GAP;
    point.y >= timeline_y - 10
        && point.y <= timeline_y + 10
        && point.x >= PLAYER_SIDE_PADDING
        && point.x <= client.right - PLAYER_SIDE_PADDING
}

fn seek_timeline_to_x(state: &mut UiState, x: i32) {
    let mut client: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(state.overlay, &mut client) };
    let left = PLAYER_SIDE_PADDING;
    let right = (client.right - PLAYER_SIDE_PADDING).max(left + 1);
    let status = state.recording.snapshot();
    let progress = (x.clamp(left, right) - left) as f64 / (right - left) as f64;
    let last_frame = status.timeline_frame_count().saturating_sub(1);
    seek_recording_preview(state, (progress * last_frame as f64).round() as usize);
}

pub(super) fn handle_recording_preview_click(state: &mut UiState, point: ScreenPoint) -> bool {
    if !is_recording_preview(state) {
        return false;
    }
    let mut client: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(state.overlay, &mut client) };
    let width = client.right - client.left;
    let height = client.bottom - client.top;
    let footer_top = height - PLAYER_FOOTER_HEIGHT;

    let play_rect = (16, footer_top + 14, 58, height - 8);
    let export_rect = (width - 286, footer_top + 10, width - 176, height - 10);
    let copy_rect = (width - 166, footer_top + 10, width - 16, height - 10);

    let export_menu_open = state
        .capture
        .as_ref()
        .is_some_and(|capture| capture.menu == Some(crate::toolbar::ToolbarMenu::RecordingExport));
    if export_menu_open {
        if let Some((menu, index)) = recording_export_menu_hit(state, point) {
            if menu == crate::toolbar::ToolbarMenu::RecordingExport {
                if let Some(capture) = state.capture.as_mut() {
                    capture.close_menu();
                }
                let format = match index {
                    0 => crate::recorder::AnimationFormat::Mp4,
                    1 => crate::recorder::AnimationFormat::Gif,
                    _ => crate::recorder::AnimationFormat::Webp,
                };
                super::export_recording_from_editor(state, format);
                return true;
            }
        }
        if !inside(point, export_rect) {
            if let Some(capture) = state.capture.as_mut() {
                capture.close_menu();
            }
            super::invalidate(state);
        } else {
            if let Some(capture) = state.capture.as_mut() {
                capture.close_menu();
            }
            super::invalidate(state);
            return true;
        }
    }

    if near_recording_timeline(state, point) {
        seek_timeline_to_x(state, point.x);
        return true;
    }

    if point.y < footer_top {
        return true;
    }

    if inside(point, play_rect) {
        toggle_recording_preview(state);
    } else if inside(point, export_rect) {
        super::export_recording_from_editor(state, state.recording_setup_format);
    } else if inside(point, copy_rect) {
        copy_recording_preview_frame(state);
    }
    true
}

pub(super) const PLAYER_FOOTER_HEIGHT: i32 = 66;
pub(super) const PLAYER_TIMELINE_GAP: i32 = 13;
pub(super) const PLAYER_SIDE_PADDING: i32 = 24;

fn inside(point: ScreenPoint, rect: (i32, i32, i32, i32)) -> bool {
    point.x >= rect.0 && point.x < rect.2 && point.y >= rect.1 && point.y < rect.3
}

fn toggle_recording_preview(state: &mut UiState) {
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
        }
        let fps = state.recording.snapshot().fps.max(1);
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

fn recording_export_menu_hit(
    state: &UiState,
    point: ScreenPoint,
) -> Option<(crate::toolbar::ToolbarMenu, usize)> {
    let capture = state.capture.as_ref()?;
    let menu = capture.menu?;
    let origin = super::input::toolbar_origin(capture);
    let layout = crate::toolbar::menu_layout(
        origin,
        toolbar_layout(capture.screen_rect),
        menu,
        capture.tool,
    );
    let bounds = capture.screen_rect;
    let left = layout.left.clamp(
        bounds.left + 8,
        (bounds.right - layout.width - 8).max(bounds.left + 8),
    );
    let top = layout.top.clamp(
        bounds.top + 8,
        (bounds.bottom - layout.height() - 8).max(bounds.top + 8),
    );
    let layout = crate::toolbar::ToolbarMenuLayout { left, top, ..layout };
    let index = crate::toolbar::menu_hit_test(layout, (point.x, point.y))?;
    Some((menu, index))
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
    canvas.fill(0x1d);
    for alpha in canvas[3..].iter_mut().step_by(4) {
        *alpha = 255;
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
        Annotation::Rect(rect, color, size, style) => {
            Annotation::Rect(map_rect(*rect), *color, map_size(*size), *style)
        }
        Annotation::Ellipse(rect, color, size, style) => {
            Annotation::Ellipse(map_rect(*rect), *color, map_size(*size), *style)
        }
        Annotation::Arrow(start, end, color, size, style) => Annotation::Arrow(
            map_point(*start),
            map_point(*end),
            *color,
            map_size(*size),
            *style,
        ),
        Annotation::Line(start, end, color, size, style) => Annotation::Line(
            map_point(*start),
            map_point(*end),
            *color,
            map_size(*size),
            *style,
        ),
        Annotation::Highlight(start, end, color, size) => {
            Annotation::Highlight(map_point(*start), map_point(*end), *color, map_size(*size))
        }
        Annotation::Pen(points, color, size, style) => Annotation::Pen(
            points.iter().copied().map(map_point).collect(),
            *color,
            map_size(*size),
            *style,
        ),
        Annotation::Mosaic(points, size) => Annotation::Mosaic(
            points.iter().copied().map(map_point).collect(),
            map_size(*size),
        ),
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
            let pixels = compose_export(
                &settings,
                editor.source_width,
                editor.source_height,
                editor.source_pixels.clone(),
                annotations,
                source_rect,
            );
            Some((settings, editor.source_width, editor.source_height, pixels))
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
