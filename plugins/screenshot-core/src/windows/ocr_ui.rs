use super::scroll_ui::show_scroll_tip;
use super::{set_error, wide, UiState, WAKE};
use crate::model::Settings;
use crate::tencent_ocr::{RecognizeAgentRequest, TencentCloudCredentials, TencentOcrService};
use arboard::Clipboard;
use std::ptr::{null, null_mut};
use std::thread;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject,
    DrawTextW, EndPaint, InvalidateRect, RoundRect, SelectObject, SetBkMode, SetTextColor,
    SetWindowRgn, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    FF_DONTCARE, FW_NORMAL, HGDIOBJ, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowRect, PostMessageW,
    SendMessageW, SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow, ES_AUTOVSCROLL,
    ES_MULTILINE, ES_READONLY, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_CLOSE, WM_DESTROY,
    WM_ERASEBKGND, WM_LBUTTONUP, WM_NCHITTEST, WM_PAINT, WM_SETFONT, WM_SIZE, WS_CHILD,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

const OCR_RESULT_WIDTH: i32 = 480;
const OCR_RESULT_HEIGHT: i32 = 560;

pub(super) fn start_text_extraction(state: &mut UiState) {
    if state.ocr_pending {
        return;
    }
    let (width, height, pixels) = match selected_image(state) {
        Ok(image) => image,
        Err(error) => {
            set_error(state, error.clone());
            show_scroll_tip(state, error);
            return;
        }
    };
    let settings = state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .map_err(|_| ());
    let settings: Settings = match settings {
        Ok(settings) => settings,
        Err(_) => {
            let error = "OCR 设置锁已损坏".to_string();
            set_error(state, error.clone());
            show_scroll_tip(state, error);
            return;
        }
    };
    let credentials: TencentCloudCredentials = match state
        .tencent_credentials
        .lock()
        .map_err(|_| ())
        .ok()
        .and_then(|credentials| credentials.clone())
    {
        Some(credentials) => credentials,
        None => {
            let error = "请先在详情页配置腾讯云 OCR 的 SecretId 和 SecretKey".to_string();
            set_error(state, error.clone());
            show_scroll_tip(state, error);
            return;
        }
    };

    state.ocr_generation = state.ocr_generation.wrapping_add(1);
    let generation = state.ocr_generation;
    state.ocr_pending = true;
    show_scroll_tip(state, "正在提取文字…".into());

    let command_tx = state.command_tx.clone();
    let hwnd = state.hwnd as isize;
    thread::spawn(move || {
        let result = TencentOcrService::new(credentials)
            .map_err(|error| error.to_string())
            .and_then(|service| {
                service
                    .with_region(settings.ocr_region.clone())
                    .recognize_rgba(
                        width,
                        height,
                        &pixels,
                        RecognizeAgentRequest {
                            select_model: Some(settings.ocr_select_model),
                            ..RecognizeAgentRequest::default()
                        },
                    )
                    .map_err(|error| error.to_string())
            })
            .and_then(|response| {
                response
                    .text()
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| "腾讯云 OCR 没有返回文字".to_string())
            });
        if command_tx
            .send(super::Command::OcrFinished { generation, result })
            .is_ok()
        {
            unsafe {
                PostMessageW(hwnd as _, WAKE, 0, 0);
            }
        }
    });
}

pub(super) fn start_qr_recognition(state: &mut UiState) {
    if state.ocr_pending {
        return;
    }
    let (width, height, pixels) = match selected_image(state) {
        Ok(image) => image,
        Err(error) => {
            set_error(state, error.clone());
            show_scroll_tip(state, error);
            return;
        }
    };

    state.ocr_generation = state.ocr_generation.wrapping_add(1);
    let generation = state.ocr_generation;
    state.ocr_pending = true;
    show_scroll_tip(state, "正在识别二维码…".into());

    let command_tx = state.command_tx.clone();
    let hwnd = state.hwnd as isize;
    thread::spawn(move || {
        let result = crate::qr::decode_rgba(width, height, &pixels);
        if command_tx
            .send(super::Command::QrFinished { generation, result })
            .is_ok()
        {
            unsafe {
                PostMessageW(hwnd as _, WAKE, 0, 0);
            }
        }
    });
}

pub(super) fn finish_text_extraction(
    state: &mut UiState,
    generation: u64,
    result: Result<String, String>,
) {
    finish_recognition(state, generation, result, "文字提取结果");
}

pub(super) fn finish_qr_recognition(
    state: &mut UiState,
    generation: u64,
    result: Result<String, String>,
) {
    finish_recognition(state, generation, result, "二维码识别结果");
}

fn finish_recognition(
    state: &mut UiState,
    generation: u64,
    result: Result<String, String>,
    title: &'static str,
) {
    if generation != state.ocr_generation {
        return;
    }
    state.ocr_pending = false;
    if state.capture.is_none() && state.editor.is_none() {
        return;
    }

    match result {
        Ok(text) => {
            if let Ok(mut status) = state.status.lock() {
                status.ocr_text = Some(text.clone());
                status.error = None;
            }
            if !show_result_window(state, &text, title) {
                show_scroll_tip(state, "识别完成，但结果窗口无法打开".into());
            }
        }
        Err(error) => {
            set_error(state, error.clone());
            show_scroll_tip(state, error);
        }
    }
}

