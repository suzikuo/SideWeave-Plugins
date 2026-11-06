use super::{set_error, wide, UiState, DEFAULT_GUI_FONT};
use crate::model::{ScreenPoint, ScreenRect, RECORDING_FPS_OPTIONS};
use crate::recorder::{AnimationFormat, GifExportQuality, WebpExportSize};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW,
    Ellipse, EndPaint, FillRect, GetStockObject, InvalidateRect, Polygon, RoundRect, SelectObject,
    SetBkMode, SetTextColor, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::MK_LBUTTON;
use windows_sys::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDCBF_CANCEL_BUTTON,
    TDCBF_OK_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_EXPLORER, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetCursorPos, GetWindowRect, KillTimer,
    MessageBoxW, SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, LWA_ALPHA,
    LWA_COLORKEY, MB_ICONERROR, MB_OK, SWP_NOACTIVATE, SWP_NOZORDER, SWP_NOSIZE,
    SW_SHOWNOACTIVATE, WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_TIMER, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, IDOK, WS_POPUP,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};

const CONTROL_TIMER: usize = 0x5243;
const CONTROL_WIDTH: i32 = 448;
const CONTROL_HEIGHT: i32 = 64;
const TIMER_TEXT_RECT: RECT = RECT { left: 48, top: 2, right: 314, bottom: 62 };
const PAUSE_BUTTON: (i32, i32, i32, i32) = (48, 10, 92, 54);
const STOP_BUTTON: (i32, i32, i32, i32) = (326, 9, 436, 55);
const CANCEL_BUTTON: (i32, i32, i32, i32) = (326, 9, 436, 55);
const CLEAR_BUTTON: (i32, i32, i32, i32) = (176, 14, 232, 50);
const EDIT_BUTTON: (i32, i32, i32, i32) = (242, 14, 426, 50);

pub(super) fn create_control_toolbar(
    state: &mut UiState,
    selection: ScreenRect,
    screen: ScreenRect,
) -> Result<(), String> {
    destroy_control_toolbar(state);
    state.recording_error_dialog_message = None;
    let (x, y) = control_position(selection, screen);
    let class_name = wide("AgileTilesScreenshotCoreV2");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            x,
            y,
            CONTROL_WIDTH,
            CONTROL_HEIGHT,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            state as *mut UiState as *const c_void,
        )
    };
    if hwnd.is_null() {
        return Err(
            "\u{65e0}\u{6cd5}\u{521b}\u{5efa}\u{5f55}\u{5236}\u{5de5}\u{5177}\u{680f}".into(),
        );
    }
    state.recording_toolbar = hwnd;
    state.recording_toolbar_signature = None;
    state.recording_toolbar_static_signature = None;
    unsafe {
        SetLayeredWindowAttributes(hwnd, 0x00030201, 248, LWA_ALPHA | LWA_COLORKEY);
        SetTimer(hwnd, CONTROL_TIMER, 250, None);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(())
}

pub(super) fn destroy_control_toolbar(state: &mut UiState) {
    if state.recording_toolbar.is_null() {
        return;
    }
    unsafe {
        KillTimer(state.recording_toolbar, CONTROL_TIMER);
        DestroyWindow(state.recording_toolbar);
    }
    state.recording_toolbar = std::ptr::null_mut();
    state.recording_toolbar_drag = None;
    state.recording_toolbar_signature = None;
    state.recording_toolbar_static_signature = None;
}

