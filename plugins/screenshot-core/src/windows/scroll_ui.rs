use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, InvalidateRect, RoundRect, SelectObject,
    SetBkMode, SetTextColor, HGDIOBJ, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowRect,
    SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, ShowWindow, LWA_ALPHA, LWA_COLORKEY,
    SW_HIDE, SW_SHOWNOACTIVATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::capture::ScreenCapture;
use crate::model::ScreenRect;
use crate::scroll::{CapturePhase, FrameResult, MatchAxis, ScrollingCapture};
use crate::scroll_input::{ScrollInputDelta, ScrollInputListener};
use crate::toolbar::{
    button_origin as toolbar_button_origin, hit_test as toolbar_hit_test, ToolbarAction,
    ToolbarMenu, TOOLBAR_BUTTON_HEIGHT, TOOLBAR_BUTTON_WIDTH, TOOLBAR_ITEMS,
};

use super::draw::draw_rect_border;
use super::editor::open_editor;
use super::input::{open_toolbar_menu, VK_ESCAPE_CODE, VK_RETURN_CODE};
use super::output::{copy_capture, finish, finish_with_dialog, pin_last};
use super::paint::draw_rgba_preview;
use super::toolbar_paint::{
    draw_toolbar_gdi, scroll_toolbar_action, scroll_toolbar_layout, scroll_toolbar_position,
    SCROLL_TOOLBAR_ACTIONS,
};
use super::{
    cancel, clear_error, set_error, wide, ScrollPreviewFeedback, UiState, MAX_SCROLL_EXTENT,
    SCROLL_MASK_ALPHA, SCROLL_OBSERVE_INTERVAL, SCROLL_PREVIEW_GAP, SCROLL_PREVIEW_HEIGHT,
    SCROLL_PREVIEW_MARGIN, SCROLL_PREVIEW_WIDTH, SCROLL_TIP_HEIGHT, SCROLL_TIP_MARGIN,
    SCROLL_TIP_WIDTH, TIMER_SCROLL,
};

pub(crate) fn selection_fits_capture_bounds(screen: ScreenRect, selection: ScreenRect) -> bool {
    selection.clipped_to(screen) == Some(selection)
}

pub(crate) fn scroll_preview_position(screen: ScreenRect, selection: ScreenRect) -> (i32, i32) {
    let min_x = screen.left + SCROLL_PREVIEW_MARGIN;
    let max_x = (screen.right - SCROLL_PREVIEW_WIDTH - SCROLL_PREVIEW_MARGIN).max(min_x);
    let min_y = screen.top + SCROLL_PREVIEW_MARGIN;
    let max_y = (screen.bottom - SCROLL_PREVIEW_HEIGHT - SCROLL_PREVIEW_MARGIN).max(min_y);
    let attached_x = selection.left.clamp(min_x, max_x);
    let attached_y = selection.top.clamp(min_y, max_y);
    let candidates = [
        (selection.right + SCROLL_PREVIEW_GAP, attached_y),
        (
            selection.left - SCROLL_PREVIEW_WIDTH - SCROLL_PREVIEW_GAP,
            attached_y,
        ),
        (
            attached_x,
            selection.top - SCROLL_PREVIEW_HEIGHT - SCROLL_PREVIEW_GAP,
        ),
        (attached_x, selection.bottom + SCROLL_PREVIEW_GAP),
        (max_x, min_y),
        (min_x, min_y),
        (max_x, max_y),
        (min_x, max_y),
    ];
    candidates
        .into_iter()
        .find(|(x, y)| {
            let right = *x + SCROLL_PREVIEW_WIDTH;
            let bottom = *y + SCROLL_PREVIEW_HEIGHT;
            *x >= min_x
                && *y >= min_y
                && right <= screen.right - SCROLL_PREVIEW_MARGIN
                && bottom <= screen.bottom - SCROLL_PREVIEW_MARGIN
                && (right <= selection.left
                    || *x >= selection.right
                    || bottom <= selection.top
                    || *y >= selection.bottom)
        })
        .unwrap_or((max_x, min_y))
}

