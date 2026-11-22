use super::toolbar_paint::{argb, create_ui_font, draw_round_rect_gdip, ensure_gdiplus};
use super::{set_error, wide, UiState};
use crate::model::{ScreenPoint, ScreenRect, RECORDING_FPS_OPTIONS};
use crate::recorder::{AnimationFormat, GifExportQuality, WebpExportSize};
use crate::theme::ThemePalette;
use crate::toolbar::Tool;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateRoundRectRgn,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, InvalidateRect,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, HGDIOBJ, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    DashCapRound, GdipCreateFromHDC, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush,
    GdipDeleteGraphics, GdipDeletePen, GdipDrawArc, GdipDrawLine, GdipFillEllipse,
    GdipSetPenLineCap197819, GdipSetPenLineJoin, GdipSetPixelOffsetMode, GdipSetSmoothingMode,
    GpBrush, GpGraphics, GpPen, GpSolidFill, LineCapRound, LineJoinRound, PixelOffsetModeHalf,
    SmoothingModeAntiAlias8x8, UnitPixel,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::MK_LBUTTON;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_EXPLORER, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDCBF_CANCEL_BUTTON, TDCBF_OK_BUTTON,
    TDF_ALLOW_DIALOG_CANCELLATION, WM_MOUSELEAVE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, GetCursorPos, GetWindowRect, KillTimer,
    MessageBoxW, SetTimer, SetWindowPos, ShowWindow, IDOK, MB_ICONERROR, MB_OK, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE, WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_TIMER,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const CONTROL_TIMER: usize = 0x5243;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecAction {
    Pause,
    ToggleAnnotate,
    ToolFadePen,
    ToolPen,
    ToolRect,
    ToolArrow,
    Undo,
    Clear,
    Stop,
    Cancel,
    ClearPreview,
    OpenPreview,
}

#[derive(Clone, Copy)]
struct RecButton {
    id: usize,
    action: RecAction,
    rect: (i32, i32, i32, i32),
}

struct RecBarMetrics {
    scale: f32,
    width: i32,
    height: i32,
    expanded: bool,
    buttons: Vec<RecButton>,
    dividers_x: Vec<i32>,
}

fn bar_scale(state: &UiState) -> f32 {
    state
        .settings
        .lock()
        .map(|s| crate::theme::ui_scale_for_settings(&s))
        .unwrap_or_else(|_| crate::theme::system_dpi_scale())
}

fn compute_metrics(state: &UiState, phase: &str) -> RecBarMetrics {
    let s = bar_scale(state);
    let px = |dp: f32| (dp * s).round() as i32;
    let height = px(52.0);
    let expanded = phase == "recording" && !state.recording_passthrough;
    let mut buttons = Vec::new();
    let mut dividers_x = Vec::new();
    let by0 = px(8.0);
    let by1 = height - px(8.0);

    let width = match phase {
        "recording" => {
            dividers_x.push(px(116.0));
            buttons.push(RecButton {
                id: 0,
                action: RecAction::Pause,
                rect: (px(124.0), by0, px(162.0), by1),
            });
            buttons.push(RecButton {
                id: 1,
                action: RecAction::ToggleAnnotate,
                rect: (px(168.0), by0, px(248.0), by1),
            });
            if expanded {
                dividers_x.push(px(256.0));
                let tools = [
                    (2, RecAction::ToolFadePen),
                    (3, RecAction::ToolPen),
                    (4, RecAction::ToolRect),
                    (5, RecAction::ToolArrow),
                    (6, RecAction::Undo),
                    (7, RecAction::Clear),
                ];
                let mut x = 264.0f32;
                for (id, action) in tools {
                    buttons.push(RecButton {
                        id,
                        action,
                        rect: (px(x), by0, px(x + 36.0), by1),
                    });
                    x += 40.0;
                }
                dividers_x.push(px(x + 2.0));
                let sx = x + 10.0;
                buttons.push(RecButton {
                    id: 8,
                    action: RecAction::Stop,
                    rect: (px(sx), by0, px(sx + 84.0), by1),
                });
                buttons.push(RecButton {
                    id: 9,
                    action: RecAction::Cancel,
                    rect: (px(sx + 90.0), by0, px(sx + 124.0), by1),
                });
                px(sx + 134.0)
            } else {
                dividers_x.push(px(256.0));
                buttons.push(RecButton {
                    id: 8,
                    action: RecAction::Stop,
                    rect: (px(264.0), by0, px(348.0), by1),
                });
                buttons.push(RecButton {
                    id: 9,
                    action: RecAction::Cancel,
                    rect: (px(354.0), by0, px(388.0), by1),
                });
                px(398.0)
            }
        }
        "preview" => {
            buttons.push(RecButton {
                id: 10,
                action: RecAction::ClearPreview,
                rect: (px(254.0), by0, px(318.0), by1),
            });
            buttons.push(RecButton {
                id: 11,
                action: RecAction::OpenPreview,
                rect: (px(326.0), by0, px(444.0), by1),
            });
            px(456.0)
        }
        "exporting" | "cancelling" => {
            buttons.push(RecButton {
                id: 9,
                action: RecAction::Cancel,
                rect: (px(368.0), by0, px(444.0), by1),
            });
            px(456.0)
        }
        "processing" => px(320.0),
        _ => {
            buttons.push(RecButton {
                id: 9,
                action: RecAction::Cancel,
                rect: (px(368.0), by0, px(444.0), by1),
            });
            px(456.0)
        }
    };

    RecBarMetrics {
        scale: s,
        width,
        height,
        expanded,
        buttons,
        dividers_x,
    }
}