pub(super) unsafe fn control_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> isize {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_control_toolbar(hwnd, state);
            0
        }
        WM_TIMER => {
            let status = state.recording.snapshot();
            show_error_details_if_needed(state, hwnd, &status);
            if status.phase == "preview"
                && state.recording_auto_open_pending
                && state.editor.is_none()
                && !state.recording_export_dialog_open
            {
                super::queue_open_recording_editor(state);
            } else if state.recording_toolbar_static_signature.as_deref()
                != Some(recording_toolbar_static_signature(&status).as_str()) {
                InvalidateRect(hwnd, std::ptr::null(), 0);
            } else if state.recording_toolbar_signature.as_deref()
                != Some(recording_toolbar_signature(&status).as_str()) {
                InvalidateRect(hwnd, &TIMER_TEXT_RECT, 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let point = (
                (lparam & 0xffff) as i16 as i32,
                ((lparam >> 16) & 0xffff) as i16 as i32,
            );
            let x = point.0;
            if (0..40).contains(&x) {
                let mut cursor: POINT = zeroed();
                let mut rect: RECT = zeroed();
                if GetCursorPos(&mut cursor) != 0 && GetWindowRect(hwnd, &mut rect) != 0 {
                    state.recording_toolbar_drag = Some((
                        ScreenPoint { x: cursor.x, y: cursor.y },
                        ScreenPoint { x: rect.left, y: rect.top },
                    ));
                    SetCapture(hwnd);
                }
            } else {
                // This non-activating layered popup may not receive the matching
                // mouse-up reliably, so dispatch controls as soon as they're pressed.
                handle_control_click(state, point);
            }
            0
        }
        WM_MOUSEMOVE if wparam & MK_LBUTTON as usize != 0 => {
            if let Some((start, origin)) = state.recording_toolbar_drag {
                let mut cursor: POINT = zeroed();
                if GetCursorPos(&mut cursor) != 0 {
                    let mut x = origin.x + cursor.x - start.x;
                    let mut y = origin.y + cursor.y - start.y;
                    if let Some(screen) = state.screen.as_ref().map(super::ScreenCapture::bounds) {
                        x = x.clamp(screen.left + 4, (screen.right - CONTROL_WIDTH - 4).max(screen.left + 4));
                        y = y.clamp(screen.top + 4, (screen.bottom - CONTROL_HEIGHT - 4).max(screen.top + 4));
                    }
                    SetWindowPos(hwnd, std::ptr::null_mut(), x, y, 0, 0, SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE);
                }
            }
            0
        }
        WM_LBUTTONUP => {
            if state.recording_toolbar_drag.take().is_some() {
                ReleaseCapture();
            }
            0
        }
        WM_CAPTURECHANGED => {
            state.recording_toolbar_drag = None;
            0
        }
        WM_CLOSE => {
            state.recording.cancel();
            destroy_control_toolbar(state);
            super::cancel(state);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, CONTROL_TIMER);
            0
        }
        _ => super::DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn control_position(selection: ScreenRect, screen: ScreenRect) -> (i32, i32) {
    let margin = 8;
    let selection_width = selection.right - selection.left;
    let desired_x = selection.left + (selection_width - CONTROL_WIDTH) / 2;
    let x = desired_x.clamp(
        screen.left + margin,
        (screen.right - CONTROL_WIDTH - margin).max(screen.left + margin),
    );
    let above = selection.top - CONTROL_HEIGHT - margin;
    let below = selection.bottom + margin;
    let y = if above >= screen.top + margin {
        above
    } else if below + CONTROL_HEIGHT <= screen.bottom - margin {
        below
    } else {
        (screen.top + margin).min(screen.bottom - CONTROL_HEIGHT - margin)
    };
    (x, y)
}

unsafe fn paint_control_toolbar(hwnd: HWND, state: &mut UiState) {
    let mut paint: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(hwnd, &mut paint);
    let background = CreateSolidBrush(0x00030201);
    let rect = RECT {
        left: 0,
        top: 0,
        right: CONTROL_WIDTH,
        bottom: CONTROL_HEIGHT,
    };
    FillRect(dc, &rect, background);
    DeleteObject(background as _);

    let shadow = CreateSolidBrush(0x00cbd3dd);
    let shadow_old_brush = SelectObject(dc, shadow as _);
    let shadow_old_pen = SelectObject(
        dc,
        GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_PEN as i32),
    );
    RoundRect(dc, 2, 3, CONTROL_WIDTH - 2, CONTROL_HEIGHT - 1, 22, 22);
    SelectObject(dc, shadow_old_pen);
    SelectObject(dc, shadow_old_brush);
    DeleteObject(shadow as _);

    let border = CreatePen(0, 1, 0x00d8dee7);
    let white = CreateSolidBrush(0x00ffffff);
    let old_border = SelectObject(dc, border as _);
    let old_white = SelectObject(dc, white as _);
    RoundRect(dc, 1, 1, CONTROL_WIDTH - 1, CONTROL_HEIGHT - 1, 22, 22);
    SelectObject(dc, old_border);
    SelectObject(dc, old_white);
    DeleteObject(border as _);
    DeleteObject(white as _);

    let grip = CreateSolidBrush(0x0097a0ac);
    let old_grip = SelectObject(dc, grip as _);
    for row in 0..3 {
        for column in 0..2 {
            let left = 17 + column * 6;
            let top = 20 + row * 7;
            Ellipse(dc, left, top, left + 3, top + 3);
        }
    }
    SelectObject(dc, old_grip);
    DeleteObject(grip as _);

    let status = state.recording.snapshot();
    state.recording_toolbar_signature = Some(recording_toolbar_signature(&status));
    state.recording_toolbar_static_signature = Some(recording_toolbar_static_signature(&status));
    let label = control_status_text(&status);
    match status.phase.as_str() {
        "recording" => {
            draw_pause_toggle(dc, PAUSE_BUTTON, status.paused);
            draw_status_dot(dc, 111, 32, 0x00454af5);
            draw_control_text(dc, 126, 6, 314, 31, "正在录制", 0x0076818f);
            draw_control_text(dc, 126, 28, 314, 57, &recording_time_text(&status), 0x00343a45);
            draw_control_button(dc, STOP_BUTTON, "结束录制", 0x00454af5, true);
        }
        "processing" => {
            draw_control_text(dc, 48, 10, 436, 54, &label, 0x004a5666);
        }
        "exporting" | "cancelling" => {
            draw_control_text(dc, 48, 3, 314, 40, &label, 0x004a5666);
            if status.phase == "exporting" {
                draw_control_progress_bar(dc, 48, 47, 314, 55, status.progress_percent);
            }
            draw_control_button(dc, CANCEL_BUTTON, "取消", 0x00f3f5f8, false);
        }
        "preview" => {
            draw_control_text(dc, 48, 18, 164, 54, &label, 0x004a5666);
            draw_control_button(dc, CLEAR_BUTTON, "清除", 0x00f3f5f8, false);
            draw_control_button(dc, EDIT_BUTTON, "打开预览", 0x00f76f16, true);
        }
        _ => {
            draw_control_text(dc, 48, 10, 314, 54, &label, 0x004a5666);
            draw_control_button(dc, CANCEL_BUTTON, "返回", 0x00f3f5f8, false);
        }
    }
    EndPaint(hwnd, &paint);
}

