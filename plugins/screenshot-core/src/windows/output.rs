use super::editor::finish_editor;
use super::input::VK_ESCAPE_CODE;
use super::paint::draw_rgba_preview;
use super::scroll_ui::{
    destroy_scroll_border, destroy_scroll_masks, destroy_scroll_preview, stop_scroll_input,
};
use super::{
    clear_error, set_error, update_status, wide, PinnedImageWindow, UiState, TIMER_SCROLL,
};
use crate::export::{compose as compose_export, copy_to_clipboard, save_png, save_png_to_path};
use crate::model::Settings;

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FrameRect, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::MK_CONTROL;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_EXPLORER, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetSystemMetrics,
    GetWindowRect, IsWindowVisible, KillTimer, SetLayeredWindowAttributes, SetWindowPos,
    ShowWindow, HTCAPTION, LWA_ALPHA, SWP_NOACTIVATE, SW_HIDE, SW_SHOW, WM_DESTROY, WM_KEYDOWN,
    WM_MBUTTONUP, WM_MOUSEWHEEL, WM_NCHITTEST, WM_NCMBUTTONUP, WM_NCRBUTTONUP, WM_PAINT,
    WM_RBUTTONUP, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub(super) fn hide_capture_windows(state: &UiState) {
    let mut windows = vec![
        state.overlay,
        state.scroll_control,
        state.scroll_tooltip,
        state.scroll_preview,
        state.scroll_tip,
        state.scroll_border,
        state.recording_toolbar,
    ];
    windows.extend(state.scroll_masks.iter().copied());

    for window in windows {
        if !window.is_null() && unsafe { IsWindowVisible(window) != 0 } {
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
        }
    }
}