unsafe fn apply_round_window_region(hwnd: HWND, width: i32, height: i32, scale: f32) {
    let r = (24.0 * scale).round() as i32;
    let rgn = CreateRoundRectRgn(0, 0, width + 1, height + 1, r, r);
    if !rgn.is_null() && SetWindowRgn(hwnd, rgn, 1) == 0 {
        DeleteObject(rgn as HGDIOBJ);
    }
}

pub(super) fn create_control_toolbar(
    state: &mut UiState,
    selection: ScreenRect,
    screen: ScreenRect,
) -> Result<(), String> {
    destroy_control_toolbar(state);
    state.recording_error_dialog_message = None;
    state.recording_toolbar_hover = None;
    state.recording_toolbar_tracking = false;
    let status = state.recording.snapshot();
    let metrics = compute_metrics(state, status.phase.as_str());
    let (x, y) = control_position(selection, screen, metrics.width, metrics.height, metrics.scale);
    let class_name = wide("AgileTilesScreenshotCoreV2");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            x,
            y,
            metrics.width,
            metrics.height,
            null_mut(),
            null_mut(),
            GetModuleHandleW(std::ptr::null()),
            state as *mut UiState as *const c_void,
        )
    };
    if hwnd.is_null() {
        return Err("无法创建录制工具栏".into());
    }
    state.recording_toolbar = hwnd;
    state.recording_toolbar_signature = None;
    state.recording_toolbar_static_signature = None;
    unsafe {
        apply_round_window_region(hwnd, metrics.width, metrics.height, metrics.scale);
        SetTimer(hwnd, CONTROL_TIMER, 250, None);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(())
}