unsafe fn draw_pause_toggle(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    rect: (i32, i32, i32, i32),
    paused: bool,
) {
    let brush = CreateSolidBrush(0x00f3f5f8);
    let pen = CreatePen(0, 1, 0x00e2e7ef);
    let old_brush = SelectObject(dc, brush as _);
    let old_pen = SelectObject(dc, pen as _);
    RoundRect(dc, rect.0, rect.1, rect.2, rect.3, 10, 10);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush as _);
    DeleteObject(pen as _);

    let icon = CreateSolidBrush(0x00343a45);
    let old_icon = SelectObject(dc, icon as _);
    if paused {
        let points = [
            POINT { x: rect.0 + 16, y: rect.1 + 12 },
            POINT { x: rect.0 + 16, y: rect.3 - 12 },
            POINT { x: rect.2 - 13, y: (rect.1 + rect.3) / 2 },
        ];
        Polygon(dc, points.as_ptr(), points.len() as i32);
    } else {
        let left_bar = RECT { left: rect.0 + 15, top: rect.1 + 12, right: rect.0 + 21, bottom: rect.3 - 12 };
        let right_bar = RECT { left: rect.0 + 25, top: rect.1 + 12, right: rect.0 + 31, bottom: rect.3 - 12 };
        FillRect(dc, &left_bar, icon);
        FillRect(dc, &right_bar, icon);
    }
    SelectObject(dc, old_icon);
    DeleteObject(icon as _);
}