pub(super) fn scroll_tip_position(
    screen: ScreenRect,
    selection: ScreenRect,
    preview_position: (i32, i32),
) -> Option<(i32, i32)> {
    let margin = SCROLL_TIP_MARGIN;
    let min_x = screen.left + margin;
    let max_x = (screen.right - SCROLL_TIP_WIDTH - margin).max(min_x);
    let min_y = screen.top + margin;
    let max_y = (screen.bottom - SCROLL_TIP_HEIGHT - margin).max(min_y);
    let centered_x =
        ((selection.left + selection.right - SCROLL_TIP_WIDTH) / 2).clamp(min_x, max_x);
    let centered_y =
        ((selection.top + selection.bottom - SCROLL_TIP_HEIGHT) / 2).clamp(min_y, max_y);
    let candidates = [
        (centered_x, selection.top - SCROLL_TIP_HEIGHT - margin),
        (centered_x, selection.bottom + margin),
        (selection.left - SCROLL_TIP_WIDTH - margin, centered_y),
        (selection.right + margin, centered_y),
        (min_x, min_y),
        (max_x, min_y),
        (min_x, max_y),
        (max_x, max_y),
    ];
    let preview_right = preview_position.0 + SCROLL_PREVIEW_WIDTH;
    let preview_bottom = preview_position.1 + SCROLL_PREVIEW_HEIGHT;

    candidates.into_iter().find(|(x, y)| {
        let right = *x + SCROLL_TIP_WIDTH;
        let bottom = *y + SCROLL_TIP_HEIGHT;
        let within_screen = *x >= min_x
            && *y >= min_y
            && right <= screen.right - margin
            && bottom <= screen.bottom - margin;
        let outside_selection = right <= selection.left
            || *x >= selection.right
            || bottom <= selection.top
            || *y >= selection.bottom;
        let outside_preview = right <= preview_position.0
            || *x >= preview_right
            || bottom <= preview_position.1
            || *y >= preview_bottom;
        within_screen && outside_selection && outside_preview
    })
}

pub(super) fn create_scroll_masks(
    state: &mut UiState,
    screen: ScreenRect,
    selection: ScreenRect,
) {
    let regions = [
        (
            screen.left,
            screen.top,
            screen.width() as i32,
            selection.top - screen.top,
        ),
        (
            screen.left,
            selection.bottom,
            screen.width() as i32,
            screen.bottom - selection.bottom,
        ),
        (
            screen.left,
            selection.top,
            selection.left - screen.left,
            selection.height() as i32,
        ),
        (
            selection.right,
            selection.top,
            screen.right - selection.right,
            selection.height() as i32,
        ),
    ];

    unsafe {
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let instance = GetModuleHandleW(null());
        for (x, y, width, height) in regions {
            if width <= 0 || height <= 0 {
                continue;
            }
            let mask = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_POPUP,
                x,
                y,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                state as *mut UiState as *const c_void,
            );
            if mask.is_null() {
                continue;
            }
            state.scroll_masks.push(mask);
            SetLayeredWindowAttributes(mask, 0, SCROLL_MASK_ALPHA, LWA_ALPHA);
            ShowWindow(mask, SW_SHOWNOACTIVATE);
        }
    }
}

pub(super) fn create_scroll_border(state: &mut UiState, selection: ScreenRect) {
    if !state.scroll_border.is_null() {
        destroy_scroll_border(state);
    }

    unsafe {
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let border = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            selection.left - 4,
            selection.top - 4,
            (selection.width() + 8) as i32,
            (selection.height() + 8) as i32,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            state as *mut UiState as *const c_void,
        );
        if !border.is_null() {
            let _ = SetLayeredWindowAttributes(border, 0, 0, LWA_COLORKEY);
            state.scroll_border = border;
            ShowWindow(border, SW_SHOWNOACTIVATE);
            InvalidateRect(border, null(), 0);
        }
    }
}

pub(super) fn destroy_scroll_preview(state: &mut UiState) {
    if !state.scroll_preview.is_null() {
        unsafe {
            DestroyWindow(state.scroll_preview);
        }
        state.scroll_preview = null_mut();
    }
}

pub(super) fn destroy_scroll_tip(state: &mut UiState) {
    if !state.scroll_tip.is_null() {
        unsafe {
            DestroyWindow(state.scroll_tip);
        }
        state.scroll_tip = null_mut();
    }
    state.scroll_tip_text = None;
}

pub(super) fn destroy_scroll_masks(state: &mut UiState) {
    let masks = std::mem::take(&mut state.scroll_masks);
    for mask in masks {
        unsafe {
            DestroyWindow(mask);
        }
    }
}

pub(super) fn destroy_scroll_border(state: &mut UiState) {
    if !state.scroll_border.is_null() {
        unsafe {
            DestroyWindow(state.scroll_border);
        }
        state.scroll_border = null_mut();
    }
}

pub(super) fn open_scroll_mode_menu(state: &mut UiState) {
    if state
        .capture
        .as_ref()
        .and_then(|capture| capture.selection)
        .is_some()
    {
        open_toolbar_menu(state, ToolbarMenu::ScrollMode);
    }
}