fn show_result_window(state: &mut UiState, text: &str, title: &'static str) -> bool {
    state.ocr_result_title = title;
    unsafe {
        if state.ocr_result_window.is_null() {
            let (x, y) = result_window_position(state);
            let class_name = wide("AgileTilesScreenshotCoreV2");
            let title = wide(title);
            let instance = GetModuleHandleW(null_mut());
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                x,
                y,
                OCR_RESULT_WIDTH,
                OCR_RESULT_HEIGHT,
                null_mut(),
                null_mut(),
                instance,
                state as *mut UiState as _,
            );
            if window.is_null() {
                return false;
            }
            state.ocr_result_window = window;

            let region =
                CreateRoundRectRgn(0, 0, OCR_RESULT_WIDTH + 1, OCR_RESULT_HEIGHT + 1, 22, 22);
            if !region.is_null() && SetWindowRgn(window, region, 1) == 0 {
                DeleteObject(region as HGDIOBJ);
            }

            let edit_class = wide("EDIT");
            let edit_style = WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_AUTOVSCROLL as u32
                | ES_READONLY as u32;
            state.ocr_result_edit = CreateWindowExW(
                0,
                edit_class.as_ptr(),
                wide("").as_ptr(),
                edit_style,
                0,
                0,
                0,
                0,
                window,
                null_mut(),
                instance,
                null_mut(),
            );
            if state.ocr_result_edit.is_null() {
                DestroyWindow(window);
                return false;
            }
            state.ocr_result_font = CreateFontW(
                -18,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                wide("Segoe UI").as_ptr(),
            );
            if !state.ocr_result_font.is_null() {
                SendMessageW(
                    state.ocr_result_edit,
                    WM_SETFONT,
                    state.ocr_result_font as usize,
                    1,
                );
            }
        }

        let content = wide(&to_windows_line_endings(text));
        SetWindowTextW(state.ocr_result_edit, content.as_ptr());
        state.ocr_result_copied = false;
        let title = wide(title);
        SetWindowTextW(state.ocr_result_window, title.as_ptr());
        layout_result_controls(state.ocr_result_window, state);
        ShowWindow(state.ocr_result_window, SW_SHOW);
        SetForegroundWindow(state.ocr_result_window);
        SetFocus(state.ocr_result_edit);
        InvalidateRect(state.ocr_result_window, null(), 0);
    }
    true
}

fn to_windows_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

fn result_window_position(state: &UiState) -> (i32, i32) {
    let screen = state
        .capture
        .as_ref()
        .map(|capture| capture.screen_rect)
        .or_else(|| state.screen.as_ref().map(|screen| screen.bounds()));
    if let Some(screen) = screen {
        let x = screen.left + ((screen.right - screen.left - OCR_RESULT_WIDTH).max(0) / 2);
        let y = screen.top + ((screen.bottom - screen.top - OCR_RESULT_HEIGHT).max(0) / 2);
        (x, y)
    } else {
        (120, 120)
    }
}