unsafe fn draw_control_text(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    value: &str,
    color: u32,
) {
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let old_font = SelectObject(dc, font);
    SetBkMode(dc, 1);
    SetTextColor(dc, color);
    let text = wide(value);
    let mut rect = RECT {
        left,
        top,
        right,
        bottom,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut rect, 1 | 4 | 0x20);
    SelectObject(dc, old_font);
}

unsafe fn draw_control_button(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    rect: (i32, i32, i32, i32),
    label: &str,
    color: u32,
    primary: bool,
) {
    let brush = CreateSolidBrush(if primary { color } else { 0x00f3f5f8 });
    let pen = CreatePen(0, 1, if primary { color } else { 0x00e2e7ef });
    let old_brush = SelectObject(dc, brush as _);
    let old_pen = SelectObject(dc, pen as _);
    RoundRect(dc, rect.0, rect.1, rect.2, rect.3, 12, 12);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush as _);
    DeleteObject(pen as _);
    let text_color = if primary { 0x00ffffff } else { 0x00343a45 };
    draw_control_text(dc, rect.0 + 3, rect.1 + 2, rect.2 - 3, rect.3 - 2, label, text_color);
}

unsafe fn draw_status_dot(dc: windows_sys::Win32::Graphics::Gdi::HDC, x: i32, y: i32, color: u32) {
    let brush = CreateSolidBrush(color);
    let old = SelectObject(dc, brush as _);
    Ellipse(dc, x - 5, y - 5, x + 5, y + 5);
    SelectObject(dc, old);
    DeleteObject(brush as _);
}

unsafe fn draw_control_progress_bar(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    percent: u8,
) {
    let track_brush = CreateSolidBrush(0x00dce2e9);
    let old_brush = SelectObject(dc, track_brush as _);
    let old_pen = SelectObject(
        dc,
        GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_PEN as i32),
    );
    RoundRect(dc, left, top, right, bottom, bottom - top, bottom - top);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(track_brush as _);

    let filled_right = left + ((right - left) * i32::from(percent.min(100)) / 100);
    if filled_right <= left {
        return;
    }
    let progress_brush = CreateSolidBrush(0x00f76f16);
    let old_brush = SelectObject(dc, progress_brush as _);
    let old_pen = SelectObject(
        dc,
        GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_PEN as i32),
    );
    RoundRect(dc, left, top, filled_right, bottom, bottom - top, bottom - top);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(progress_brush as _);
}

fn control_status_text(status: &crate::recorder::RecordingStatus) -> String {
    match status.phase.as_str() {
        "recording" => "屏幕录制中".into(),
        "processing" => "正在结束录制，请稍候…".into(),
        "exporting" => format!(
            "{} {}%",
            status.message.as_deref().unwrap_or("正在导出…"),
            status.progress_percent
        ),
        "cancelling" => "正在取消…".into(),
        "preview" => match status.message.as_deref() {
            Some(message) if message.starts_with("导出失败：") => "导出失败，打开预览".into(),
            Some(message) if message.starts_with("导出完成：") => "导出完成".into(),
            _ => format!("预览就绪 · {} 帧", status.frame_count),
        },
        "error" => status.message.clone().unwrap_or_else(|| "录制失败".into()),
        "cancelled" => "录制已取消".into(),
        _ => "录制工具".into(),
    }
}

fn recording_toolbar_signature(status: &crate::recorder::RecordingStatus) -> String {
    let recording = status.phase == "recording";
    format!(
        "{}|{}|{}|{}|{}|{}",
        status.phase,
        status.paused,
        if recording { status.elapsed_ms / 1000 } else { 0 },
        if recording { 0 } else { status.frame_count },
        if status.phase == "exporting" {
            status.progress_percent
        } else {
            0
        },
        status.message.as_deref().unwrap_or_default()
    )
}