pub(super) fn start_scroll(state: &mut UiState, axis: MatchAxis) {
    let Some(selection) = state.capture.as_ref().and_then(|capture| capture.selection) else {
        return;
    };
    if !state.scroll_control.is_null() {
        return;
    }
    clear_error(state);
    let Some(screen) = state.screen.as_ref() else {
        set_error(state, "屏幕捕获不可用".into());
        return;
    };
    if !selection_fits_capture_bounds(screen.bounds(), selection) {
        set_error(state, "选区超出物理桌面范围".into());
        return;
    };
    let frame = screen.crop(selection);
    let initial_probe = screen.probe(selection);
    let screen_rect = state
        .capture
        .as_ref()
        .map(|capture| capture.screen_rect)
        .unwrap_or(ScreenRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        });
    let Some(capture) = state.capture.as_mut() else {
        return;
    };
    capture.scroll_axis = Some(axis);
    capture.toolbar_hover = None;
    let scroll = capture.scroll.get_or_insert_with(ScrollingCapture::pending);
    scroll.set_axis(axis);
    scroll.seed(selection.width(), selection.height(), frame);
    scroll.seed_probe(initial_probe);
    let layout = scroll_toolbar_layout();
    let (toolbar_x, toolbar_y) = scroll_toolbar_position(selection, screen_rect);

    unsafe {
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let instance = GetModuleHandleW(null());
        create_scroll_masks(state, screen_rect, selection);
        state.scroll_preview_feedback = ScrollPreviewFeedback::Waiting;

        let control = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            toolbar_x,
            toolbar_y,
            layout.width,
            layout.height,
            null_mut(),
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        if control.is_null() {
            destroy_scroll_masks(state);
            set_error(state, "无法创建截图工具栏".into());
            return;
        }
        state.scroll_control = control;
        ShowWindow(control, SW_SHOWNOACTIVATE);
        InvalidateRect(control, null(), 0);

        // The full-screen selection surface is replaced by a lightweight
        // border. Keep extra space around the crop because the GDI pen is
        // centered on the rectangle edge and can otherwise bleed into pixels.
        create_scroll_border(state, selection);

        let (preview_x, preview_y) = scroll_preview_position(screen_rect, selection);
        let preview = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            preview_x,
            preview_y,
            SCROLL_PREVIEW_WIDTH,
            SCROLL_PREVIEW_HEIGHT,
            null_mut(),
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        if !preview.is_null() {
            state.scroll_preview = preview;
            ShowWindow(preview, SW_SHOWNOACTIVATE);
            InvalidateRect(preview, null(), 0);
        }

        // Replace the captured full-screen selection image with live desktop
        // content while keeping the four click-through dimming panels visible.
        ShowWindow(state.overlay, SW_HIDE);
        state.pending_scroll_input = ScrollInputDelta::default();
        match ScrollInputListener::install(selection, state.scroll_target, axis) {
            Ok(listener) => state.scroll_input = Some(listener),
            Err(_) => state.scroll_input = None,
        }
        if !super::register_scroll_escape_hotkey(state) {
            set_error(state, "无法注册滚动截图的 ESC 快捷键".into());
        }
        if !state.scroll_target.is_null() {
            let _ = SetForegroundWindow(state.scroll_target);
        }
        if !state.scroll_preview.is_null() {
            show_scroll_tip(state, "滚动截图已就绪，请继续滚动页面".into());
        }
        SetTimer(state.overlay, TIMER_SCROLL, SCROLL_OBSERVE_INTERVAL, None);
    }
}

pub(super) fn drain_scroll_input(state: &mut UiState) {
    let Some(listener) = state.scroll_input.as_ref() else {
        return;
    };
    let delta = listener.drain();
    state.pending_scroll_input.merge(delta);
}

pub(super) fn stop_scroll_input(state: &mut UiState) {
    drain_scroll_input(state);
    super::unregister_scroll_escape_hotkey(state);
    if state.scroll_input.is_none() && !state.pending_scroll_input.has_events() {
        state.pending_scroll_input = ScrollInputDelta::default();
        return;
    }
    state.scroll_input.take();
    state.pending_scroll_input = ScrollInputDelta::default();
}

