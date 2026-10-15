use super::editor::finish_editor;
use super::input::VK_ESCAPE_CODE;
use super::paint::draw_rgba_preview;
use super::scroll_ui::{
    destroy_scroll_border, destroy_scroll_masks, destroy_scroll_preview, stop_scroll_input,
};
use super::{clear_error, set_error, update_status, wide, UiState, TIMER_SCROLL};
use crate::export::{compose as compose_export, copy_to_clipboard, save_png, save_png_to_path};
use crate::model::Settings;

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_EXPLORER, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetSystemMetrics, KillTimer,
    IsWindowVisible, ShowWindow, SW_HIDE, SW_SHOW, WM_DESTROY, WM_KEYDOWN, WM_PAINT,
    WM_RBUTTONUP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
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
    unsafe {
        match message {
            WM_PAINT => {
                let mut paint_struct: PAINTSTRUCT = zeroed();
                let dc = BeginPaint(hwnd, &mut paint_struct);
                if let Some((width, height, pixels)) = state.pin_image.as_ref() {
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
                }
                EndPaint(hwnd, &paint_struct);
                0
            }
            WM_RBUTTONUP => {
                DestroyWindow(hwnd);
                state.pin_window = null_mut();
                state.pin_image = None;
                if let Ok(mut status) = state.status.lock() {
                    status.pinned = false;
                }
                0
            }
            WM_KEYDOWN if wparam as u32 == VK_ESCAPE_CODE => {
                DestroyWindow(hwnd);
                state.pin_window = null_mut();
                state.pin_image = None;
                if let Ok(mut status) = state.status.lock() {
                    status.pinned = false;
                }
                0
            }
            WM_DESTROY => {
                state.pin_window = null_mut();
                state.pin_image = None;
                if let Ok(mut status) = state.status.lock() {
                    status.pinned = false;
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, _lparam),
        }
    }
}

pub(super) fn pin_last(state: &mut UiState) {
    let Some((width, height, pixels)) = state.last_image.clone() else {
        set_error(state, "没有可贴图的截图".into());
        return;
    };
    unsafe {
        if !state.pin_window.is_null() {
            DestroyWindow(state.pin_window);
        }
        let screen_width = GetSystemMetrics(78).max(640);
        let target_width = (width as i32).clamp(160, 720);
        let target_height = ((height as f32 * target_width as f32 / width as f32) as i32)
            .min(GetSystemMetrics(79).max(480) - 80)
            .max(80);
        let x = GetSystemMetrics(76) + screen_width - target_width - 30;
        let y = GetSystemMetrics(77) + 30;
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let instance = GetModuleHandleW(null());
        state.pin_image = Some((width, height, pixels));
        state.pin_window = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
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
        ShowWindow(state.pin_window, SW_SHOW);
        if let Ok(mut status) = state.status.lock() {
            status.pinned = true;
            status.error = None;
        }
    }
}