fn recording_toolbar_static_signature(status: &crate::recorder::RecordingStatus) -> String {
    let frame_count = if status.phase == "recording" {
        0
    } else {
        status.frame_count
    };
    format!(
        "{}|{}|{}|{}",
        status.phase,
        status.paused,
        frame_count,
        status.message.as_deref().unwrap_or_default()
    )
}

fn recording_time_text(status: &crate::recorder::RecordingStatus) -> String {
    if status.max_duration_ms == 0 {
        format!("{} / ∞", format_elapsed(status.elapsed_ms))
    } else {
        format!(
            "{} / {}",
            format_elapsed(status.elapsed_ms),
            format_elapsed(status.max_duration_ms)
        )
    }
}

fn format_elapsed(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn handle_control_click(state: &mut UiState, point: (i32, i32)) {
    let status = state.recording.snapshot();
    match status.phase.as_str() {
        "recording" if inside(point, PAUSE_BUTTON) => {
            if let Err(error) = state.recording.toggle_pause() {
                set_error(state, error);
            }
        }
        "recording" if inside(point, STOP_BUTTON) => super::stop_recording(state),
        "exporting" if inside(point, CANCEL_BUTTON) => {
            state.recording.cancel();
            destroy_control_toolbar(state);
            super::restore_recording_selection(state);
        }
        "preview" if inside(point, EDIT_BUTTON) => {
            super::queue_open_recording_editor(state);
        }
        "preview" if inside(point, CLEAR_BUTTON) => {
            state.recording.cancel();
            destroy_control_toolbar(state);
            super::cancel(state);
        }
        "error" | "cancelled" if inside(point, CANCEL_BUTTON) => {
            destroy_control_toolbar(state);
            super::restore_recording_selection(state);
        }
        _ => {}
    }
}

fn inside(point: (i32, i32), rect: (i32, i32, i32, i32)) -> bool {
    point.0 >= rect.0 && point.0 < rect.2 && point.1 >= rect.1 && point.1 < rect.3
}

pub(super) fn export_recording(state: &mut UiState, format: AnimationFormat) {
    // Export completion returns to preview; it must not trigger the one-time
    // editor transition intended for the end of a new recording.
    state.recording_auto_open_pending = false;
    state.recording_error_dialog_message = None;
    let initial_dir = state
        .settings
        .lock()
        .map(|settings| settings.save_directory.clone())
        .unwrap_or_default();
    let owner = if state.editor.is_some() {
        state.overlay
    } else if !state.recording_toolbar.is_null() {
        state.recording_toolbar
    } else {
        state.hwnd
    };
    state.recording_export_dialog_open = true;
    let source_fps = state.recording.snapshot().fps.max(1);
    let path = match save_animation_with_dialog(owner, format, &initial_dir, source_fps) {
        Ok(path) => path,
        Err(error) => {
            state.recording_export_dialog_open = false;
            set_error(state, error);
            return;
        }
    };
    state.recording_export_dialog_open = false;
    if let Some(toolbar) = (!state.recording_toolbar.is_null()).then_some(state.recording_toolbar) {
        unsafe { SetTimer(toolbar, CONTROL_TIMER, 250, None) };
    }
    let Some((path, gif_export_quality, webp_export_size, output_fps)) = path else {
        return;
    };
    let (_, recording_bitrate_mbps) =
        super::configured_recording_options(&state.settings);
    let (gif_colors, webp_quality) =
        super::configured_animation_export_options(&state.settings);
    if let Err(error) = state
        .recording
        .export(
            format,
            gif_export_quality,
            webp_export_size,
            path,
            output_fps,
            recording_bitrate_mbps,
            gif_colors,
            webp_quality,
        )
    {
        set_error(state, error);
    } else if state.editor.is_some() {
        if let Some(editor) = state.editor.as_mut() {
            editor.playing = false;
        }
        unsafe {
            KillTimer(state.overlay, super::editor::TIMER_RECORDING_PREVIEW);
            SetTimer(
                state.overlay,
                super::editor::TIMER_RECORDING_EXPORT_STATUS,
                250,
                None,
            );
            InvalidateRect(state.overlay, std::ptr::null(), 0);
        }
    }
}

pub(super) fn show_error_details_if_needed(
    state: &mut UiState,
    owner: HWND,
    status: &crate::recorder::RecordingStatus,
) {
    let is_recording_error = status.phase == "error";
    let is_export_error = status.phase == "preview"
        && status
            .message
            .as_deref()
            .is_some_and(|message| message.starts_with("导出失败："));
    if !is_recording_error && !is_export_error {
        return;
    }
    let Some(message) = status.message.as_deref() else {
        return;
    };
    if state.recording_error_dialog_message.as_deref() == Some(message) {
        return;
    }
    state.recording_error_dialog_message = Some(message.to_string());

    let title = wide(if is_export_error {
        "AgileTiles 导出失败"
    } else {
        "AgileTiles 录制失败"
    });
    let instruction = wide(if is_export_error {
        "导出没有完成"
    } else {
        "录制没有完成"
    });
    let content = wide(message);
    let mut config: TASKDIALOGCONFIG = unsafe { zeroed() };
    config.cbSize = size_of::<TASKDIALOGCONFIG>() as u32;
    config.hwndParent = owner;
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION;
    config.dwCommonButtons = TDCBF_OK_BUTTON;
    config.pszWindowTitle = title.as_ptr();
    config.pszMainInstruction = instruction.as_ptr();
    config.pszContent = content.as_ptr();
    config.nDefaultButton = IDOK;

    let mut clicked_button = 0;
    let result = unsafe {
        TaskDialogIndirect(
            &config,
            &mut clicked_button,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result < 0 {
        unsafe {
            MessageBoxW(
                owner,
                content.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            )
        };
    }
}

fn save_animation_with_dialog(
    owner: windows_sys::Win32::Foundation::HWND,
    format: AnimationFormat,
    initial_dir: &str,
    source_fps: u32,
) -> Result<Option<(PathBuf, GifExportQuality, WebpExportSize, u32)>, String> {
    let (title, default_name, filter) = match format {
        AnimationFormat::Webp => (
            "Export animated WebP",
            "AgileTiles_Recording.webp",
            wide("Animated WebP (*.webp)\0*.webp\0"),
        ),
        AnimationFormat::Gif => (
            "Export GIF",
            "AgileTiles_Recording.gif",
            wide("GIF 动图 (*.gif)\0*.gif\0"),
        ),
        AnimationFormat::Mp4 => (
            "Export MP4",
            "AgileTiles_Recording.mp4",
            wide("MP4 video (*.mp4)\0*.mp4\0"),
        ),
    };
    let title = wide(title);
    let extension = wide(format.extension());
    let initial_dir = (!initial_dir.trim().is_empty()).then(|| wide(initial_dir));
    let initial_dir_ptr = initial_dir.as_ref().map(|value| value.as_ptr());
    let mut file = vec![0u16; 512];
    let default_name = wide(default_name);
    let copy_len = default_name.len().min(file.len() - 1);
    file[..copy_len].copy_from_slice(&default_name[..copy_len]);
    let mut dialog: OPENFILENAMEW = unsafe { zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.lpstrInitialDir = initial_dir_ptr.unwrap_or(std::ptr::null());
    dialog.lpstrTitle = title.as_ptr();
    dialog.lpstrDefExt = extension.as_ptr();
    dialog.nFilterIndex = 1;
    dialog.Flags = OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT;
    if unsafe { GetSaveFileNameW(&mut dialog) } == 0 {
        return Ok(None);
    }
    let gif_export_quality = if format == AnimationFormat::Gif {
        let Some(quality) = choose_gif_export_quality(owner)? else {
            return Ok(None);
        };
        quality
    } else {
        GifExportQuality::Recommended
    };
    let webp_export_size = if format == AnimationFormat::Webp {
        let Some(size) = choose_webp_export_size(owner)? else {
            return Ok(None);
        };
        size
    } else {
        WebpExportSize::Recommended
    };
    let output_fps = if matches!(format, AnimationFormat::Gif | AnimationFormat::Webp) {
        let Some(fps) = choose_animation_export_fps(owner, format, source_fps)? else {
            return Ok(None);
        };
        fps
    } else {
        source_fps
    };
    let end = file
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(file.len());
    let selected = String::from_utf16(&file[..end])
        .map_err(|_| "无法读取导出文件名".to_string())?;
    if selected.trim().is_empty() {
        return Ok(None);
    }
    let mut path = PathBuf::from(selected);
    path.set_extension(format.extension());
    Ok(Some((
        path,
        gif_export_quality,
        webp_export_size,
        output_fps,
    )))
}

fn choose_gif_export_quality(
    owner: windows_sys::Win32::Foundation::HWND,
) -> Result<Option<GifExportQuality>, String> {
    let title = wide("GIF 导出参数");
    let instruction = wide("选择导出画面的清晰度");
    let content = wide("选择 GIF 画面尺寸上限。智能推荐会根据录制尺寸和帧率自动选择；尺寸越大，导出文件通常也越大。");
    let options = [
        wide("智能推荐（默认）"),
        wide("清晰（最高 1080p）"),
        wide("均衡（最高 720p）"),
        wide("小文件（最高 540p）"),
    ];
    let radio_buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: 1,
            pszButtonText: options[0].as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: 2,
            pszButtonText: options[1].as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: 3,
            pszButtonText: options[2].as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: 4,
            pszButtonText: options[3].as_ptr(),
        },
    ];
    let mut config: TASKDIALOGCONFIG = unsafe { zeroed() };
    config.cbSize = size_of::<TASKDIALOGCONFIG>() as u32;
    config.hwndParent = owner;
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION;
    config.dwCommonButtons = TDCBF_OK_BUTTON | TDCBF_CANCEL_BUTTON;
    config.pszWindowTitle = title.as_ptr();
    config.pszMainInstruction = instruction.as_ptr();
    config.pszContent = content.as_ptr();
    config.cRadioButtons = radio_buttons.len() as u32;
    config.pRadioButtons = radio_buttons.as_ptr();
    config.nDefaultRadioButton = 1;
    config.nDefaultButton = IDOK;

    let mut clicked_button = 0;
    let mut selected_option = 0;
    let result = unsafe {
        TaskDialogIndirect(
            &config,
            &mut clicked_button,
            &mut selected_option,
            std::ptr::null_mut(),
        )
    };
    if result < 0 {
        return Err(format!("无法打开 GIF 导出参数窗口（错误码 {result:#x}）"));
    }
    if clicked_button != IDOK {
        return Ok(None);
    }
    let quality = match selected_option {
        2 => GifExportQuality::High,
        3 => GifExportQuality::Balanced,
        4 => GifExportQuality::Small,
        _ => GifExportQuality::Recommended,
    };
    Ok(Some(quality))
}

fn choose_webp_export_size(
    owner: windows_sys::Win32::Foundation::HWND,
) -> Result<Option<WebpExportSize>, String> {
    let title = wide("WebP 导出参数");
    let instruction = wide("选择导出规格");
    let content = wide("智能推荐会根据录制尺寸和帧率自动选择；尺寸越小，导出越快，文件通常也越小。WebP 画质仍使用详情设置中的质量选项。");
    let options = [
        wide("智能推荐（默认）"),
        wide("清晰（最高 1080p）"),
        wide("均衡（最高 720p）"),
        wide("快速导出（最高 540p）"),
    ];
    let radio_buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: 1,
            pszButtonText: options[0].as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: 2,
            pszButtonText: options[1].as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: 3,
            pszButtonText: options[2].as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: 4,
            pszButtonText: options[3].as_ptr(),
        },
    ];
    let mut config: TASKDIALOGCONFIG = unsafe { zeroed() };
    config.cbSize = size_of::<TASKDIALOGCONFIG>() as u32;
    config.hwndParent = owner;
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION;
    config.dwCommonButtons = TDCBF_OK_BUTTON | TDCBF_CANCEL_BUTTON;
    config.pszWindowTitle = title.as_ptr();
    config.pszMainInstruction = instruction.as_ptr();
    config.pszContent = content.as_ptr();
    config.cRadioButtons = radio_buttons.len() as u32;
    config.pRadioButtons = radio_buttons.as_ptr();
    config.nDefaultRadioButton = 1;
    config.nDefaultButton = IDOK;

    let mut clicked_button = 0;
    let mut selected_option = 0;
    let result = unsafe {
        TaskDialogIndirect(
            &config,
            &mut clicked_button,
            &mut selected_option,
            std::ptr::null_mut(),
        )
    };
    if result < 0 {
        return Err(format!("无法打开 WebP 导出参数窗口（错误码 {result:#x}）"));
    }
    if clicked_button != IDOK {
        return Ok(None);
    }
    let size = match selected_option {
        2 => WebpExportSize::High,
        3 => WebpExportSize::Balanced,
        4 => WebpExportSize::Small,
        _ => WebpExportSize::Recommended,
    };
    Ok(Some(size))
}