pub(super) fn scroll_tick(state: &mut UiState) {
    let Some(selection) = state.capture.as_ref().and_then(|capture| capture.selection) else {
        return;
    };
    drain_scroll_input(state);
    let overlay = state.overlay;
    if overlay.is_null() || state.scroll_control.is_null() {
        return;
    }
    let probe = ScreenCapture::grab_probe(selection);
    let Some(probe) = probe else {
        if let Some(capture) = state.capture.as_mut() {
            if let Some(scroll) = capture.scroll.as_mut() {
                scroll.pause_after_capture_failure();
            }
        }
        set_error(state, "无法读取滚动观察探针".into());
        unsafe {
            SetTimer(overlay, TIMER_SCROLL, SCROLL_OBSERVE_INTERVAL, None);
        }
        return;
    };
    let (observer_result, phase, ready) = {
        let Some(capture) = state.capture.as_mut() else {
            return;
        };
        let Some(scroll) = capture.scroll.as_mut() else {
            return;
        };
        let result = scroll.observe_probe(probe);
        (result, scroll.phase, scroll.ready_to_capture())
    };
    let fast_motion_capture = observer_result == FrameResult::MotionDetected;
    if !ready && !fast_motion_capture {
        match (observer_result, phase) {
            (FrameResult::MotionDetected, CapturePhase::Settling)
            | (FrameResult::Unchanged, CapturePhase::Settling) => {
                set_scroll_status(state, "画面正在稳定".into());
            }
            (FrameResult::Unchanged, CapturePhase::Observing) => {
                set_scroll_status(state, "未检测到滚动变化".into());
            }
            (FrameResult::InvalidFrame, _) => {
                set_error(state, "观察帧无效，已暂停采集".into());
            }
            _ => {}
        }
    } else {
        set_scroll_preview_feedback(state, ScrollPreviewFeedback::Matching);
        drain_scroll_input(state);
        scroll_capture_tick(state, selection);
    }
    if !state.overlay.is_null() {
        unsafe {
            SetTimer(state.overlay, TIMER_SCROLL, SCROLL_OBSERVE_INTERVAL, None);
        }
    }
}

pub(super) fn scroll_capture_tick(state: &mut UiState, selection: ScreenRect) {
    let overlay = state.overlay;
    if overlay.is_null() {
        return;
    }
    let input_delta = std::mem::take(&mut state.pending_scroll_input);
    let motion_axis = state
        .capture
        .as_ref()
        .and_then(|capture| capture.scroll.as_ref())
        .map(|scroll| scroll.match_axis);
    if let Some(axis) = motion_axis {
        state
            .scroll_motion_model
            .ensure_context(state.scroll_target as isize, axis);
    }
    let Some(frame_id) = state
        .capture
        .as_mut()
        .and_then(|capture| capture.scroll.as_mut())
        .map(ScrollingCapture::next_capture_frame_id)
    else {
        return;
    };
    let current = ScreenCapture::grab_region(selection);
    let Some(current) = current else {
        if let Some(capture) = state.capture.as_mut() {
            if let Some(scroll) = capture.scroll.as_mut() {
                scroll.pause_after_capture_failure_with_input(input_delta);
            }
        }
        set_error(state, "无法读取滚动区域".into());
        unsafe {
            SetTimer(overlay, TIMER_SCROLL, SCROLL_OBSERVE_INTERVAL, None);
        }
        return;
    };
    let result = {
        let (capture_state, motion_model) =
            (&mut state.capture, &mut state.scroll_motion_model);
        let Some(capture) = capture_state.as_mut() else {
            return;
        };
        let Some(scroll) = capture.scroll.as_mut() else {
            return;
        };
        scroll.capture_stable_with_id_and_motion(
            current,
            selection.width(),
            selection.height(),
            MAX_SCROLL_EXTENT,
            frame_id,
            input_delta,
            motion_model,
        )
    };
    let preview_feedback = match result {
        FrameResult::Matched { .. } => ScrollPreviewFeedback::Matched,
        FrameResult::NoOverlap
        | FrameResult::ReverseDirection { .. }
        | FrameResult::InvalidFrame
        | FrameResult::ReachedLimit => ScrollPreviewFeedback::Rejected,
        FrameResult::Unchanged => ScrollPreviewFeedback::Waiting,
        FrameResult::MotionDetected => ScrollPreviewFeedback::Matching,
    };
    set_scroll_preview_feedback(state, preview_feedback);
    unsafe {
        if !state.scroll_preview.is_null() {
            InvalidateRect(state.scroll_preview, null(), 0);
        }
    }
    match result {
        FrameResult::ReverseDirection { .. } => {
            set_error(state, "检测到反向滚动，已暂停采集".into())
        }
        FrameResult::NoOverlap => set_error(state, "未找到可靠重叠，请继续滚动".into()),
        FrameResult::InvalidFrame => set_error(state, "帧无效，已暂停采集".into()),
        FrameResult::ReachedLimit => set_error(state, "已达到长图尺寸上限".into()),
        FrameResult::Matched { .. } | FrameResult::Unchanged | FrameResult::MotionDetected => {
            clear_error(state)
        }
    }
}

