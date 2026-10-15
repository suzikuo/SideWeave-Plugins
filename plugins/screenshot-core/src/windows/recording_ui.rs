use super::{set_error, wide, UiState, DEFAULT_GUI_FONT};
use crate::model::{ScreenPoint, ScreenRect};
use crate::recorder::AnimationFormat;
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
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_EXPLORER, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetCursorPos, GetWindowRect, KillTimer,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, LWA_ALPHA, LWA_COLORKEY,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_NOSIZE, SW_SHOWNOACTIVATE, WM_CAPTURECHANGED, WM_CLOSE,
    WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_TIMER, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};

const CONTROL_TIMER: usize = 0x5243;
const CONTROL_WIDTH: i32 = 448;
const CONTROL_HEIGHT: i32 = 64;
const TIMER_TEXT_RECT: RECT = RECT { left: 126, top: 10, right: 314, bottom: 54 };
const PAUSE_BUTTON: (i32, i32, i32, i32) = (48, 10, 92, 54);
const STOP_BUTTON: (i32, i32, i32, i32) = (326, 9, 436, 55);
const CANCEL_BUTTON: (i32, i32, i32, i32) = (326, 9, 436, 55);
const CLEAR_BUTTON: (i32, i32, i32, i32) = (170, 14, 212, 50);
const EDIT_BUTTON: (i32, i32, i32, i32) = (218, 14, 266, 50);
const MP4_BUTTON: (i32, i32, i32, i32) = (272, 14, 318, 50);
const GIF_BUTTON: (i32, i32, i32, i32) = (324, 14, 368, 50);
const WEBP_BUTTON: (i32, i32, i32, i32) = (374, 14, 426, 50);

pub(super) fn create_control_toolbar(
    state: &mut UiState,
    selection: ScreenRect,
    screen: ScreenRect,
) -> Result<(), String> {
    destroy_control_toolbar(state);
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
            if status.phase == "preview" && state.editor.is_none() {
                super::open_recording_editor(state);
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

    let border = CreatePen(0, 1, 0x00e2e7ef);
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
            draw_control_text(dc, 126, 10, 314, 54, &recording_time_text(&status), 0x004a5666);
            draw_control_button(dc, STOP_BUTTON, "结束录制", 0x00454af5, true);
        }
        "processing" | "exporting" | "cancelling" => {
            draw_control_text(dc, 48, 10, 314, 54, &label, 0x004a5666);
            draw_control_button(dc, CANCEL_BUTTON, "取消", 0x00f3f5f8, false);
        }
        "preview" => {
            draw_control_text(dc, 48, 18, 164, 54, &label, 0x004a5666);
            draw_control_button(dc, CLEAR_BUTTON, "清除", 0x00f3f5f8, false);
            draw_control_button(dc, EDIT_BUTTON, "编辑", 0x00f3f5f8, false);
            draw_control_button(dc, MP4_BUTTON, "MP4", 0x00f76f16, true);
            draw_control_button(dc, GIF_BUTTON, "GIF", 0x00f3f5f8, false);
            draw_control_button(dc, WEBP_BUTTON, "WebP", 0x00f3f5f8, false);
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

fn control_status_text(status: &crate::recorder::RecordingStatus) -> String {
    match status.phase.as_str() {
        "recording" => "屏幕录制中".into(),
        "processing" => "正在生成预览…".into(),
        "exporting" => "正在导出…".into(),
        "cancelling" => "正在取消…".into(),
        "preview" => format!("预览就绪 · {} 帧", status.frame_count),
        "error" => status.message.clone().unwrap_or_else(|| "录制失败".into()),
        "cancelled" => "录制已取消".into(),
        _ => "录制工具".into(),
    }
}

fn recording_toolbar_signature(status: &crate::recorder::RecordingStatus) -> String {
    let recording = status.phase == "recording";
    format!(
        "{}|{}|{}|{}|{}",
        status.phase,
        status.paused,
        if recording { status.elapsed_ms / 1000 } else { 0 },
        if recording { 0 } else { status.frame_count },
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
        "processing" | "exporting" if inside(point, CANCEL_BUTTON) => {
            state.recording.cancel();
            destroy_control_toolbar(state);
            super::restore_recording_selection(state);
        }
        "preview" if inside(point, EDIT_BUTTON) => {
            super::open_recording_editor(state);
        }
        "preview" if inside(point, MP4_BUTTON) => {
            export_recording(state, AnimationFormat::Mp4);
        }
        "preview" if inside(point, GIF_BUTTON) => {
            export_recording(state, AnimationFormat::Gif);
        }
        "preview" if inside(point, WEBP_BUTTON) => {
            export_recording(state, AnimationFormat::Webp);
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
    let initial_dir = state
        .settings
        .lock()
        .map(|settings| settings.save_directory.clone())
        .unwrap_or_default();
    let owner = if state.recording_toolbar.is_null() {
        state.hwnd
    } else {
        state.recording_toolbar
    };
    let Some(path) = save_animation_with_dialog(owner, format, &initial_dir) else {
        return;
    };
    let output_fps = super::configured_recording_fps(&state.settings, format);
    let (_, mp4_bitrate_mbps) = super::configured_mp4_recording_options(&state.settings);
    if let Err(error) = state
        .recording
        .export(format, path, output_fps, mp4_bitrate_mbps)
    {
        set_error(state, error);
    }
}

fn save_animation_with_dialog(
    owner: windows_sys::Win32::Foundation::HWND,
    format: AnimationFormat,
    initial_dir: &str,
) -> Option<PathBuf> {
    let (description, title, default_name) = match format {
        AnimationFormat::Webp => (
            "Animated WebP (*.webp)",
            "Export animated WebP",
            "AgileTiles_Recording.webp",
        ),
        AnimationFormat::Gif => (
            "GIF animation (*.gif)",
            "Export GIF",
            "AgileTiles_Recording.gif",
        ),
        AnimationFormat::Mp4 => (
            "MP4 video (*.mp4)",
            "Export MP4",
            "AgileTiles_Recording.mp4",
        ),
    };
    let filter = wide(&format!("{description}\0*.{}\0\0", format.extension()));
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
    dialog.Flags = OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT;
    if unsafe { GetSaveFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let end = file
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(file.len());
    let selected = String::from_utf16(&file[..end]).ok()?;
    if selected.trim().is_empty() {
        return None;
    }
    let mut path = PathBuf::from(selected);
    path.set_extension(format.extension());
    Some(path)
}