fn choose_animation_export_fps(
    owner: windows_sys::Win32::Foundation::HWND,
    format: AnimationFormat,
    source_fps: u32,
) -> Result<Option<u32>, String> {
    let source_fps = source_fps.max(1);
    let format_name = match format {
        AnimationFormat::Gif => "GIF",
        AnimationFormat::Webp => "WebP",
        AnimationFormat::Mp4 => "MP4",
    };
    let title = wide(&format!("{format_name} 导出帧率"));
    let instruction = wide("选择导出帧率");
    let content =
        wide("较低帧率会减少编码帧数，通常能加快导出；动画会更不连贯，但总时长保持不变。");
    let mut fps_values = vec![source_fps];
    fps_values.extend(
        RECORDING_FPS_OPTIONS
            .iter()
            .rev()
            .copied()
            .filter(|fps| *fps < source_fps),
    );
    let options = fps_values
        .iter()
        .enumerate()
        .map(|(index, fps)| {
            if index == 0 {
                wide(&format!("保持录制帧率（{fps} FPS，默认）"))
            } else {
                wide(&format!("{fps} FPS"))
            }
        })
        .collect::<Vec<_>>();
    let radio_buttons = options
        .iter()
        .enumerate()
        .map(|(index, option)| TASKDIALOG_BUTTON {
            nButtonID: index as i32 + 1,
            pszButtonText: option.as_ptr(),
        })
        .collect::<Vec<_>>();
    let mut config: TASKDIALOGCONFIG = unsafe { zeroed() };
    config.cbSize = size_of::<TASKDIALOGCONFIG>() as u32;
    config.hwndParent = owner;
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION;
    config.dwCommonButtons = TDCBF_OK_BUTTON | TDCBF_CANCEL_BUTTON;
    config.pszWindowTitle = title.as_ptr();
    config.pszMainInstruction = instruction.as_ptr();
    config.pszContent = content.as_ptr();
    config.cRadioButtons = radio_buttons.len() as u32;
    config.pRadioButtons = radio_buttons.as_ptr();
    config.nDefaultRadioButton = 1;
    config.nDefaultButton = IDOK;

    let mut clicked_button = 0;
    let mut selected_option = 0;
    let result = unsafe {
        TaskDialogIndirect(
            &config,
            &mut clicked_button,
            &mut selected_option,
            std::ptr::null_mut(),
        )
    };
    if result < 0 {
        return Err(format!(
            "无法打开 {format_name} 导出帧率窗口（错误码 {result:#x}）"
        ));
    }
    if clicked_button != IDOK {
        return Ok(None);
    }
    let selected_index = selected_option.saturating_sub(1) as usize;
    Ok(Some(
        fps_values
            .get(selected_index)
            .copied()
            .unwrap_or(source_fps),
    ))
}