unsafe fn layout_result_controls(hwnd: HWND, state: &UiState) {
    if state.ocr_result_edit.is_null() {
        return;
    }
    let mut client: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut client);
    let width = (client.right - client.left).max(OCR_RESULT_WIDTH);
    let height = (client.bottom - client.top).max(OCR_RESULT_HEIGHT);
    SetWindowPos(
        state.ocr_result_edit,
        null_mut(),
        20,
        68,
        width - 40,
        height - 140,
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

pub(super) unsafe fn result_window_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_result_window(hwnd, state);
            0
        }
        WM_SIZE => {
            layout_result_controls(hwnd, state);
            0
        }
        WM_NCHITTEST => {
            let screen_x = (lparam as i16) as i32;
            let screen_y = ((lparam >> 16) as i16) as i32;
            let mut window_rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut window_rect);
            let client_x = screen_x - window_rect.left;
            let client_y = screen_y - window_rect.top;
            if (0..54).contains(&client_y) && client_x < OCR_RESULT_WIDTH - 54 {
                2 // HTCAPTION: let the user drag the result card.
            } else {
                1 // HTCLIENT
            }
        }
        WM_LBUTTONUP => {
            let x = (lparam as i16) as i32;
            let y = ((lparam >> 16) as i16) as i32;
            let width = OCR_RESULT_WIDTH;
            let height = OCR_RESULT_HEIGHT;
            if x >= width - 54 && y < 54 {
                DestroyWindow(hwnd);
                return 0;
            }
            if x >= width - 100 && x <= width - 20 && y >= height - 56 && y <= height - 20 {
                let text = state
                    .status
                    .lock()
                    .ok()
                    .and_then(|status| status.ocr_text.clone());
                if let Some(text) = text {
                    match Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)) {
                        Ok(()) => {
                            state.ocr_result_copied = true;
                            super::request_cancel(state);
                            return 0;
                        }
                        Err(error) => {
                            if let Ok(mut status) = state.status.lock() {
                                status.error = Some(format!("复制识别结果失败：{error}"));
                            }
                        }
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            if state.ocr_result_window == hwnd {
                state.ocr_result_window = null_mut();
                state.ocr_result_edit = null_mut();
                state.ocr_result_copied = false;
                if !state.ocr_result_font.is_null() {
                    DeleteObject(state.ocr_result_font as HGDIOBJ);
                    state.ocr_result_font = null_mut();
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn paint_result_window(hwnd: HWND, state: &UiState) {
    let mut paint: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut paint);
    let mut client: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut client);
    let width = client.right - client.left;
    let height = client.bottom - client.top;
    let old_font = if state.ocr_result_font.is_null() {
        null_mut()
    } else {
        SelectObject(dc, state.ocr_result_font as HGDIOBJ)
    };

    let white = CreateSolidBrush(0x00ffffff);
    let border = CreatePen(0, 1, 0x00d9d9d9);
    let old_brush = SelectObject(dc, white as HGDIOBJ);
    let old_pen = SelectObject(dc, border as HGDIOBJ);
    RoundRect(dc, 0, 0, width, height, 22, 22);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(border as HGDIOBJ);
    DeleteObject(white as HGDIOBJ);

    SetBkMode(dc, 1);
    SetTextColor(dc, 0x00434343);
    let title = wide(state.ocr_result_title);
    let mut title_rect = RECT {
        left: 24,
        top: 14,
        right: width - 60,
        bottom: 48,
    };
    DrawTextW(dc, title.as_ptr(), -1, &mut title_rect, 0x20 | 0x4 | 0x8000);

    SetTextColor(dc, 0x006b7280);
    let close = wide("×");
    let mut close_rect = RECT {
        left: width - 50,
        top: 12,
        right: width - 16,
        bottom: 48,
    };
    DrawTextW(
        dc,
        close.as_ptr(),
        -1,
        &mut close_rect,
        0x1 | 0x4 | 0x20 | 0x8000,
    );

    let button_left = width - 100;
    let button_top = height - 56;
    let blue = CreateSolidBrush(0x00ff5d16);
    let blue_pen = CreatePen(0, 1, 0x00ff5d16);
    let old_brush = SelectObject(dc, blue as HGDIOBJ);
    let old_pen = SelectObject(dc, blue_pen as HGDIOBJ);
    RoundRect(dc, button_left, button_top, width - 20, height - 20, 10, 10);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(blue_pen as HGDIOBJ);
    DeleteObject(blue as HGDIOBJ);

    SetTextColor(dc, 0x00ffffff);
    let button_text = wide(if state.ocr_result_copied {
        "已复制"
    } else {
        "复制"
    });
    let mut button_rect = RECT {
        left: button_left,
        top: button_top,
        right: width - 20,
        bottom: height - 20,
    };
    DrawTextW(
        dc,
        button_text.as_ptr(),
        -1,
        &mut button_rect,
        0x1 | 0x4 | 0x20 | 0x8000,
    );

    if !old_font.is_null() {
        SelectObject(dc, old_font);
    }
    EndPaint(hwnd, &paint);
}

pub(super) fn invalidate_pending(state: &mut UiState) {
    state.ocr_generation = state.ocr_generation.wrapping_add(1);
    state.ocr_pending = false;
}

pub(super) fn close_session_transients(state: &mut UiState) {
    invalidate_pending(state);
    unsafe {
        if !state.ocr_result_window.is_null() {
            DestroyWindow(state.ocr_result_window);
        }
    }
    super::scroll_ui::destroy_scroll_tip(state);
    if let Ok(mut status) = state.status.lock() {
        status.ocr_text = None;
        status.error = None;
    }
}

fn selected_image(state: &UiState) -> Result<(usize, usize, Vec<u8>), String> {
    if let Some(editor) = state.editor.as_ref() {
        if editor.recording_preview {
            return Err("录屏预览中无法提取文字".into());
        }
        return Ok((
            editor.source_width,
            editor.source_height,
            editor.source_pixels.clone(),
        ));
    }

    let capture = state
        .capture
        .as_ref()
        .ok_or_else(|| "请先选择截图区域，再提取文字".to_string())?;
    let selection = capture
        .selection
        .ok_or_else(|| "请先选择截图区域，再提取文字".to_string())?;

    if let Some((width, height, pixels)) = capture
        .scroll
        .as_ref()
        .and_then(|scroll| scroll.assembled.as_ref())
    {
        return Ok((*width, *height, pixels.clone()));
    }

    let screen = state
        .screen
        .as_ref()
        .ok_or_else(|| "截图图像不可用，无法提取文字".to_string())?;
    if selection.clipped_to(screen.bounds()) != Some(selection) {
        return Err("截图选区超出了图像范围".into());
    }
    Ok((
        selection.width(),
        selection.height(),
        screen.crop(selection),
    ))
}
