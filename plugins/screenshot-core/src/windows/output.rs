use super::editor::finish_editor;
use super::input::VK_ESCAPE_CODE;
use super::paint::draw_rgba_preview;
use super::scroll_ui::{
    destroy_scroll_border, destroy_scroll_masks, destroy_scroll_preview, stop_scroll_input,
};
use super::{
    clear_error, set_error, update_status, wide, PinnedImageWindow, UiState, TIMER_SCROLL,
};
use crate::export::{
    compose_with_options, copy_to_clipboard, expand_filename_template, render_text_pin_card,
    save_image_to_path, save_png, ComposeOptions,
};
use crate::model::Settings;

use arboard::Clipboard;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FrameRect, InvalidateRect, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::MK_CONTROL;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_EXPLORER, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetSystemMetrics,
    GetWindowRect, IsWindowVisible, KillTimer, SetLayeredWindowAttributes, SetWindowPos,
    ShowWindow, HTCAPTION, HWND_NOTOPMOST, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_HIDE, SW_SHOW, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_MBUTTONUP,
    WM_MOUSEWHEEL, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCMBUTTONUP, WM_NCRBUTTONUP, WM_PAINT,
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
    let filter = wide(
        "PNG 图片 (*.png)\0*.png\0JPEG 图片 (*.jpg)\0*.jpg\0WebP 图片 (*.webp)\0*.webp\0",
    );
    let title = wide("保存截图");
    let default_ext = match settings.save_format.as_str() {
        "jpg" => "jpg",
        "webp" => "webp",
        _ => "png",
    };
    let extension = wide(default_ext);
    let mut file = vec![0u16; 512];
    let stem = expand_filename_template(&settings.file_name_template);
    let default_name = wide(&format!("{stem}.{default_ext}"));
    let copy_len = default_name.len().min(file.len() - 1);
    file[..copy_len].copy_from_slice(&default_name[..copy_len]);
    let initial_dir = settings.resolved_save_directory();
    let _ = std::fs::create_dir_all(&initial_dir);
    let initial_dir_str = initial_dir.to_string_lossy().into_owned();
    let initial_dir_buffer = (!initial_dir_str.is_empty()).then(|| wide(&initial_dir_str));
    let initial_dir_ptr = initial_dir_buffer
        .as_ref()
        .map(|value| value.as_ptr())
        .unwrap_or(null());
    let mut dialog: OPENFILENAMEW = unsafe { zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.nFilterIndex = match default_ext {
        "jpg" => 2,
        "webp" => 3,
        _ => 1,
    };
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
        path.set_extension(default_ext);
    }
    save_image_to_path(&path, pixels, width, height)?;
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
            let (width, height, pixels) = if capture.show_translated_overlay {
                capture
                    .translated_image
                    .as_ref()
                    .map(|(w, h, p)| (*w, *h, p.clone()))
            } else {
                None
            }
            .or_else(|| {
                capture
                    .scroll
                    .as_ref()
                    .and_then(|scroll| scroll.assembled.as_ref())
                    .map(|(w, h, p)| (*w, *h, p.clone()))
            })
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
            let options = ComposeOptions {
                corner_radius: capture.corner_radius,
                drop_shadow: capture.drop_shadow,
            };
            let (out_w, out_h, pixels) = compose_with_options(
                &settings,
                width,
                height,
                pixels,
                capture.annotations.clone(),
                selection,
                options,
            );
            Some((settings, out_w, out_h, pixels))
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
    let is_scroll = capture.scroll.is_some();
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
    let (width, height, pixels) = if capture.show_translated_overlay {
        capture.translated_image.take()
    } else {
        None
    }
    .or_else(|| capture.scroll.and_then(|scroll| scroll.assembled))
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
    let options = ComposeOptions {
        corner_radius: capture.corner_radius,
        drop_shadow: capture.drop_shadow,
    };
    let (width, height, pixels) = compose_with_options(
        &settings,
        width,
        height,
        pixels,
        capture.annotations,
        selection,
        options,
    );

    let finish_mode = settings.finish_action.as_str();
    let should_save = if copy_only {
        false
    } else if saved_path.is_some() || show_save_dialog {
        true
    } else {
        finish_mode != "copy_only" || settings.auto_save
    };

    let path = if !should_save {
        None
    } else if let Some(path) = saved_path {
        Some(path)
    } else {
        save_png(&settings, &pixels, width, height)
    };

    if let Some(ref p) = path {
        crate::history::record_item(
            if is_scroll { "scroll" } else { "screenshot" },
            p,
            width,
            height,
        );
        if let Ok(mut status) = state.status.lock() {
            status.error = None;
        }
    } else if should_save && !show_save_dialog && !copy_only {
        set_error(state, "图片保存失败，请检查保存目录".into());
    }

    let should_copy = if copy_only {
        true
    } else if finish_mode == "save_only" {
        false
    } else {
        settings.auto_copy || finish_mode == "copy_only" || finish_mode == "copy_and_save"
    };
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
                    if pin.show_border || pin.thumbnail_mode {
                        let border_col = if pin.thumbnail_mode {
                            0x0059_C734 // SnowAir green highlight for thumbnail
                        } else {
                            0x0038_383C
                        };
                        let outer = CreateSolidBrush(border_col);
                        if !outer.is_null() {
                            FrameRect(dc, &rect, outer);
                            DeleteObject(outer);
                        }
                        rect.left += 1;
                        rect.top += 1;
                        rect.right -= 1;
                        rect.bottom -= 1;
                        if rect.right > rect.left && rect.bottom > rect.top {
                            let inner = CreateSolidBrush(if pin.thumbnail_mode {
                                0x0059_C734
                            } else {
                                0x00FF_FFFF
                            });
                            if !inner.is_null() {
                                FrameRect(dc, &rect, inner);
                                DeleteObject(inner);
                            }
                        }
                    }
                }
                EndPaint(hwnd, &paint_struct);
                0
            }
            WM_NCHITTEST => HTCAPTION as LRESULT,
            WM_LBUTTONDBLCLK | WM_NCLBUTTONDBLCLK => {
                let action = state
                    .settings
                    .lock()
                    .map(|s| s.pin_double_click.clone())
                    .unwrap_or_else(|_| "thumbnail".into());
                match action.as_str() {
                    "close" => {
                        DestroyWindow(hwnd);
                    }
                    "copy" => {
                        let (w, h, px) = state.pinned_images[pin_index].image.clone();
                        let _ = copy_to_clipboard(w, h, &px);
                    }
                    _ => {
                        toggle_pin_thumbnail(state, pin_index);
                    }
                }
                0
            }
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
                state.pinned_images[pin_index].thumbnail_mode = false;
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
                let cursor_zoom = state
                    .settings
                    .lock()
                    .map(|s| s.pin_cursor_zoom)
                    .unwrap_or(true);
                let (x, y) = if cursor_zoom {
                    let mut cursor: POINT = zeroed();
                    GetCursorPos(&mut cursor);
                    (
                        cursor.x
                            - (((cursor.x - rect.left) as f64 * new_width as f64
                                / old_width as f64)
                                .round() as i32),
                        cursor.y
                            - (((cursor.y - rect.top) as f64 * new_height as f64
                                / old_height as f64)
                                .round() as i32),
                    )
                } else {
                    let cx = rect.left + old_width / 2;
                    let cy = rect.top + old_height / 2;
                    (cx - new_width / 2, cy - new_height / 2)
                };
                SetWindowPos(
                    hwnd,
                    null_mut(),
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
                pin.thumbnail_mode = false;
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
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_RBUTTONUP | WM_NCRBUTTONUP => {
                super::pin_menu::open_pin_menu(state, pin_index);
                0
            }
            WM_KEYDOWN if wparam as u32 == VK_ESCAPE_CODE => {
                super::pin_menu::destroy_pin_menu(state);
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                if state
                    .pin_menu
                    .as_ref()
                    .is_some_and(|m| m.target_pin_hwnd == hwnd)
                {
                    super::pin_menu::destroy_pin_menu(state);
                }
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

unsafe fn toggle_pin_thumbnail(state: &mut UiState, pin_index: usize) {
    let Some(pin) = state.pinned_images.get_mut(pin_index) else {
        return;
    };
    let hwnd = pin.hwnd;
    let mut rect: RECT = zeroed();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return;
    }
    if pin.thumbnail_mode {
        pin.thumbnail_mode = false;
        let target = pin.restore_size.take().unwrap_or(pin.default_size);
        resize_pin_around_center(hwnd, rect, target);
    } else {
        pin.thumbnail_mode = true;
        pin.restore_size = Some((
            (rect.right - rect.left).max(1),
            (rect.bottom - rect.top).max(1),
        ));
        let (iw, ih) = (pin.image.0.max(1) as f64, pin.image.1.max(1) as f64);
        let scale = (96.0 / iw).min(96.0 / ih).min(1.0);
        let tw = (iw * scale).round().max(56.0) as i32;
        let th = (ih * scale).round().max(56.0) as i32;
        resize_pin_around_center(hwnd, rect, (tw, th));
    }
    InvalidateRect(hwnd, null(), 0);
}

fn rotate_rgba_90_cw(w: usize, h: usize, pixels: &[u8]) -> (usize, usize, Vec<u8>) {
    let new_w = h;
    let new_h = w;
    let mut out = vec![0u8; new_w * new_h * 4];
    for y in 0..h {
        for x in 0..w {
            let src = (y * w + x) * 4;
            let nx = h - 1 - y;
            let ny = x;
            let dst = (ny * new_w + nx) * 4;
            out[dst..dst + 4].copy_from_slice(&pixels[src..src + 4]);
        }
    }
    (new_w, new_h, out)
}

fn flip_rgba_horizontal(w: usize, h: usize, pixels: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let src = (y * w + x) * 4;
            let dst = (y * w + (w - 1 - x)) * 4;
            out[dst..dst + 4].copy_from_slice(&pixels[src..src + 4]);
        }
    }
    out
}