pub(super) fn scroll_control_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    unsafe {
        match message {
            WM_PAINT => {
                paint_scroll_toolbar(hwnd, state);
                0
            }
            WM_ERASEBKGND => 1,
            WM_MOUSEMOVE => {
                if !state.scroll_toolbar_tracking_mouse {
                    let mut tracking = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut tracking);
                    state.scroll_toolbar_tracking_mouse = true;
                }
                let point = ((lparam as i16) as i32, ((lparam >> 16) as i16) as i32);
                let next = state
                    .capture
                    .as_ref()
                    .and_then(|_| toolbar_hit_test((0, 0), point, scroll_toolbar_layout()));
                let previous = state.capture.as_ref().map(|capture| capture.toolbar_hover);
                if previous.is_some_and(|previous| previous != next) {
                    if let Some(capture) = state.capture.as_mut() {
                        capture.toolbar_hover = next;
                    }
                    if let Some(Some(index)) = previous {
                        invalidate_scroll_toolbar_button(hwnd, index);
                    }
                    if let Some(index) = next {
                        invalidate_scroll_toolbar_button(hwnd, index);
                    }
                    update_scroll_toolbar_tooltip(state, next);
                }
                0
            }
            WM_MOUSELEAVE => {
                state.scroll_toolbar_tracking_mouse = false;
                let previous = state.capture.as_ref().map(|capture| capture.toolbar_hover);
                if let Some(capture) = state.capture.as_mut() {
                    capture.toolbar_hover = None;
                }
                if let Some(Some(index)) = previous {
                    invalidate_scroll_toolbar_button(hwnd, index);
                }
                update_scroll_toolbar_tooltip(state, None);
                0
            }
            WM_LBUTTONUP => {
                let point = ((lparam as i16) as i32, ((lparam >> 16) as i16) as i32);
                let action = state.capture.as_ref().and_then(|_| {
                    toolbar_hit_test((0, 0), point, scroll_toolbar_layout())
                        .and_then(scroll_toolbar_action)
                });
                match action {
                    Some(ToolbarAction::Pin) => {
                        finish(state);
                        pin_last(state);
                    }
                    Some(ToolbarAction::Ocr) => super::ocr_ui::start_text_extraction(state),
                    Some(ToolbarAction::Copy) => copy_capture(state),
                    Some(ToolbarAction::Save) => finish_with_dialog(state),
                    Some(ToolbarAction::Cancel) => cancel(state),
                    Some(ToolbarAction::Finish) => finish(state),
                    Some(ToolbarAction::Edit) => open_editor(state),
                    _ => {}
                }
                0
            }
            WM_RBUTTONUP => {
                cancel(state);
                0
            }
            WM_KEYDOWN if wparam as u32 == VK_ESCAPE_CODE => {
                cancel(state);
                0
            }
            WM_KEYDOWN if wparam as u32 == VK_RETURN_CODE => {
                finish(state);
                0
            }
            WM_DESTROY => {
                if state.scroll_control == hwnd {
                    destroy_scroll_toolbar_tooltip(state);
                    state.scroll_toolbar_tracking_mouse = false;
                    state.scroll_control = null_mut();
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

unsafe fn invalidate_scroll_toolbar_button(hwnd: HWND, index: usize) {
    let Some((left, top)) = toolbar_button_origin((0, 0), index, scroll_toolbar_layout()) else {
        return;
    };
    let rect = RECT {
        left,
        top,
        right: left + TOOLBAR_BUTTON_WIDTH,
        bottom: top + TOOLBAR_BUTTON_HEIGHT,
    };
    InvalidateRect(hwnd, &rect, 0);
}

fn scroll_toolbar_tooltip_label(index: usize) -> Option<&'static str> {
    let action = scroll_toolbar_action(index)?;
    if action == ToolbarAction::Copy {
        return Some("\u{590d}\u{5236}");
    }
    TOOLBAR_ITEMS
        .iter()
        .find(|item| item.action == action)
        .map(|item| item.label)
}

unsafe fn update_scroll_toolbar_tooltip(state: &mut UiState, index: Option<usize>) {
    destroy_scroll_toolbar_tooltip(state);
    let Some(index) = index else {
        return;
    };
    let Some(label) = scroll_toolbar_tooltip_label(index) else {
        return;
    };
    let Some(screen) = state.capture.as_ref().map(|capture| capture.screen_rect) else {
        return;
    };
    if state.scroll_control.is_null() {
        return;
    }

    let mut toolbar_rect: RECT = zeroed();
    if GetWindowRect(state.scroll_control, &mut toolbar_rect) == 0 {
        return;
    }
    let layout = scroll_toolbar_layout();
    let Some((button_x, _button_y)) =
        toolbar_button_origin((toolbar_rect.left, toolbar_rect.top), index, layout)
    else {
        return;
    };

    let width = if label.chars().count() > 5 { 96 } else { 68 };
    let height = 24;
    let min_left = screen.left + 4;
    let max_left = (screen.right - width - 4).max(min_left);
    let center_x = button_x + crate::toolbar::TOOLBAR_BUTTON_WIDTH / 2;
    let left = (center_x - width / 2).clamp(min_left, max_left);
    let below = toolbar_rect.bottom + 6;
    let min_top = screen.top + 4;
    let max_top = (screen.bottom - height - 4).max(min_top);
    let preferred_top = if below + height <= screen.bottom - 4 {
        below
    } else {
        toolbar_rect.top - height - 6
    };
    let top = preferred_top.clamp(min_top, max_top);

    let class_name = wide("AgileTilesScreenshotCoreV2");
    let title = wide(label);
    let tooltip = CreateWindowExW(
        WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_TRANSPARENT,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_POPUP,
        left,
        top,
        width,
        height,
        null_mut(),
        null_mut(),
        GetModuleHandleW(null()),
        state as *mut UiState as *const c_void,
    );
    if tooltip.is_null() {
        return;
    }
    state.scroll_tooltip = tooltip;
    state.scroll_tooltip_text = Some(label.to_owned());
    ShowWindow(tooltip, SW_SHOWNOACTIVATE);
    InvalidateRect(tooltip, null(), 0);
}

unsafe fn destroy_scroll_toolbar_tooltip(state: &mut UiState) {
    if !state.scroll_tooltip.is_null() {
        DestroyWindow(state.scroll_tooltip);
        state.scroll_tooltip = null_mut();
    }
    state.scroll_tooltip_text = None;
}

pub(super) unsafe fn scroll_tooltip_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint_struct: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut paint_struct);
            let mut client: RECT = zeroed();
            GetClientRect(hwnd, &mut client);
            let background = CreateSolidBrush(0x001d2129);
            FillRect(dc, &client, background);
            DeleteObject(background as HGDIOBJ);
            let old_font = SelectObject(
                dc,
                windows_sys::Win32::Graphics::Gdi::GetStockObject(super::DEFAULT_GUI_FONT),
            );
            SetBkMode(dc, 1);
            SetTextColor(dc, 0x00ffffff);
            if let Some(text) = state.scroll_tooltip_text.as_ref() {
                let text = wide(text);
                DrawTextW(dc, text.as_ptr(), -1, &mut client, 1 | 4 | 0x20 | 0x8000);
            }
            SelectObject(dc, old_font);
            EndPaint(hwnd, &paint_struct);
            0
        }
        WM_MOUSEACTIVATE => 3,
        WM_NCHITTEST => -1,
        WM_DESTROY => {
            if state.scroll_tooltip == hwnd {
                state.scroll_tooltip = null_mut();
                state.scroll_tooltip_text = None;
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

pub(super) unsafe fn scroll_border_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint_scroll_border(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => -1,
        WM_MOUSEACTIVATE => 3,
        WM_DESTROY => {
            if state.scroll_border == hwnd {
                state.scroll_border = null_mut();
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

pub(super) unsafe fn paint_scroll_border(hwnd: HWND) {
    let mut paint_struct: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(hwnd, &mut paint_struct);
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    let background = CreateSolidBrush(0x00000000);
    FillRect(dc, &client, background);
    DeleteObject(background as HGDIOBJ);
    draw_rect_border(
        dc,
        1,
        1,
        client.right - client.left - 2,
        client.bottom - client.top - 2,
        0x00f76f16,
        2,
    );
    EndPaint(hwnd, &paint_struct);
}

pub(super) unsafe fn paint_scroll_toolbar(hwnd: HWND, state: &UiState) {
    let mut paint_struct: PAINTSTRUCT = zeroed();
    let target_dc = BeginPaint(hwnd, &mut paint_struct);
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    let width = client.right - client.left;
    let height = client.bottom - client.top;
    let memory_dc = CreateCompatibleDC(target_dc);
    let bitmap = CreateCompatibleBitmap(target_dc, width, height);
    if memory_dc.is_null() || bitmap.is_null() {
        if !memory_dc.is_null() {
            DeleteDC(memory_dc);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap as HGDIOBJ);
        }
        paint_scroll_toolbar_to_dc(target_dc, &client, state);
        EndPaint(hwnd, &paint_struct);
        return;
    }
    let old_bitmap = SelectObject(memory_dc, bitmap as HGDIOBJ);
    paint_scroll_toolbar_to_dc(memory_dc, &client, state);
    BitBlt(target_dc, 0, 0, width, height, memory_dc, 0, 0, SRCCOPY);
    SelectObject(memory_dc, old_bitmap);
    DeleteObject(bitmap as HGDIOBJ);
    DeleteDC(memory_dc);
    EndPaint(hwnd, &paint_struct);
}

unsafe fn paint_scroll_toolbar_to_dc(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    client: &RECT,
    state: &UiState,
) {
    let background = CreateSolidBrush(0x00ffffff);
    FillRect(dc, client, background);
    DeleteObject(background as HGDIOBJ);

    if let Some(capture) = state.capture.as_ref() {
        let layout = scroll_toolbar_layout();
        draw_toolbar_gdi(
            dc,
            0,
            0,
            layout,
            capture.tool,
            capture.toolbar_hover,
            false,
            scroll_toolbar_action,
            SCROLL_TOOLBAR_ACTIONS.len(),
            capture.annotation_color,
            capture.stroke_width,
            capture.brush_size,
            capture.text_size,
            capture.stroke_style,
            false,
        );
    }
}

pub(super) unsafe fn scroll_preview_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint_scroll_preview(hwnd, state);
            0
        }
        WM_DESTROY => {
            if state.scroll_preview == hwnd {
                state.scroll_preview = null_mut();
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

pub(super) unsafe fn scroll_tip_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_scroll_tip(hwnd, state);
            0
        }
        WM_MOUSEACTIVATE => 3, // MA_NOACTIVATE
        WM_NCHITTEST => -1,     // HTTRANSPARENT
        WM_DESTROY => {
            if state.scroll_tip == hwnd {
                state.scroll_tip = null_mut();
            }
            state.scroll_tip_text = None;
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

pub(super) unsafe fn scroll_mask_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint_struct: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut paint_struct);
            let mut client: RECT = zeroed();
            GetClientRect(hwnd, &mut client);
            let shade = CreateSolidBrush(0x000000);
            FillRect(dc, &client, shade);
            DeleteObject(shade as HGDIOBJ);
            EndPaint(hwnd, &paint_struct);
            0
        }
        WM_MOUSEACTIVATE => 3, // MA_NOACTIVATE
        WM_NCHITTEST => -1,     // HTTRANSPARENT
        WM_DESTROY => {
            state.scroll_masks.retain(|mask| *mask != hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

pub(super) unsafe fn paint_scroll_tip(hwnd: HWND, state: &UiState) {
    let mut paint_struct: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(hwnd, &mut paint_struct);
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    let background = CreateSolidBrush(0x001d2129);
    let border = CreatePen(0, 1, 0x00333842);
    let old_brush = SelectObject(dc, background as HGDIOBJ);
    let old_pen = SelectObject(dc, border as HGDIOBJ);
    RoundRect(dc, client.left, client.top, client.right, client.bottom, 10, 10);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(border as HGDIOBJ);
    DeleteObject(background as HGDIOBJ);

    if let Some(message) = state.scroll_tip_text.as_deref() {
        SetBkMode(dc, 1);
        SetTextColor(dc, 0x00f4f7fb);
        let text = wide(message);
        let mut text_rect = RECT {
            left: 14,
            top: 7,
            right: client.right - 14,
            bottom: client.bottom - 7,
        };
        DrawTextW(dc, text.as_ptr(), -1, &mut text_rect, 0x1 | 0x4 | 0x10 | 0x8000);
    }
    EndPaint(hwnd, &paint_struct);
}

pub(super) unsafe fn paint_scroll_preview(hwnd: HWND, state: &UiState) {
    let mut paint_struct: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(hwnd, &mut paint_struct);
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    let width = (client.right - client.left).max(1);
    let height = (client.bottom - client.top).max(1);

    let background = CreateSolidBrush(0x001d2633);
    FillRect(dc, &client, background);
    DeleteObject(background as HGDIOBJ);

    SetBkMode(dc, 1);
    let mode_title = match state
        .capture
        .as_ref()
        .and_then(|capture| capture.scroll_axis)
    {
        Some(MatchAxis::Horizontal) => "横向滚动预览",
        Some(MatchAxis::Vertical) => "纵向滚动预览",
        None => "滚动预览",
    };
    let title = wide(mode_title);
    SetTextColor(dc, 0x00f4f7fb);
    let mut title_rect = RECT {
        left: 10,
        top: 7,
        right: width - 10,
        bottom: 24,
    };
    DrawTextW(dc, title.as_ptr(), -1, &mut title_rect, 0x20 | 0x8000);

    let feedback = match state.scroll_preview_feedback {
        ScrollPreviewFeedback::Waiting => ("等待页面滚动", 0x00b7c4d4),
        ScrollPreviewFeedback::Matching => ("检测到滚动，正在匹配…", 0x00e5c27b),
        ScrollPreviewFeedback::Matched => ("拼接成功，可以继续滚动", 0x0086d69a),
        ScrollPreviewFeedback::Rejected => ("未可靠匹配，长图未修改", 0x008b9df0),
    };
    let feedback_text = wide(feedback.0);
    let mut feedback_rect = RECT {
        left: 10,
        top: 26,
        right: width - 10,
        bottom: 48,
    };
    SetTextColor(dc, feedback.1);
    DrawTextW(dc, feedback_text.as_ptr(), -1, &mut feedback_rect, 0x20 | 0x8000);

    let preview_left = 10;
    let preview_top = 54;
    let preview_width = (width - 20).max(1);
    let preview_height = (height - 66).max(1);
    let preview_background = CreateSolidBrush(0x000b1017);
    let preview_rect = RECT {
        left: preview_left,
        top: preview_top,
        right: preview_left + preview_width,
        bottom: preview_top + preview_height,
    };
    FillRect(dc, &preview_rect, preview_background);
    DeleteObject(preview_background as HGDIOBJ);

    if let Some((image_width, image_height, pixels)) = state
        .capture
        .as_ref()
        .and_then(|capture| capture.scroll.as_ref())
        .and_then(|scroll| scroll.assembled.as_ref())
    {
        if *image_width > 0 && *image_height > 0 && !pixels.is_empty() {
            let scale = (preview_width as f64 / *image_width as f64)
                .min(preview_height as f64 / *image_height as f64);
            let target_width = ((*image_width as f64 * scale).round() as i32)
                .max(1)
                .min(preview_width);
            let target_height = ((*image_height as f64 * scale).round() as i32)
                .max(1)
                .min(preview_height);
            let target_x = preview_left + (preview_width - target_width) / 2;
            let target_y = preview_top + (preview_height - target_height) / 2;
            draw_rgba_preview(
                dc,
                pixels,
                *image_width as i32,
                *image_height as i32,
                target_x,
                target_y,
                target_width,
                target_height,
            );
            let size_text = wide(&format!("{} × {}", image_width, image_height));
            let mut size_rect = RECT {
                left: 14,
                top: height - 30,
                right: width - 14,
                bottom: height - 10,
            };
            SetTextColor(dc, 0x00b7c4d4);
            DrawTextW(dc, size_text.as_ptr(), -1, &mut size_rect, 1 | 4);
        }
    } else {
        let waiting = wide("等待首次滚动…");
        let mut waiting_rect = preview_rect;
        SetTextColor(dc, 0x00b7c4d4);
        DrawTextW(dc, waiting.as_ptr(), -1, &mut waiting_rect, 1 | 4);
    }

    draw_rect_border(dc, 0, 0, width, height, 0x005a6c82, 1);
    EndPaint(hwnd, &paint_struct);
}

pub(super) fn set_scroll_status(state: &mut UiState, message: String) {
    if let Ok(mut status) = state.status.lock() {
        status.error = Some(message.clone());
    }
    if !state.scroll_preview.is_null() {
        show_scroll_tip(state, message);
    }
}

pub(super) fn set_scroll_preview_feedback(state: &mut UiState, feedback: ScrollPreviewFeedback) {
    if state.scroll_preview_feedback == feedback {
        return;
    }
    state.scroll_preview_feedback = feedback;
    if !state.scroll_preview.is_null() {
        unsafe {
            InvalidateRect(state.scroll_preview, null(), 0);
        }
    }
}

pub(super) fn show_scroll_tip(state: &mut UiState, message: String) {
    if state.scroll_tip_text.as_deref() == Some(message.as_str()) {
        return;
    }
    let Some((screen, selection)) = state
        .capture
        .as_ref()
        .and_then(|capture| capture.selection.map(|selection| (capture.screen_rect, selection)))
    else {
        return;
    };

    unsafe {
        let created = state.scroll_tip.is_null();
        if created {
            let preview_position = scroll_preview_position(screen, selection);
            let Some((x, y)) = scroll_tip_position(screen, selection, preview_position) else {
                return;
            };
            let class_name = wide("AgileTilesScreenshotCoreV2");
            let instance = GetModuleHandleW(null());
            let tip = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_TRANSPARENT,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_POPUP,
                x,
                y,
                SCROLL_TIP_WIDTH,
                SCROLL_TIP_HEIGHT,
                null_mut(),
                null_mut(),
                instance,
                state as *const UiState as *const c_void,
            );
            if tip.is_null() {
                return;
            }
            state.scroll_tip = tip;
        }
        state.scroll_tip_text = Some(message);
        InvalidateRect(state.scroll_tip, null(), 0);
        if created {
            ShowWindow(state.scroll_tip, SW_SHOWNOACTIVATE);
        }
    }
}