pub(super) fn save_png_with_dialog(
    owner: HWND,
    settings: &Settings,
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<Option<String>, String> {
    let filter = wide("PNG image (*.png)\0*.png\0");
    let title = wide("Save screenshot");
    let extension = wide("png");
    let mut file = vec![0u16; 512];
    let default_name = wide("AgileTiles.png");
    let copy_len = default_name.len().min(file.len() - 1);
    file[..copy_len].copy_from_slice(&default_name[..copy_len]);
    let initial_dir = settings.save_directory.trim();
    let initial_dir_buffer = (!initial_dir.is_empty()).then(|| wide(initial_dir));
    let initial_dir_ptr = initial_dir_buffer
        .as_ref()
        .map(|value| value.as_ptr())
        .unwrap_or(null());
    let mut dialog: OPENFILENAMEW = unsafe { zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.lpstrInitialDir = initial_dir_ptr;
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT;
    dialog.lpstrDefExt = extension.as_ptr();
    if unsafe { GetSaveFileNameW(&mut dialog) } == 0 {
        return Ok(None);
    }
    let end = file
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(file.len());
    let selected = String::from_utf16(&file[..end]).map_err(|error| error.to_string())?;
    if selected.trim().is_empty() {
        return Ok(None);
    }
    let mut path = std::path::PathBuf::from(selected);
    if path.extension().is_none() {
        path.set_extension("png");
    }
    save_png_to_path(&path, pixels, width, height)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

pub(super) fn finish(state: &mut UiState) {
    finish_inner(state, false, false, None);
}

pub(super) fn finish_with_dialog(state: &mut UiState) {
    finish_inner(state, true, false, None);
}

pub(super) fn copy_capture(state: &mut UiState) {
    finish_inner(state, false, true, None);
}

pub(super) fn finish_inner(
    state: &mut UiState,
    show_save_dialog: bool,
    copy_only: bool,
    already_saved_path: Option<String>,
) {
    if state.editor.is_some() {
        finish_editor(state, show_save_dialog, copy_only, already_saved_path);
        return;
    }
    let mut saved_path = already_saved_path;
    if show_save_dialog && saved_path.is_none() {
        let export = state.capture.as_ref().and_then(|capture| {
            let selection = capture.selection?;
            let screen = state.screen.as_ref()?;
            let (width, height, pixels) = capture
                .scroll
                .as_ref()
                .and_then(|scroll| scroll.assembled.as_ref())
                .map(|(width, height, pixels)| (*width, *height, pixels.clone()))
                .unwrap_or_else(|| {
                    (
                        selection.width(),
                        selection.height(),
                        screen.crop(selection),
                    )
                });
            let settings = state
                .settings
                .lock()
                .map(|settings| settings.clone())
                .unwrap_or_default();
            let pixels = compose_export(
                &settings,
                width,
                height,
                pixels,
                capture.annotations.clone(),
                selection,
            );
            Some((settings, width, height, pixels))
        });
        if let Some((settings, width, height, pixels)) = export {
            hide_capture_windows(state);
            match save_png_with_dialog(state.hwnd, &settings, &pixels, width, height) {
                Ok(Some(path)) => saved_path = Some(path),
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
    }
    super::ocr_ui::close_session_transients(state);
    stop_scroll_input(state);
    let Some(mut capture) = state.capture.take() else {
        return;
    };
    if let Some(scroll) = capture.scroll.as_mut() {
        scroll.finish();
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
        unsafe {
            DestroyWindow(state.scroll_control);
        }
        state.scroll_control = null_mut();
    }
    destroy_scroll_preview(state);
    destroy_scroll_masks(state);
    destroy_scroll_border(state);
    let Some(selection) = capture.selection else {
        update_status(state, false);
        return;
    };
    state.render = None;
    let screen = state.screen.take();
    let settings = state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default();
    let (width, height, pixels) = capture
        .scroll
        .and_then(|scroll| scroll.assembled)
        .unwrap_or_else(|| {
            (
                selection.width(),
                selection.height(),
                screen
                    .as_ref()
                    .map(|value| value.crop(selection))
                    .unwrap_or_default(),
            )
        });
    let pixels = compose_export(
        &settings,
        width,
        height,
        pixels,
        capture.annotations,
        selection,
    );
    let path_result = if copy_only {
        Ok(None)
    } else if let Some(path) = saved_path {
        Ok(Some(path))
    } else {
        Ok(save_png(&settings, &pixels, width, height))
    };
    let path = match path_result {
        Ok(path) => path,
        Err(error) => {
            set_error(state, error);
            None
        }
    };
    if path.is_some() {
        if let Ok(mut status) = state.status.lock() {
            status.error = None;
        }
    } else if !show_save_dialog && !copy_only {
        set_error(state, "PNG 保存失败，请检查保存目录".into());
    }
    let should_copy = copy_only || settings.auto_copy;
    if should_copy {
        if let Err(error) = copy_to_clipboard(width, height, &pixels) {
            set_error(state, format!("复制截图失败：{error}"));
        } else if copy_only {
            clear_error(state);
        }
    }
    state.last_size = Some((width, height));
    if !copy_only || state.last_path.is_none() {
        state.last_path = path;
    }
    state.last_image = Some((width, height, pixels.clone()));
    if let Ok(mut image) = state.image.lock() {
        *image = Some((width, height, pixels));
    }
    update_status(state, false);
}

pub(super) fn pin_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    _lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    let Some(pin_index) = state.pinned_images.iter().position(|pin| pin.hwnd == hwnd) else {
        return unsafe { DefWindowProcW(hwnd, message, wparam, _lparam) };
    };
    unsafe {
        match message {
            WM_PAINT => {
                let mut paint_struct: PAINTSTRUCT = zeroed();
                let dc = BeginPaint(hwnd, &mut paint_struct);
                if let Some(pin) = state.pinned_images.get(pin_index) {
                    let (width, height, pixels) = &pin.image;
                    let mut rect: RECT = zeroed();
                    GetClientRect(hwnd, &mut rect);
                    draw_rgba_preview(
                        dc,
                        pixels,
                        *width as i32,
                        *height as i32,
                        0,
                        0,
                        (rect.right - rect.left).max(1),
                        (rect.bottom - rect.top).max(1),
                    );
                    let black = CreateSolidBrush(0x0000_0000);
                    if !black.is_null() {
                        FrameRect(dc, &rect, black);
                        DeleteObject(black);
                    }
                    rect.left += 1;
                    rect.top += 1;
                    rect.right -= 1;
                    rect.bottom -= 1;
                    if rect.right > rect.left && rect.bottom > rect.top {
                        let white = CreateSolidBrush(0x00ff_ffff);
                        if !white.is_null() {
                            FrameRect(dc, &rect, white);
                            DeleteObject(white);
                        }
                    }
                }
                EndPaint(hwnd, &paint_struct);
                0
            }
            WM_NCHITTEST => HTCAPTION as LRESULT,
            WM_MOUSEWHEEL => {
                let delta = ((wparam >> 16) as i16) as f32 / 120.0;
                if delta == 0.0 {
                    return 0;
                }
                if (wparam as u32 & 0xffff) & MK_CONTROL != 0 {
                    let pin = &mut state.pinned_images[pin_index];
                    let opacity =
                        (pin.opacity as i32 + (delta * 20.0) as i32).clamp(32, 255) as u8;
                    pin.opacity = opacity;
                    SetLayeredWindowAttributes(hwnd, 0, opacity, LWA_ALPHA);
                    return 0;
                }
                let mut rect: RECT = zeroed();
                if GetWindowRect(hwnd, &mut rect) == 0 {
                    return 0;
                }
                let old_width = (rect.right - rect.left).max(1);
                let old_height = (rect.bottom - rect.top).max(1);
                let (image_width, image_height) = {
                    let image = &state.pinned_images[pin_index].image;
                    (image.0, image.1)
                };
                let max_width = GetSystemMetrics(0).max(640) * 2;
                let max_height = GetSystemMetrics(1).max(480) * 2;
                let min_scale = 80.0 / image_width.max(image_height).max(1) as f64;
                let max_scale = (max_width as f64 / image_width as f64)
                    .min(max_height as f64 / image_height as f64);
                let current_scale = old_width as f64 / image_width as f64;
                let scale = (current_scale * 1.1_f64.powf(delta as f64))
                    .clamp(min_scale, max_scale.max(min_scale));
                let new_width = (image_width as f64 * scale).round() as i32;
                let new_height = (image_height as f64 * scale).round() as i32;
                let mut cursor: POINT = zeroed();
                GetCursorPos(&mut cursor);
                let x = cursor.x
                    - (((cursor.x - rect.left) as f64 * new_width as f64 / old_width as f64).round()
                        as i32);
                let y = cursor.y
                    - (((cursor.y - rect.top) as f64 * new_height as f64 / old_height as f64).round()
                        as i32);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    x,
                    y,
                    new_width,
                    new_height,
                    SWP_NOACTIVATE,
                );
                0
            }
            WM_MBUTTONUP | WM_NCMBUTTONUP => {
                let mut rect: RECT = zeroed();
                if GetWindowRect(hwnd, &mut rect) == 0 {
                    return 0;
                }
                let current = (
                    (rect.right - rect.left).max(1),
                    (rect.bottom - rect.top).max(1),
                );
                let pin = &mut state.pinned_images[pin_index];
                if current == pin.default_size && pin.opacity == 255 {
                    if let Some(restore_size) = pin.restore_size.take() {
                        pin.restore_size = Some(pin.default_size);
                        resize_pin_around_center(hwnd, rect, restore_size);
                    }
                } else {
                    pin.restore_size = Some(current);
                    resize_pin_around_center(hwnd, rect, pin.default_size);
                }
                pin.opacity = 255;
                SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
                0
            }
            WM_RBUTTONUP | WM_NCRBUTTONUP => {
                DestroyWindow(hwnd);
                0
            }
            WM_KEYDOWN if wparam as u32 == VK_ESCAPE_CODE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                state.pinned_images.retain(|pin| pin.hwnd != hwnd);
                if let Ok(mut status) = state.status.lock() {
                    status.pinned = !state.pinned_images.is_empty();
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, _lparam),
        }
    }
}

unsafe fn resize_pin_around_center(hwnd: HWND, rect: RECT, size: (i32, i32)) {
    let old_width = (rect.right - rect.left).max(1);
    let old_height = (rect.bottom - rect.top).max(1);
    let center_x = rect.left + old_width / 2;
    let center_y = rect.top + old_height / 2;
    SetWindowPos(
        hwnd,
        std::ptr::null_mut(),
        center_x - size.0 / 2,
        center_y - size.1 / 2,
        size.0,
        size.1,
        SWP_NOACTIVATE,
    );
}

pub(super) fn pin_last(state: &mut UiState) {
    let Some((width, height, pixels)) = state.last_image.clone() else {
        set_error(state, "没有可贴图的截图".into());
        return;
    };
    unsafe {
        let screen_left = GetSystemMetrics(76);
        let screen_top = GetSystemMetrics(77);
        let screen_width = GetSystemMetrics(78).max(640);
        let screen_height = GetSystemMetrics(79).max(480);
        let available_width = (screen_width - 60).max(160) as f64;
        let available_height = (screen_height - 80).max(120) as f64;
        let image_width = width.max(1) as f64;
        let image_height = height.max(1) as f64;
        let mut scale = (720.0 / image_width)
            .min(available_width / image_width)
            .min(available_height / image_height)
            .min(1.0);
        if image_width * scale < 160.0
            && available_width >= 160.0
            && available_height / image_height >= 160.0 / image_width
        {
            scale = 160.0 / image_width;
        }
        let target_width = (image_width * scale).round().max(1.0) as i32;
        let target_height = (image_height * scale).round().max(1.0) as i32;
        let cascade = (state.pinned_images.len() as i32 % 6) * 28;
        let x = (screen_left + screen_width - target_width - 30 - cascade).max(screen_left + 10);
        let max_y = screen_top + screen_height - target_height - 10;
        let y = (screen_top + 30 + cascade).min(max_y.max(screen_top + 10));
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let instance = GetModuleHandleW(null());
        let window = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            x,
            y,
            target_width,
            target_height,
            null_mut(),
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        if window.is_null() {
            set_error(state, "无法创建贴图窗口".into());
            return;
        }
        state.pinned_images.push(PinnedImageWindow {
            hwnd: window,
            image: (width, height, pixels),
            default_size: (target_width, target_height),
            restore_size: None,
            opacity: 255,
        });
        SetLayeredWindowAttributes(window, 0, 255, LWA_ALPHA);
        ShowWindow(window, SW_SHOW);
        if let Ok(mut status) = state.status.lock() {
            status.pinned = true;
            status.error = None;
        }
    }
}