fn flip_rgba_vertical(w: usize, h: usize, pixels: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let row_bytes = w * 4;
    for y in 0..h {
        let src = y * row_bytes;
        let dst = (h - 1 - y) * row_bytes;
        out[dst..dst + row_bytes].copy_from_slice(&pixels[src..src + row_bytes]);
    }
    out
}

pub(super) unsafe fn execute_pin_menu_action(
    state: &mut UiState,
    pin_hwnd: HWND,
    action: super::pin_menu::PinMenuAction,
) {
    use super::pin_menu::PinMenuAction;
    let Some(pin_index) = state.pinned_images.iter().position(|p| p.hwnd == pin_hwnd) else {
        return;
    };
    match action {
        PinMenuAction::Copy => {
            if let Some(pin) = state.pinned_images.get(pin_index) {
                let (w, h, px) = &pin.image;
                let _ = copy_to_clipboard(*w, *h, px);
            }
        }
        PinMenuAction::Save => {
            if let Some(pin) = state.pinned_images.get(pin_index) {
                let (w, h, px) = pin.image.clone();
                let settings = state
                    .settings
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default();
                if let Ok(Some(path)) = save_png_with_dialog(pin_hwnd, &settings, &px, w, h) {
                    crate::history::record_item("screenshot", &path, w, h);
                }
            }
        }
        PinMenuAction::Ocr => {
            if let Some(pin) = state.pinned_images.get(pin_index) {
                let (w, h, px) = pin.image.clone();
                super::ocr_ui::start_ocr_for_rgba(state, w, h, px);
            }
        }
        PinMenuAction::Rotate90 => {
            if let Some(pin) = state.pinned_images.get_mut(pin_index) {
                let (nw, nh, npx) = rotate_rgba_90_cw(pin.image.0, pin.image.1, &pin.image.2);
                pin.image = (nw, nh, npx);
                pin.default_size = (pin.default_size.1, pin.default_size.0);
                pin.thumbnail_mode = false;
                let mut rect: RECT = zeroed();
                if GetWindowRect(pin_hwnd, &mut rect) != 0 {
                    let old_w = (rect.right - rect.left).max(1);
                    let old_h = (rect.bottom - rect.top).max(1);
                    resize_pin_around_center(pin_hwnd, rect, (old_h, old_w));
                }
                InvalidateRect(pin_hwnd, null(), 0);
            }
        }
        PinMenuAction::FlipH => {
            if let Some(pin) = state.pinned_images.get_mut(pin_index) {
                pin.image.2 = flip_rgba_horizontal(pin.image.0, pin.image.1, &pin.image.2);
                InvalidateRect(pin_hwnd, null(), 0);
            }
        }
        PinMenuAction::FlipV => {
            if let Some(pin) = state.pinned_images.get_mut(pin_index) {
                pin.image.2 = flip_rgba_vertical(pin.image.0, pin.image.1, &pin.image.2);
                InvalidateRect(pin_hwnd, null(), 0);
            }
        }
        PinMenuAction::ToggleThumbnail => {
            toggle_pin_thumbnail(state, pin_index);
        }
        PinMenuAction::ToggleTopmost => {
            if let Some(pin) = state.pinned_images.get_mut(pin_index) {
                pin.topmost = !pin.topmost;
                let z = if pin.topmost {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                };
                SetWindowPos(pin_hwnd, z, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        PinMenuAction::ToggleBorder => {
            if let Some(pin) = state.pinned_images.get_mut(pin_index) {
                pin.show_border = !pin.show_border;
                InvalidateRect(pin_hwnd, null(), 0);
            }
        }
        PinMenuAction::Opacity(alpha) => {
            if let Some(pin) = state.pinned_images.get_mut(pin_index) {
                pin.opacity = alpha.max(32);
                SetLayeredWindowAttributes(pin_hwnd, 0, pin.opacity, LWA_ALPHA);
            }
        }
        PinMenuAction::Scale(pct) => {
            if let Some(pin) = state.pinned_images.get_mut(pin_index) {
                pin.thumbnail_mode = false;
                let factor = f64::from(pct) / 100.0;
                let mut rect: RECT = zeroed();
                if GetWindowRect(pin_hwnd, &mut rect) != 0 {
                    let tw = ((pin.image.0 as f64) * factor).round().max(80.0) as i32;
                    let th = ((pin.image.1 as f64) * factor).round().max(60.0) as i32;
                    resize_pin_around_center(pin_hwnd, rect, (tw, th));
                    InvalidateRect(pin_hwnd, null(), 0);
                }
            }
        }
        PinMenuAction::Close => {
            DestroyWindow(pin_hwnd);
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
        null_mut(),
        center_x - size.0 / 2,
        center_y - size.1 / 2,
        size.0,
        size.1,
        SWP_NOACTIVATE,
    );
}

pub(super) fn pin_image_rgba(state: &mut UiState, width: usize, height: usize, pixels: Vec<u8>) {
    if width == 0 || height == 0 || pixels.len() < width * height * 4 {
        set_error(state, "无法创建贴图：图像数据无效".into());
        return;
    }
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
            thumbnail_mode: false,
            topmost: true,
            show_border: true,
        });
        SetLayeredWindowAttributes(window, 0, 255, LWA_ALPHA);
        ShowWindow(window, SW_SHOW);
        if let Ok(mut status) = state.status.lock() {
            status.pinned = true;
            status.error = None;
        }
    }
}

pub(super) fn pin_last(state: &mut UiState) {
    let Some((width, height, pixels)) = state.last_image.clone() else {
        set_error(state, "没有可贴图的截图".into());
        return;
    };
    pin_image_rgba(state, width, height, pixels);
}

pub(super) fn pin_image_from_file(state: &mut UiState, path: &str) {
    let p = Path::new(path);
    match image::open(p) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            pin_image_rgba(state, w as usize, h as usize, rgba.into_raw());
        }
        Err(e) => {
            set_error(state, format!("加载贴图文件失败：{e}"));
        }
    }
}

/// SnowAir-style Quick Pin: pins clipboard image if present, else renders clipboard text as a floating card,
/// else falls back to pinning the last captured screenshot.
pub(super) fn quick_pin(state: &mut UiState) {
    if let Ok(mut cb) = Clipboard::new() {
        if let Ok(img) = cb.get_image() {
            if img.width > 0 && img.height > 0 {
                pin_image_rgba(state, img.width, img.height, img.bytes.into_owned());
                return;
            }
        }
        if let Ok(text) = cb.get_text() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                let dark = state.current_theme().is_dark;
                let (w, h, px) = render_text_pin_card(trimmed, dark);
                pin_image_rgba(state, w, h, px);
                return;
            }
        }
    }
    pin_last(state);
}