/// Smoothly resize and recenter the unified recording bar when toggling inline annotation tools!
pub(crate) fn sync_toolbar_layout(state: &mut UiState) {
    let hwnd = state.recording_toolbar;
    if hwnd.is_null() {
        return;
    }
    let status = state.recording.snapshot();
    let metrics = compute_metrics(state, status.phase.as_str());
    unsafe {
        let mut rc: RECT = zeroed();
        if GetWindowRect(hwnd, &mut rc) != 0 {
            let old_w = rc.right - rc.left;
            let old_h = rc.bottom - rc.top;
            if old_w != metrics.width || old_h != metrics.height {
                let cx = (rc.left + rc.right) / 2;
                let mut nx = cx - metrics.width / 2;
                let mut ny = rc.top;
                if let Some(screen) = state.screen.as_ref().map(super::ScreenCapture::bounds) {
                    nx = nx.clamp(
                        screen.left + 8,
                        (screen.right - metrics.width - 8).max(screen.left + 8),
                    );
                    ny = ny.clamp(
                        screen.top + 8,
                        (screen.bottom - metrics.height - 8).max(screen.top + 8),
                    );
                }
                SetWindowPos(
                    hwnd,
                    (-1isize) as HWND,
                    nx,
                    ny,
                    metrics.width,
                    metrics.height,
                    SWP_NOACTIVATE,
                );
                apply_round_window_region(hwnd, metrics.width, metrics.height, metrics.scale);
            }
        }
        state.recording_toolbar_hover = None;
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

pub(super) fn destroy_control_toolbar(state: &mut UiState) {
    if state.recording_toolbar.is_null() {
        return;
    }
    unsafe {
        KillTimer(state.recording_toolbar, CONTROL_TIMER);
        DestroyWindow(state.recording_toolbar);
    }
    state.recording_toolbar = null_mut();
    state.recording_toolbar_drag = None;
    state.recording_toolbar_hover = None;
    state.recording_toolbar_tracking = false;
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
                != Some(recording_toolbar_static_signature(state, &status).as_str())
            {
                sync_toolbar_layout(state);
            } else if state.recording_toolbar_signature.as_deref()
                != Some(recording_toolbar_signature(state, &status).as_str())
            {
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let pt = (
                (lparam & 0xffff) as i16 as i32,
                ((lparam >> 16) & 0xffff) as i16 as i32,
            );
            let status = state.recording.snapshot();
            let metrics = compute_metrics(state, status.phase.as_str());
            if let Some(btn) = metrics.buttons.iter().find(|b| inside(pt, b.rect)) {
                handle_control_action(state, btn.action);
            } else {
                // Drag smoothly from anywhere on the bar background / grip / timer!
                let mut cursor: POINT = zeroed();
                let mut rect: RECT = zeroed();
                if GetCursorPos(&mut cursor) != 0 && GetWindowRect(hwnd, &mut rect) != 0 {
                    state.recording_toolbar_drag = Some((
                        ScreenPoint { x: cursor.x, y: cursor.y },
                        ScreenPoint { x: rect.left, y: rect.top },
                    ));
                    SetCapture(hwnd);
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            if wparam & MK_LBUTTON as usize != 0 {
                if let Some((start, origin)) = state.recording_toolbar_drag {
                    let mut cursor: POINT = zeroed();
                    let mut rc: RECT = zeroed();
                    if GetCursorPos(&mut cursor) != 0 && GetWindowRect(hwnd, &mut rc) != 0 {
                        let w = rc.right - rc.left;
                        let h = rc.bottom - rc.top;
                        let mut x = origin.x + cursor.x - start.x;
                        let mut y = origin.y + cursor.y - start.y;
                        if let Some(screen) = state.screen.as_ref().map(super::ScreenCapture::bounds)
                        {
                            x = x.clamp(screen.left + 4, (screen.right - w - 4).max(screen.left + 4));
                            y = y.clamp(screen.top + 4, (screen.bottom - h - 4).max(screen.top + 4));
                        }
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            x,
                            y,
                            0,
                            0,
                            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
                        );
                    }
                    return 0;
                }
            }
            if !state.recording_toolbar_tracking {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                if TrackMouseEvent(&mut tme) != 0 {
                    state.recording_toolbar_tracking = true;
                }
            }
            let pt = (
                (lparam & 0xffff) as i16 as i32,
                ((lparam >> 16) & 0xffff) as i16 as i32,
            );
            let status = state.recording.snapshot();
            let metrics = compute_metrics(state, status.phase.as_str());
            let next_hover = metrics
                .buttons
                .iter()
                .find(|b| inside(pt, b.rect))
                .map(|b| b.id);
            if state.recording_toolbar_hover != next_hover {
                state.recording_toolbar_hover = next_hover;
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_MOUSELEAVE => {
            state.recording_toolbar_tracking = false;
            if state.recording_toolbar_hover.take().is_some() {
                InvalidateRect(hwnd, std::ptr::null(), 0);
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
            super::demo::stop_demo(state);
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

fn control_position(
    selection: ScreenRect,
    screen: ScreenRect,
    width: i32,
    height: i32,
    scale: f32,
) -> (i32, i32) {
    let margin = (10.0 * scale).round() as i32;
    let selection_width = selection.right - selection.left;
    let desired_x = selection.left + (selection_width - width) / 2;
    let x = desired_x.clamp(
        screen.left + margin,
        (screen.right - width - margin).max(screen.left + margin),
    );
    let above = selection.top - height - margin;
    let below = selection.bottom + margin;
    let y = if above >= screen.top + margin {
        above
    } else if below + height <= screen.bottom - margin {
        below
    } else {
        (screen.top + margin).min(screen.bottom - height - margin)
    };
    (x, y)
}

unsafe fn paint_control_toolbar(hwnd: HWND, state: &mut UiState) {
    let mut paint: PAINTSTRUCT = zeroed();
    let target_dc = BeginPaint(hwnd, &mut paint);
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    let w = (client.right - client.left).max(1);
    let h = (client.bottom - client.top).max(1);

    let mem_dc = CreateCompatibleDC(target_dc);
    let bmp = CreateCompatibleBitmap(target_dc, w, h);
    let use_buf = !mem_dc.is_null() && !bmp.is_null();
    let old_bmp = if use_buf {
        SelectObject(mem_dc, bmp as HGDIOBJ)
    } else {
        null_mut()
    };
    let dc = if use_buf { mem_dc } else { target_dc };

    paint_control_toolbar_to_dc(dc, w, h, state);

    if use_buf {
        BitBlt(target_dc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
        SelectObject(mem_dc, old_bmp);
        DeleteObject(bmp as HGDIOBJ);
        DeleteDC(mem_dc);
    } else {
        if !bmp.is_null() {
            DeleteObject(bmp as HGDIOBJ);
        }
        if !mem_dc.is_null() {
            DeleteDC(mem_dc);
        }
    }
    EndPaint(hwnd, &paint);
}

unsafe fn paint_control_toolbar_to_dc(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    w: i32,
    h: i32,
    state: &mut UiState,
) {
    let theme = state.current_theme();
    let status = state.recording.snapshot();
    state.recording_toolbar_signature = Some(recording_toolbar_signature(state, &status));
    state.recording_toolbar_static_signature =
        Some(recording_toolbar_static_signature(state, &status));
    let metrics = compute_metrics(state, status.phase.as_str());
    let s = metrics.scale;
    let px = |dp: f32| (dp * s).round() as i32;

    // Solid bg fill first so corners match window rgn cleanly
    let bg_br = CreateSolidBrush(ThemePalette::colorref(theme.bg));
    let full = RECT { left: 0, top: 0, right: w, bottom: h };
    FillRect(dc, &full, bg_br);
    DeleteObject(bg_br as _);

    ensure_gdiplus();
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(dc, &mut g) == 0 && !g.is_null() {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);

        // Crisp 1px rounded inner border
        draw_round_rect_gdip(
            g,
            0.5,
            0.5,
            w as f32 - 1.0,
            h as f32 - 1.0,
            12.0 * s,
            Some(argb(theme.bg, Some(0xFF))),
            Some((argb(theme.icon, Some(0x2E)), (1.0 * s).max(1.0))),
        );

        // 6-dot drag grip
        let mut grip_br: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(argb(theme.icon, Some(0x70)), &mut grip_br) == 0 && !grip_br.is_null()
        {
            let r = 1.45 * s;
            for row in [-5.5f32, 0.0, 5.5] {
                for col in [-2.6f32, 2.6] {
                    let gx = 18.0 * s + col * s;
                    let gy = h as f32 * 0.5 + row * s;
                    GdipFillEllipse(g, grip_br as *mut GpBrush, gx - r, gy - r, r * 2.0, r * 2.0);
                }
            }
            GdipDeleteBrush(grip_br as *mut GpBrush);
        }

        // Group dividers
        let mut div_pen: *mut GpPen = null_mut();
        if GdipCreatePen1(argb(theme.icon, Some(0x28)), (1.0 * s).max(1.0), UnitPixel, &mut div_pen)
            == 0
            && !div_pen.is_null()
        {
            for &dx in &metrics.dividers_x {
                GdipDrawLine(
                    g,
                    div_pen,
                    dx as f32,
                    h as f32 * 0.5 - 11.0 * s,
                    dx as f32,
                    h as f32 * 0.5 + 11.0 * s,
                );
            }
            GdipDeletePen(div_pen);
        }

        let icon_col = argb(theme.icon, Some(0xFF));
        let accent_col = argb(theme.accent, Some(0xFF));
        let danger_col = argb(theme.danger, Some(0xFF));
        let hover_col = argb(theme.hover, Some(0xFF));
        let active_bg = argb(theme.accent, Some(if theme.is_dark { 0x2A } else { 0x20 }));
        let active_tool = super::demo::recording_canvas_tool(state);
        let has_ann = super::demo::recording_canvas_has_annotations(state);

        if status.phase == "recording" {
            // Recording status pulse dot
            let dot_col = if status.paused {
                0xFF_F59E0B // Amber when paused
            } else {
                danger_col // Red live dot
            };
            let mut dot_br: *mut GpSolidFill = null_mut();
            if GdipCreateSolidFill(dot_col, &mut dot_br) == 0 && !dot_br.is_null() {
                let dr = 4.6 * s;
                let dx = 37.0 * s;
                let dy = h as f32 * 0.5;
                GdipFillEllipse(g, dot_br as *mut GpBrush, dx - dr, dy - dr, dr * 2.0, dr * 2.0);
                GdipDeleteBrush(dot_br as *mut GpBrush);
            }
        } else if status.phase == "exporting" {
            // SnowAir green progress bar
            let pl = 36.0 * s;
            let pr = 354.0 * s;
            let py = h as f32 - 12.0 * s;
            let ph = 5.0 * s;
            draw_round_rect_gdip(
                g,
                pl,
                py,
                pr - pl,
                ph,
                ph * 0.5,
                Some(argb(theme.slider_track, Some(0xFF))),
                None,
            );
            let fill_w = (pr - pl) * (f32::from(status.progress_percent.min(100)) / 100.0);
            if fill_w > 2.0 {
                draw_round_rect_gdip(g, pl, py, fill_w, ph, ph * 0.5, Some(accent_col), None);
            }
        }

        // Draw all buttons (background + crisp GDI+ icons)
        for btn in &metrics.buttons {
            let hovered = state.recording_toolbar_hover == Some(btn.id);
            let (l, t, r, b) = btn.rect;
            let bw = (r - l) as f32;
            let bh = (b - t) as f32;
            let cx = (l + r) as f32 * 0.5;
            let cy = (t + b) as f32 * 0.5;

            let is_selected_tool = match btn.action {
                RecAction::ToolFadePen => active_tool == Tool::FadePen,
                RecAction::ToolPen => active_tool == Tool::Pen,
                RecAction::ToolRect => active_tool == Tool::Rectangle,
                RecAction::ToolArrow => active_tool == Tool::Arrow,
                _ => false,
            };

            match btn.action {
                RecAction::Stop => {
                    let fill = if hovered { 0xFF_DC2626 } else { danger_col };
                    draw_round_rect_gdip(g, l as f32, t as f32, bw, bh, 9.0 * s, Some(fill), None);
                }
                RecAction::OpenPreview => {
                    draw_round_rect_gdip(
                        g,
                        l as f32,
                        t as f32,
                        bw,
                        bh,
                        9.0 * s,
                        Some(accent_col),
                        None,
                    );
                }
                RecAction::ToggleAnnotate => {
                    let annotating = !state.recording_passthrough;
                    let fill = if annotating {
                        accent_col
                    } else if hovered {
                        hover_col
                    } else {
                        argb(theme.hover, Some(0xAA))
                    };
                    draw_round_rect_gdip(
                        g,
                        l as f32,
                        t as f32,
                        bw,
                        bh,
                        9.0 * s,
                        Some(fill),
                        (!annotating).then_some((argb(theme.icon, Some(0x22)), 1.0)),
                    );
                }
                _ => {
                    if is_selected_tool {
                        draw_round_rect_gdip(
                            g,
                            l as f32,
                            t as f32,
                            bw,
                            bh,
                            8.0 * s,
                            Some(active_bg),
                            None,
                        );
                    } else if hovered {
                        draw_round_rect_gdip(
                            g,
                            l as f32,
                            t as f32,
                            bw,
                            bh,
                            8.0 * s,
                            Some(hover_col),
                            None,
                        );
                    }
                }
            }

            // Vector icons for icon-only buttons
            let stroke_col = match btn.action {
                RecAction::Undo | RecAction::Clear if !has_ann => argb(theme.icon_disabled, Some(0x88)),
                RecAction::Cancel if hovered => danger_col,
                _ if is_selected_tool => accent_col,
                _ => icon_col,
            };
            draw_rec_button_icon(g, btn.action, cx, cy, s, stroke_col, status.paused);
        }

        GdipDeleteGraphics(g);
    }

    // Crisp ClearType Microsoft YaHei UI text labels
    let text_c = ThemePalette::colorref(theme.info_text);
    let muted_c = ThemePalette::colorref(theme.info_muted);
    match status.phase.as_str() {
        "recording" => {
            draw_rec_text(
                dc,
                px(47.0),
                0,
                px(114.0),
                h,
                &format_elapsed(status.elapsed_ms),
                if status.paused { muted_c } else { text_c },
                14.5,
                s,
                true,
                false,
            );
            for btn in &metrics.buttons {
                match btn.action {
                    RecAction::ToggleAnnotate => {
                        let annotating = !state.recording_passthrough;
                        draw_rec_text(
                            dc,
                            btn.rect.0,
                            btn.rect.1,
                            btn.rect.2,
                            btn.rect.3,
                            if annotating { "↖ 操作" } else { "✎ 标注" },
                            if annotating { 0x00FF_FFFF } else { text_c },
                            12.5,
                            s,
                            true,
                            true,
                        );
                    }
                    RecAction::Stop => {
                        draw_rec_text(
                            dc,
                            btn.rect.0,
                            btn.rect.1,
                            btn.rect.2,
                            btn.rect.3,
                            "■ 结束",
                            0x00FF_FFFF,
                            12.5,
                            s,
                            true,
                            true,
                        );
                    }
                    _ => {}
                }
            }
        }
        "processing" => {
            draw_rec_text(dc, px(36.0), 0, w - px(16.0), h, "正在完成录制…", text_c, 13.0, s, true, false);
        }
        "exporting" | "cancelling" => {
            let label = control_status_text(&status);
            draw_rec_text(dc, px(36.0), px(4.0), px(356.0), px(34.0), &label, text_c, 12.5, s, true, false);
            if let Some(btn) = metrics.buttons.first() {
                draw_rec_text(dc, btn.rect.0, btn.rect.1, btn.rect.2, btn.rect.3, "取消", text_c, 12.5, s, true, true);
            }
        }
        "preview" => {
            let label = control_status_text(&status);
            draw_rec_text(dc, px(36.0), 0, px(246.0), h, &label, text_c, 12.5, s, true, false);
            for btn in &metrics.buttons {
                let (lbl, col) = match btn.action {
                    RecAction::ClearPreview => ("丢弃", text_c),
                    RecAction::OpenPreview => ("预览与导出", 0x00FF_FFFF),
                    _ => continue,
                };
                draw_rec_text(dc, btn.rect.0, btn.rect.1, btn.rect.2, btn.rect.3, lbl, col, 12.5, s, true, true);
            }
        }
        _ => {
            let label = control_status_text(&status);
            draw_rec_text(dc, px(36.0), 0, px(358.0), h, &label, text_c, 12.5, s, false, false);
            if let Some(btn) = metrics.buttons.first() {
                draw_rec_text(dc, btn.rect.0, btn.rect.1, btn.rect.2, btn.rect.3, "返回", text_c, 12.5, s, true, true);
            }
        }
    }
}

unsafe fn draw_rec_button_icon(
    g: *mut GpGraphics,
    action: RecAction,
    cx: f32,
    cy: f32,
    s: f32,
    color: u32,
    paused: bool,
) {
    let mut pen: *mut GpPen = null_mut();
    let mut fill: *mut GpSolidFill = null_mut();
    let sw = (1.8 * s).max(1.4);
    GdipCreatePen1(color, sw, UnitPixel, &mut pen);
    GdipCreateSolidFill(color, &mut fill);
    if !pen.is_null() {
        GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapRound);
        GdipSetPenLineJoin(pen, LineJoinRound);
    }
    let line = |x1: f32, y1: f32, x2: f32, y2: f32| {
        GdipDrawLine(g, pen, cx + x1 * s, cy + y1 * s, cx + x2 * s, cy + y2 * s);
    };

    match action {
        RecAction::Pause => {
            if paused {
                line(-3.5, -5.5, 5.5, 0.0);
                line(5.5, 0.0, -3.5, 5.5);
                line(-3.5, 5.5, -3.5, -5.5);
            } else {
                line(-3.2, -5.5, -3.2, 5.5);
                line(3.2, -5.5, 3.2, 5.5);
            }
        }
        RecAction::ToolFadePen => {
            line(1.5, -7.2, -5.0, 0.8);
            line(-5.0, 0.8, 0.5, 0.8);
            line(0.5, 0.8, -1.5, 7.2);
            line(-1.5, 7.2, 5.0, -0.8);
            line(5.0, -0.8, -0.5, -0.8);
            line(-0.5, -0.8, 1.5, -7.2);
        }
        RecAction::ToolPen => {
            line(-6.0, 6.2, -4.0, 1.6);
            line(-4.0, 1.6, 3.8, -6.2);
            line(3.8, -6.2, 6.4, -3.6);
            line(6.4, -3.6, -1.4, 4.2);
            line(-1.4, 4.2, -6.0, 6.2);
        }
        RecAction::ToolRect => {
            draw_round_rect_gdip(
                g,
                cx - 6.8 * s,
                cy - 5.6 * s,
                13.6 * s,
                11.2 * s,
                2.4 * s,
                None,
                Some((color, sw)),
            );
        }
        RecAction::ToolArrow => {
            line(-5.8, 5.8, 5.8, -5.8);
            line(0.8, -5.8, 5.8, -5.8);
            line(5.8, -5.8, 5.8, -0.8);
        }
        RecAction::Undo => {
            line(-6.0, -2.2, -2.4, -5.8);
            line(-6.0, -2.2, -2.4, 1.4);
            line(-6.0, -2.2, 2.0, -2.2);
            GdipDrawArc(g, pen, cx - 2.0 * s, cy - 2.2 * s, 8.4 * s, 8.4 * s, -90.0, 180.0);
            line(-2.0, 6.2, 2.2, 6.2);
        }
        RecAction::Clear => {
            // Trash / clear bin icon
            line(-6.0, -4.2, 6.0, -4.2);
            line(-2.2, -6.8, 2.2, -6.8);
            line(-4.6, -4.2, -3.8, 6.4);
            line(4.6, -4.2, 3.8, 6.4);
            line(-3.8, 6.4, 3.8, 6.4);
            line(-1.6, -1.2, -1.6, 3.6);
            line(1.6, -1.2, 1.6, 3.6);
        }
        RecAction::Cancel => {
            line(-4.8, -4.8, 4.8, 4.8);
            line(4.8, -4.8, -4.8, 4.8);
        }
        _ => {}
    }

    if !fill.is_null() {
        GdipDeleteBrush(fill as *mut GpBrush);
    }
    if !pen.is_null() {
        GdipDeletePen(pen);
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_rec_text(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    value: &str,
    color: u32,
    dp_size: f32,
    scale: f32,
    medium: bool,
    centered: bool,
) {
    let font = create_ui_font(dp_size, scale, medium);
    let old_font = if !font.is_null() {
        SelectObject(dc, font as HGDIOBJ)
    } else {
        null_mut()
    };
    SetBkMode(dc, 1);
    SetTextColor(dc, color);
    let text = wide(value);
    let mut rect = RECT { left, top, right, bottom };
    DrawTextW(
        dc,
        text.as_ptr(),
        -1,
        &mut rect,
        if centered { 1 | 4 | 0x20 } else { 4 | 0x20 },
    );
    if !old_font.is_null() {
        SelectObject(dc, old_font);
    }
    if !font.is_null() {
        DeleteObject(font as HGDIOBJ);
    }
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
            _ => format!("录制完成 · {} 帧", status.frame_count),
        },
        "error" => status.message.clone().unwrap_or_else(|| "录制失败".into()),
        "cancelled" => "录制已取消".into(),
        _ => "录制工具".into(),
    }
}

fn recording_toolbar_signature(
    state: &UiState,
    status: &crate::recorder::RecordingStatus,
) -> String {
    let recording = status.phase == "recording";
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        status.phase,
        status.paused,
        state.recording_passthrough,
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

fn recording_toolbar_static_signature(
    state: &UiState,
    status: &crate::recorder::RecordingStatus,
) -> String {
    let frame_count = if status.phase == "recording" {
        0
    } else {
        status.frame_count
    };
    format!(
        "{}|{}|{}|{}|{}",
        status.phase,
        status.paused,
        state.recording_passthrough,
        frame_count,
        status.message.as_deref().unwrap_or_default()
    )
}

fn format_elapsed(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn handle_control_action(state: &mut UiState, action: RecAction) {
    match action {
        RecAction::Pause => {
            if let Err(error) = state.recording.toggle_pause() {
                set_error(state, error);
            }
            if !state.recording_toolbar.is_null() {
                unsafe { InvalidateRect(state.recording_toolbar, std::ptr::null(), 0) };
            }
        }
        RecAction::ToggleAnnotate => {
            let next_pt = !state.recording_passthrough;
            super::demo::set_recording_canvas_passthrough(state, next_pt);
            sync_toolbar_layout(state);
        }
        RecAction::ToolFadePen => {
            super::demo::set_recording_tool(state, Tool::FadePen);
            if !state.recording_toolbar.is_null() {
                unsafe { InvalidateRect(state.recording_toolbar, std::ptr::null(), 0) };
            }
        }
        RecAction::ToolPen => {
            super::demo::set_recording_tool(state, Tool::Pen);
            if !state.recording_toolbar.is_null() {
                unsafe { InvalidateRect(state.recording_toolbar, std::ptr::null(), 0) };
            }
        }
        RecAction::ToolRect => {
            super::demo::set_recording_tool(state, Tool::Rectangle);
            if !state.recording_toolbar.is_null() {
                unsafe { InvalidateRect(state.recording_toolbar, std::ptr::null(), 0) };
            }
        }
        RecAction::ToolArrow => {
            super::demo::set_recording_tool(state, Tool::Arrow);
            if !state.recording_toolbar.is_null() {
                unsafe { InvalidateRect(state.recording_toolbar, std::ptr::null(), 0) };
            }
        }
        RecAction::Undo => {
            super::demo::undo_recording_annotation(state);
            if !state.recording_toolbar.is_null() {
                unsafe { InvalidateRect(state.recording_toolbar, std::ptr::null(), 0) };
            }
        }
        RecAction::Clear => {
            super::demo::clear_recording_annotations(state);
            if !state.recording_toolbar.is_null() {
                unsafe { InvalidateRect(state.recording_toolbar, std::ptr::null(), 0) };
            }
        }
        RecAction::Stop => {
            super::stop_recording(state);
        }
        RecAction::Cancel => {
            let phase = state.recording.snapshot().phase;
            super::demo::stop_demo(state);
            if phase == "recording" || phase == "exporting" {
                state.recording.cancel();
                destroy_control_toolbar(state);
                super::cancel(state);
            } else {
                destroy_control_toolbar(state);
                super::restore_recording_selection(state);
            }
        }
        RecAction::OpenPreview => {
            super::queue_open_recording_editor(state);
        }
        RecAction::ClearPreview => {
            state.recording.cancel();
            destroy_control_toolbar(state);
            super::cancel(state);
        }
    }
}

fn inside(point: (i32, i32), rect: (i32, i32, i32, i32)) -> bool {
    point.0 >= rect.0 && point.0 < rect.2 && point.1 >= rect.1 && point.1 < rect.3
}

pub(super) fn export_recording_direct(
    state: &mut UiState,
    format: AnimationFormat,
    gif_export_quality: GifExportQuality,
    webp_export_size: WebpExportSize,
    output_fps: u32,
) {
    state.recording_auto_open_pending = false;
    state.recording_error_dialog_message = None;
    let initial_dir = state
        .settings
        .lock()
        .map(|settings| settings.resolved_save_directory().to_string_lossy().into_owned())
        .unwrap_or_default();
    let owner = if state.editor.is_some() {
        state.overlay
    } else if !state.recording_toolbar.is_null() {
        state.recording_toolbar
    } else {
        state.hwnd
    };
    state.recording_export_dialog_open = true;
    let path = match save_animation_path_only(owner, format, &initial_dir) {
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
    let Some(path) = path else {
        return;
    };
    let source_fps = state.recording.snapshot().fps.max(1);
    let effective_fps = if format == AnimationFormat::Mp4 {
        source_fps
    } else {
        output_fps.min(source_fps).max(1)
    };
    start_recording_export(
        state,
        format,
        gif_export_quality,
        webp_export_size,
        path,
        effective_fps,
    );
}

pub(super) fn reveal_exported_file(path: &str) {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return;
    }
    let _ = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{trimmed}"))
        .spawn();
}

pub(super) fn export_recording(state: &mut UiState, format: AnimationFormat) {
    // Export completion returns to preview; it must not trigger the one-time
    // editor transition intended for the end of a new recording.
    state.recording_auto_open_pending = false;
    state.recording_error_dialog_message = None;
    let initial_dir = state
        .settings
        .lock()
        .map(|settings| settings.resolved_save_directory().to_string_lossy().into_owned())
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
    start_recording_export(
        state,
        format,
        gif_export_quality,
        webp_export_size,
        path,
        output_fps,
    );
}

fn start_recording_export(
    state: &mut UiState,
    format: AnimationFormat,
    gif_export_quality: GifExportQuality,
    webp_export_size: WebpExportSize,
    path: PathBuf,
    output_fps: u32,
) {
    let (_, recording_bitrate_mbps) =
        super::configured_recording_options(&state.settings);
    let (gif_colors, webp_quality) =
        super::configured_animation_export_options(&state.settings);
    let exported_path_str = path.to_string_lossy().into_owned();
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
    } else {
        crate::history::record_item("recording", &exported_path_str, 0, 0);
        if state.editor.is_some() {
            if let Some(editor) = state.editor.as_mut() {
                editor.playing = false;
            }
            unsafe {
                KillTimer(state.overlay, super::editor::TIMER_RECORDING_PREVIEW);
                SetTimer(
                    state.overlay,
                    super::editor::TIMER_RECORDING_EXPORT_STATUS,
                    200,
                    None,
                );
                InvalidateRect(state.overlay, std::ptr::null(), 0);
            }
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

fn default_recording_filename(format: AnimationFormat) -> String {
    let now = chrono::Local::now();
    format!(
        "SideWeave_{}.{}",
        now.format("%Y%m%d_%H%M%S"),
        format.extension()
    )
}

fn save_animation_path_only(
    owner: windows_sys::Win32::Foundation::HWND,
    format: AnimationFormat,
    initial_dir: &str,
) -> Result<Option<PathBuf>, String> {
    let (title, filter) = match format {
        AnimationFormat::Webp => ("导出 WebP 动图", wide("WebP 动图 (*.webp)\0*.webp\0")),
        AnimationFormat::Gif => ("导出 GIF 动图", wide("GIF 动图 (*.gif)\0*.gif\0")),
        AnimationFormat::Mp4 => ("导出 MP4 视频", wide("MP4 视频 (*.mp4)\0*.mp4\0")),
    };
    let default_name = default_recording_filename(format);
    let title = wide(title);
    let extension = wide(format.extension());
    let initial_dir = (!initial_dir.trim().is_empty()).then(|| wide(initial_dir));
    let initial_dir_ptr = initial_dir.as_ref().map(|value| value.as_ptr());
    let mut file = vec![0u16; 512];
    let default_name_w = wide(&default_name);
    let copy_len = default_name_w.len().min(file.len() - 1);
    file[..copy_len].copy_from_slice(&default_name_w[..copy_len]);
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
    Ok(Some(path))
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
