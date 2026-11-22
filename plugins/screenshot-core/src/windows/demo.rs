//! SnowAir-inspired Presentation / Demo Mode & Zero-Lag Recording Live Annotation Canvas:
//! - Fullscreen live transparent desktop canvas OR selection-scoped recording annotation canvas
//! - Zero-lag mouse click-through: hides layered chroma-key window completely when empty & passing through,
//!   and flips WS_EX_TRANSPARENT without DWM surface reallocation when annotations remain visible
//! - Double-buffered offscreen GDI rendering for flicker-free 60 FPS live drawing
//! - Smart fading laser pen (`FadePen`) timer that runs only while laser strokes exist
//! - Solid whiteboard / dark studio / blackboard green / multi-image slides (PgUp / PgDn)

use super::draw::draw_single_annotation_gdi;
use super::input::{VK_CONTROL_CODE, VK_ESCAPE_CODE, VK_Y, VK_Z};
use super::paint::{create_gdi_surface, draw_rgba_preview, GdiSurface};
use super::toolbar_paint::{draw_toolbar_gdi, draw_toolbar_menu_gdi};
use super::{wide, UiState};
use crate::capture::ScreenCapture;
use crate::model::{Annotation, ArrowVariant, ScreenPoint, ScreenRect, StrokeStyle};
use crate::toolbar::{
    hit_test as toolbar_hit_test, layout_for_item_count_scaled, menu_hit_test, menu_layout, Tool,
    ToolbarAction, ToolbarLayout, ToolbarMenu, DEMO_TOOLBAR_ACTIONS,
};

use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetDC, InvalidateRect,
    ReleaseDC, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::MK_LBUTTON;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, KillTimer, SetLayeredWindowAttributes, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, LWA_ALPHA, LWA_COLORKEY,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WM_DESTROY,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONUP, WM_TIMER, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

pub(crate) const TIMER_DEMO_FADE: usize = 0x5260;
const TRANSPARENT_KEY: u32 = 0x0003_0201; // BGR chroma key for transparent desktop pass-through

const VK_PRIOR: u32 = 0x21; // Page Up
const VK_NEXT: u32 = 0x22; // Page Down
const VK_OEM_3: u32 = 0xC0; // ` backtick toggle pass-through

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DemoBackground {
    Transparent,
    Solid(u32), // COLORREF
    Slides {
        images: Vec<(usize, usize, Vec<u8>)>,
        index: usize,
    },
}

pub(crate) struct DemoState {
    pub(crate) canvas_hwnd: HWND,
    pub(crate) toolbar_hwnd: HWND,
    pub(crate) screen_rect: ScreenRect,
    pub(crate) recording_mode: bool,
    pub(crate) canvas_visible: bool,
    pub(crate) fade_timer_active: bool,
    pub(crate) tool: Tool,
    pub(crate) color: [u8; 4],
    pub(crate) stroke_width: u32,
    pub(crate) passthrough: bool,
    pub(crate) background: DemoBackground,
    pub(crate) annotations: Vec<Annotation>,
    pub(crate) redo_annotations: Vec<Annotation>,
    pub(crate) drag_start: Option<ScreenPoint>,
    pub(crate) current_pos: Option<ScreenPoint>,
    pub(crate) number_next: u32,
    pub(crate) toolbar_hover: Option<usize>,
    pub(crate) toolbar_drag: Option<(ScreenPoint, ScreenPoint)>,
    pub(crate) bg_menu_open: bool,
    pub(crate) bg_menu_hover: Option<usize>,
    pub(crate) fade_ms: u64,
    back_buffer: Option<GdiSurface>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Start standalone Fullscreen Presentation / Demo Mode (with floating top/bottom SnowAir toolbar).
pub(crate) fn start_demo(state: &mut UiState) {
    if state.demo.is_some() {
        stop_demo(state);
        return;
    }
    let settings = state
        .settings
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    let default_tool = match settings.demo_default_tool.as_str() {
        "pen" => Tool::Pen,
        "none" => Tool::Select,
        _ => Tool::FadePen,
    };

    unsafe {
        let sx = GetSystemMetrics(76);
        let sy = GetSystemMetrics(77);
        let sw = GetSystemMetrics(78).max(800);
        let sh = GetSystemMetrics(79).max(600);
        let screen_rect = ScreenRect {
            left: sx,
            top: sy,
            right: sx + sw,
            bottom: sy + sh,
        };
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let instance = GetModuleHandleW(null());

        let canvas_hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            sx,
            sy,
            sw,
            sh,
            null_mut(),
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        if canvas_hwnd.is_null() {
            return;
        }
        SetLayeredWindowAttributes(canvas_hwnd, TRANSPARENT_KEY, 255, LWA_COLORKEY);

        let tb_layout = demo_toolbar_layout();
        let primary_w = GetSystemMetrics(0).max(800);
        let primary_h = GetSystemMetrics(1).max(600);
        let tb_x = (primary_w - tb_layout.width) / 2;
        let tb_y = if settings.demo_toolbar_pos == "bottom" {
            primary_h - tb_layout.height - 24
        } else {
            16
        };

        let toolbar_hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            tb_x,
            tb_y,
            tb_layout.width,
            tb_layout.height,
            null_mut(),
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        if toolbar_hwnd.is_null() {
            DestroyWindow(canvas_hwnd);
            return;
        }

        let initial_passthrough = default_tool == Tool::Select;
        state.demo = Some(DemoState {
            canvas_hwnd,
            toolbar_hwnd,
            screen_rect,
            recording_mode: false,
            canvas_visible: false,
            fade_timer_active: false,
            tool: default_tool,
            color: settings.annotation_rgba(),
            stroke_width: settings.stroke_width.max(4),
            passthrough: initial_passthrough,
            background: DemoBackground::Transparent,
            annotations: Vec::new(),
            redo_annotations: Vec::new(),
            drag_start: None,
            current_pos: None,
            number_next: 1,
            toolbar_hover: None,
            toolbar_drag: None,
            bg_menu_open: false,
            bg_menu_hover: None,
            fade_ms: u64::from(settings.demo_fade_ms),
            back_buffer: None,
        });

        apply_demo_passthrough(state);
        ShowWindow(toolbar_hwnd, SW_SHOWNOACTIVATE);
        InvalidateRect(toolbar_hwnd, null(), 0);

        if let Ok(mut status) = state.status.lock() {
            status.demo_active = true;
        }
    }
}

/// Lazily create a lightweight, selection-scoped live annotation canvas for Screen Recording
/// (NO separate second toolbar — controlled directly by the unified Recording Control Bar!).
pub(crate) fn ensure_recording_canvas(state: &mut UiState, selection: ScreenRect) {
    if let Some(demo) = state.demo.as_mut() {
        if demo.recording_mode {
            return;
        }
        stop_demo(state);
    }
    let settings = state
        .settings
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    let default_tool = match settings.demo_default_tool.as_str() {
        "pen" => Tool::Pen,
        _ => Tool::FadePen,
    };

    unsafe {
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let instance = GetModuleHandleW(null());
        let w = (selection.right - selection.left).max(16);
        let h = (selection.bottom - selection.top).max(16);
        let canvas_hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            selection.left,
            selection.top,
            w,
            h,
            null_mut(),
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        if canvas_hwnd.is_null() {
            return;
        }
        SetLayeredWindowAttributes(canvas_hwnd, TRANSPARENT_KEY, 255, LWA_COLORKEY);

        state.demo = Some(DemoState {
            canvas_hwnd,
            toolbar_hwnd: null_mut(),
            screen_rect: selection,
            recording_mode: true,
            canvas_visible: false,
            fade_timer_active: false,
            tool: default_tool,
            color: settings.annotation_rgba(),
            stroke_width: settings.stroke_width.max(4),
            passthrough: false,
            background: DemoBackground::Transparent,
            annotations: Vec::new(),
            redo_annotations: Vec::new(),
            drag_start: None,
            current_pos: None,
            number_next: 1,
            toolbar_hover: None,
            toolbar_drag: None,
            bg_menu_open: false,
            bg_menu_hover: None,
            fade_ms: u64::from(settings.demo_fade_ms),
            back_buffer: None,
        });
        apply_demo_passthrough(state);
    }
}

pub(crate) fn set_recording_canvas_passthrough(state: &mut UiState, passthrough: bool) {
    state.recording_passthrough = passthrough;
    if !passthrough {
        if let Some(sel) = state.recording_selection {
            ensure_recording_canvas(state, sel);
        }
    }
    if let Some(demo) = state.demo.as_mut() {
        demo.passthrough = passthrough;
        if !passthrough && demo.tool == Tool::Select {
            demo.tool = Tool::FadePen;
        }
    }
    unsafe {
        apply_demo_passthrough(state);
    }
}

pub(crate) fn set_recording_tool(state: &mut UiState, tool: Tool) {
    if state.recording_passthrough {
        set_recording_canvas_passthrough(state, false);
    }
    if let Some(demo) = state.demo.as_mut() {
        demo.tool = tool;
        demo.passthrough = false;
    }
    unsafe {
        apply_demo_passthrough(state);
    }
}

pub(crate) fn recording_canvas_tool(state: &UiState) -> Tool {
    state
        .demo
        .as_ref()
        .filter(|d| d.recording_mode)
        .map(|d| d.tool)
        .unwrap_or(Tool::FadePen)
}

pub(crate) fn recording_canvas_has_annotations(state: &UiState) -> bool {
    state
        .demo
        .as_ref()
        .is_some_and(|d| d.recording_mode && !d.annotations.is_empty())
}

pub(crate) fn undo_recording_annotation(state: &mut UiState) {
    let mut should_update_vis = false;
    let mut hwnd: HWND = null_mut();
    if let Some(demo) = state.demo.as_mut() {
        if let Some(a) = demo.annotations.pop() {
            demo.redo_annotations.push(a);
            hwnd = demo.canvas_hwnd;
            should_update_vis = demo.annotations.is_empty() && demo.passthrough;
        }
    }
    unsafe {
        if !hwnd.is_null() {
            InvalidateRect(hwnd, null(), 0);
        }
        if should_update_vis {
            apply_demo_passthrough(state);
        }
    }
}

pub(crate) fn clear_recording_annotations(state: &mut UiState) {
    let mut hwnd: HWND = null_mut();
    if let Some(demo) = state.demo.as_mut() {
        demo.annotations.clear();
        demo.redo_annotations.clear();
        demo.number_next = 1;
        hwnd = demo.canvas_hwnd;
    }
    unsafe {
        if !hwnd.is_null() {
            InvalidateRect(hwnd, null(), 0);
        }
        apply_demo_passthrough(state);
    }
}

pub(crate) fn stop_demo(state: &mut UiState) {
    let Some(demo) = state.demo.take() else {
        return;
    };
    unsafe {
        if !demo.canvas_hwnd.is_null() {
            if demo.fade_timer_active {
                KillTimer(demo.canvas_hwnd, TIMER_DEMO_FADE);
            }
            DestroyWindow(demo.canvas_hwnd);
        }
        if !demo.toolbar_hwnd.is_null() {
            DestroyWindow(demo.toolbar_hwnd);
        }
    }
    if let Ok(mut status) = state.status.lock() {
        status.demo_active = false;
    }
}

fn demo_toolbar_layout() -> ToolbarLayout {
    layout_for_item_count_scaled(
        DEMO_TOOLBAR_ACTIONS.len(),
        i32::MAX,
        crate::theme::system_dpi_scale(),
    )
}

unsafe fn ensure_fade_timer(demo: &mut DemoState) {
    let has_fade = demo
        .annotations
        .iter()
        .any(|a| matches!(a, Annotation::FadePen { .. }));
    if has_fade && !demo.fade_timer_active {
        SetTimer(demo.canvas_hwnd, TIMER_DEMO_FADE, 90, None);
        demo.fade_timer_active = true;
    } else if !has_fade && demo.fade_timer_active {
        KillTimer(demo.canvas_hwnd, TIMER_DEMO_FADE);
        demo.fade_timer_active = false;
    }
}

/// Zero-lag pass-through switcher:
/// - If passing through AND nothing is painted on a transparent canvas -> SW_HIDE (0% DWM cost!)
/// - Otherwise -> flip ONLY WS_EX_TRANSPARENT without SWP_FRAMECHANGED or SetLayeredWindowAttributes!
unsafe fn apply_demo_passthrough(state: &mut UiState) {
    let Some(demo) = state.demo.as_mut() else {
        return;
    };
    let should_hide = demo.passthrough
        && demo.annotations.is_empty()
        && matches!(demo.background, DemoBackground::Transparent)
        && !demo.bg_menu_open;

    if should_hide {
        if demo.canvas_visible {
            ShowWindow(demo.canvas_hwnd, SW_HIDE);
            demo.canvas_visible = false;
        }
        return;
    }

    let cur = GetWindowLongPtrW(demo.canvas_hwnd, GWL_EXSTYLE) as u32;
    let next = if demo.passthrough {
        cur | WS_EX_TRANSPARENT
    } else {
        cur & !WS_EX_TRANSPARENT
    };
    if cur != next {
        SetWindowLongPtrW(demo.canvas_hwnd, GWL_EXSTYLE, next as isize);
    }
    if !demo.canvas_visible {
        ShowWindow(demo.canvas_hwnd, SW_SHOWNOACTIVATE);
        demo.canvas_visible = true;
        InvalidateRect(demo.canvas_hwnd, null(), 0);
        // Ensure recording toolbar stays visually above the annotation canvas
        if !state.recording_toolbar.is_null() {
            SetWindowPos(
                state.recording_toolbar,
                (-1isize) as HWND, // HWND_TOPMOST
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOSIZE | 0x0002, // SWP_NOMOVE
            );
        }
    }
}

pub(crate) unsafe fn demo_canvas_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_TIMER if wparam == TIMER_DEMO_FADE => {
            let now = now_ms();
            let mut changed = false;
            let mut should_sync_vis = false;
            if let Some(demo) = state.demo.as_mut() {
                let before = demo.annotations.len();
                let fade = demo.fade_ms;
                demo.annotations.retain(|a| match a {
                    Annotation::FadePen { created_ms, .. } => {
                        *created_ms == 0 || now.saturating_sub(*created_ms) < fade
                    }
                    _ => true,
                });
                changed = demo.annotations.len() != before;
                ensure_fade_timer(demo);
                should_sync_vis = changed && demo.passthrough && demo.annotations.is_empty();
            }
            if should_sync_vis {
                apply_demo_passthrough(state);
            } else if changed {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_PAINT => {
            paint_demo_canvas(hwnd, state);
            0
        }
        WM_LBUTTONDOWN => {
            let pt = ScreenPoint {
                x: (lparam as i16) as i32,
                y: ((lparam >> 16) as i16) as i32,
            };
            if let Some(demo) = state.demo.as_mut() {
                if demo.bg_menu_open {
                    let menu_l = demo_bg_menu_layout(demo);
                    if let Some(idx) = menu_hit_test(menu_l, (pt.x, pt.y)) {
                        demo.bg_menu_open = false;
                        apply_demo_background_choice(state, idx);
                        return 0;
                    }
                    demo.bg_menu_open = false;
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                let world = ScreenPoint {
                    x: pt.x + demo.screen_rect.left,
                    y: pt.y + demo.screen_rect.top,
                };
                match demo.tool {
                    Tool::Number => {
                        let n = demo.number_next;
                        demo.number_next = demo.number_next.saturating_add(1);
                        demo.annotations.push(Annotation::Number(world, n, demo.color));
                        demo.redo_annotations.clear();
                        InvalidateRect(hwnd, null(), 0);
                    }
                    Tool::Eraser => {
                        if let Some(pos) = demo.annotations.iter().rposition(|a| a.hit_test(world, 12)) {
                            let rem = demo.annotations.remove(pos);
                            demo.redo_annotations.push(rem);
                        }
                        demo.drag_start = Some(world);
                        SetCapture(hwnd);
                        InvalidateRect(hwnd, null(), 0);
                    }
                    Tool::Pen => {
                        demo.drag_start = Some(world);
                        demo.current_pos = Some(world);
                        demo.annotations.push(Annotation::Pen(
                            vec![world],
                            demo.color,
                            demo.stroke_width,
                            StrokeStyle::Solid,
                        ));
                        demo.redo_annotations.clear();
                        SetCapture(hwnd);
                        InvalidateRect(hwnd, null(), 0);
                    }
                    Tool::FadePen => {
                        demo.drag_start = Some(world);
                        demo.current_pos = Some(world);
                        demo.annotations.push(Annotation::FadePen {
                            points: vec![world],
                            color: demo.color,
                            width: demo.stroke_width,
                            created_ms: 0,
                        });
                        ensure_fade_timer(demo);
                        SetCapture(hwnd);
                        InvalidateRect(hwnd, null(), 0);
                    }
                    Tool::Select => {}
                    _ => {
                        demo.drag_start = Some(world);
                        demo.current_pos = Some(world);
                        SetCapture(hwnd);
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            let pt = ScreenPoint {
                x: (lparam as i16) as i32,
                y: ((lparam >> 16) as i16) as i32,
            };
            let held = wparam & MK_LBUTTON as usize != 0;
            if let Some(demo) = state.demo.as_mut() {
                if demo.bg_menu_open {
                    let menu_l = demo_bg_menu_layout(demo);
                    let next = menu_hit_test(menu_l, (pt.x, pt.y));
                    if demo.bg_menu_hover != next {
                        demo.bg_menu_hover = next;
                        InvalidateRect(hwnd, null(), 0);
                    }
                    return 0;
                }
                if !held || demo.drag_start.is_none() {
                    return 0;
                }
                let world = ScreenPoint {
                    x: pt.x + demo.screen_rect.left,
                    y: pt.y + demo.screen_rect.top,
                };
                demo.current_pos = Some(world);
                match demo.tool {
                    Tool::Eraser => {
                        if let Some(pos) = demo.annotations.iter().rposition(|a| a.hit_test(world, 12)) {
                            let rem = demo.annotations.remove(pos);
                            demo.redo_annotations.push(rem);
                        }
                    }
                    Tool::Pen => {
                        if let Some(Annotation::Pen(pts, _, _, _)) = demo.annotations.last_mut() {
                            pts.push(world);
                        }
                    }
                    Tool::FadePen => {
                        if let Some(Annotation::FadePen { points, .. }) = demo.annotations.last_mut()
                        {
                            points.push(world);
                        }
                    }
                    _ => {}
                }
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            let pt = ScreenPoint {
                x: (lparam as i16) as i32,
                y: ((lparam >> 16) as i16) as i32,
            };
            ReleaseCapture();
            if let Some(demo) = state.demo.as_mut() {
                if let Some(start) = demo.drag_start.take() {
                    let end = ScreenPoint {
                        x: pt.x + demo.screen_rect.left,
                        y: pt.y + demo.screen_rect.top,
                    };
                    demo.current_pos = None;
                    let rect = ScreenRect::normalized(start, end);
                    match demo.tool {
                        Tool::FadePen => {
                            if let Some(Annotation::FadePen { created_ms, .. }) =
                                demo.annotations.last_mut()
                            {
                                *created_ms = now_ms();
                            }
                            ensure_fade_timer(demo);
                        }
                        Tool::Rectangle if rect.width() > 3 && rect.height() > 3 => {
                            demo.annotations.push(Annotation::Rect {
                                rect,
                                color: demo.color,
                                width: demo.stroke_width,
                                style: StrokeStyle::Solid,
                                filled: false,
                                rounded: false,
                            });
                            demo.redo_annotations.clear();
                        }
                        Tool::Ellipse if rect.width() > 3 && rect.height() > 3 => {
                            demo.annotations.push(Annotation::Ellipse {
                                rect,
                                color: demo.color,
                                width: demo.stroke_width,
                                style: StrokeStyle::Solid,
                                filled: false,
                            });
                            demo.redo_annotations.clear();
                        }
                        Tool::Arrow if (start.x - end.x).abs() > 3 || (start.y - end.y).abs() > 3 => {
                            demo.annotations.push(Annotation::Arrow {
                                start,
                                end,
                                color: demo.color,
                                width: demo.stroke_width,
                                style: StrokeStyle::Solid,
                                variant: ArrowVariant::Single,
                            });
                            demo.redo_annotations.clear();
                        }
                        Tool::Text => {
                            demo.annotations.push(Annotation::Text(
                                start,
                                "演示标注".into(),
                                22,
                                demo.color,
                            ));
                            demo.redo_annotations.clear();
                        }
                        _ => {}
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_RBUTTONUP => {
            // Instant right-click return to desktop pass-through!
            toggle_demo_passthrough(state);
            0
        }
        WM_KEYDOWN => {
            let key = wparam as u32;
            let ctrl = GetKeyState(VK_CONTROL_CODE) < 0;
            let is_rec = state.demo.as_ref().is_some_and(|d| d.recording_mode);
            if key == VK_ESCAPE_CODE {
                if is_rec {
                    set_recording_canvas_passthrough(state, true);
                    super::recording_ui::sync_toolbar_layout(state);
                } else {
                    stop_demo(state);
                }
            } else if key == VK_OEM_3 {
                toggle_demo_passthrough(state);
            } else if ctrl && key == VK_Z {
                if let Some(demo) = state.demo.as_mut() {
                    if let Some(a) = demo.annotations.pop() {
                        demo.redo_annotations.push(a);
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
            } else if ctrl && key == VK_Y {
                if let Some(demo) = state.demo.as_mut() {
                    if let Some(a) = demo.redo_annotations.pop() {
                        demo.annotations.push(a);
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
            } else if key == VK_PRIOR || key == VK_NEXT {
                if let Some(demo) = state.demo.as_mut() {
                    if let DemoBackground::Slides { images, index } = &mut demo.background {
                        if !images.is_empty() {
                            if key == VK_NEXT {
                                *index = (*index + 1) % images.len();
                            } else {
                                *index = (*index + images.len() - 1) % images.len();
                            }
                            InvalidateRect(hwnd, null(), 0);
                        }
                    }
                }
            }
            0
        }
        WM_DESTROY => 0,
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

pub(crate) unsafe fn demo_toolbar_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let theme = state.current_theme();
            if let Some(demo) = state.demo.as_ref() {
                let layout = demo_toolbar_layout();
                draw_toolbar_gdi(
                    dc,
                    0,
                    0,
                    layout,
                    &DEMO_TOOLBAR_ACTIONS,
                    demo.tool,
                    demo.toolbar_hover,
                    !demo.annotations.is_empty(),
                    !demo.redo_annotations.is_empty(),
                    false,
                    demo.passthrough,
                    &theme,
                );
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_LBUTTONDOWN => {
            let pt = ((lparam as i16) as i32, ((lparam >> 16) as i16) as i32);
            let layout = demo_toolbar_layout();
            if let Some(idx) = toolbar_hit_test((0, 0), pt, layout) {
                if DEMO_TOOLBAR_ACTIONS.get(idx) == Some(&ToolbarAction::MoveToolbar) {
                    let mut cursor: POINT = zeroed();
                    let mut rc: RECT = zeroed();
                    if GetCursorPos(&mut cursor) != 0 && GetWindowRect(hwnd, &mut rc) != 0 {
                        if let Some(demo) = state.demo.as_mut() {
                            demo.toolbar_drag = Some((
                                ScreenPoint { x: cursor.x, y: cursor.y },
                                ScreenPoint { x: rc.left, y: rc.top },
                            ));
                        }
                        SetCapture(hwnd);
                    }
                    return 0;
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            if let Some((start, origin)) = state.demo.as_ref().and_then(|d| d.toolbar_drag) {
                let mut cursor: POINT = zeroed();
                if GetCursorPos(&mut cursor) != 0 {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        origin.x + cursor.x - start.x,
                        origin.y + cursor.y - start.y,
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
                    );
                }
                return 0;
            }
            let pt = ((lparam as i16) as i32, ((lparam >> 16) as i16) as i32);
            let next = toolbar_hit_test((0, 0), pt, demo_toolbar_layout());
            if let Some(demo) = state.demo.as_mut() {
                if demo.toolbar_hover != next {
                    demo.toolbar_hover = next;
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_LBUTTONUP => {
            if state.demo.as_mut().and_then(|d| d.toolbar_drag.take()).is_some() {
                ReleaseCapture();
                return 0;
            }
            let pt = ((lparam as i16) as i32, ((lparam >> 16) as i16) as i32);
            let action = toolbar_hit_test((0, 0), pt, demo_toolbar_layout())
                .and_then(|i| DEMO_TOOLBAR_ACTIONS.get(i).copied());
            match action {
                Some(ToolbarAction::DemoPassThrough) => toggle_demo_passthrough(state),
                Some(ToolbarAction::DemoBackground) => {
                    if let Some(demo) = state.demo.as_mut() {
                        demo.bg_menu_open = !demo.bg_menu_open;
                        demo.passthrough = false;
                    }
                    apply_demo_passthrough(state);
                    if let Some(demo) = state.demo.as_ref() {
                        InvalidateRect(demo.canvas_hwnd, null(), 0);
                    }
                }
                Some(ToolbarAction::Tool(tool)) => {
                    if let Some(demo) = state.demo.as_mut() {
                        demo.tool = tool;
                        demo.passthrough = false;
                        demo.bg_menu_open = false;
                    }
                    apply_demo_passthrough(state);
                    if let Some(demo) = state.demo.as_ref() {
                        if !demo.toolbar_hwnd.is_null() {
                            InvalidateRect(demo.toolbar_hwnd, null(), 0);
                        }
                        InvalidateRect(demo.canvas_hwnd, null(), 0);
                    }
                }
                Some(ToolbarAction::Undo) => {
                    if let Some(demo) = state.demo.as_mut() {
                        if let Some(a) = demo.annotations.pop() {
                            demo.redo_annotations.push(a);
                        }
                        InvalidateRect(demo.canvas_hwnd, null(), 0);
                        if !demo.toolbar_hwnd.is_null() {
                            InvalidateRect(demo.toolbar_hwnd, null(), 0);
                        }
                    }
                }
                Some(ToolbarAction::DemoClear) => {
                    if let Some(demo) = state.demo.as_mut() {
                        demo.annotations.clear();
                        demo.redo_annotations.clear();
                        demo.number_next = 1;
                        InvalidateRect(demo.canvas_hwnd, null(), 0);
                        if !demo.toolbar_hwnd.is_null() {
                            InvalidateRect(demo.toolbar_hwnd, null(), 0);
                        }
                    }
                }
                Some(ToolbarAction::Cancel) => stop_demo(state),
                _ => {}
            }
            0
        }
        WM_DESTROY => 0,
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

pub(crate) unsafe fn toggle_demo_passthrough(state: &mut UiState) {
    let mut is_recording_mode = false;
    let mut new_pt = true;
    if let Some(demo) = state.demo.as_mut() {
        demo.passthrough = !demo.passthrough;
        new_pt = demo.passthrough;
        is_recording_mode = demo.recording_mode;
    }
    if is_recording_mode {
        state.recording_passthrough = new_pt;
    }
    apply_demo_passthrough(state);
    if is_recording_mode {
        super::recording_ui::sync_toolbar_layout(state);
    } else if let Some(demo) = state.demo.as_ref() {
        if !demo.toolbar_hwnd.is_null() {
            InvalidateRect(demo.toolbar_hwnd, null(), 0);
        }
    }
}

unsafe fn demo_bg_menu_layout(demo: &DemoState) -> crate::toolbar::ToolbarMenuLayout {
    let mut rc: RECT = zeroed();
    let _ = GetWindowRect(demo.toolbar_hwnd, &mut rc);
    let mut l = menu_layout(
        (rc.left - demo.screen_rect.left + 72, rc.bottom - demo.screen_rect.top + 8),
        ToolbarMenu::DemoBg,
    );
    l.top = rc.bottom - demo.screen_rect.top + 8;
    l
}

unsafe fn apply_demo_background_choice(state: &mut UiState, idx: usize) {
    let bg = match idx {
        0 => DemoBackground::Transparent,
        1 => DemoBackground::Solid(0x00FA_FAFA), // Clean whiteboard
        2 => DemoBackground::Solid(0x001E_1E1F), // Dark studio
        3 => DemoBackground::Solid(0x0037_4A2B), // Classic blackboard green
        4 => {
            let slides = pick_slide_images(state.hwnd);
            if slides.is_empty() {
                return;
            }
            DemoBackground::Slides {
                images: slides,
                index: 0,
            }
        }
        _ => return,
    };
    if let Some(demo) = state.demo.as_mut() {
        let is_trans = matches!(bg, DemoBackground::Transparent);
        demo.background = bg;
        if is_trans {
            SetLayeredWindowAttributes(demo.canvas_hwnd, TRANSPARENT_KEY, 255, LWA_COLORKEY);
        } else {
            SetLayeredWindowAttributes(demo.canvas_hwnd, 0, 255, LWA_ALPHA);
        }
    }
    apply_demo_passthrough(state);
    if let Some(demo) = state.demo.as_ref() {
        InvalidateRect(demo.canvas_hwnd, null(), 0);
    }
}

fn pick_slide_images(owner: HWND) -> Vec<(usize, usize, Vec<u8>)> {
    let filter = wide("图片幻灯片 (*.png;*.jpg;*.jpeg;*.webp)\0*.png;*.jpg;*.jpeg;*.webp\0");
    let title = wide("选择演示背景图片（可多选，PgUp/PgDn 翻页）");
    let mut buf = vec![0u16; 8192];
    let mut ofn: OPENFILENAMEW = unsafe { zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_ALLOWMULTISELECT;
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return Vec::new();
    }
    let mut parts = Vec::new();
    let mut start = 0;
    for i in 0..buf.len() {
        if buf[i] == 0 {
            if i == start {
                break;
            }
            if let Ok(s) = String::from_utf16(&buf[start..i]) {
                parts.push(s);
            }
            start = i + 1;
        }
    }
    let paths: Vec<String> = if parts.len() <= 1 {
        parts
    } else {
        let dir = &parts[0];
        parts[1..].iter().map(|f| format!("{dir}\\{f}")).collect()
    };
    let mut loaded = Vec::new();
    for p in paths {
        if let Ok(img) = image::open(&p) {
            let rgba = img.to_rgba8();
            loaded.push((rgba.width() as usize, rgba.height() as usize, rgba.into_raw()));
        }
    }
    loaded
}

unsafe fn paint_demo_canvas(hwnd: HWND, state: &mut UiState) {
    let mut ps: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    let w = (client.right - client.left).max(1);
    let h = (client.bottom - client.top).max(1);
    let theme = state.current_theme();

    if let Some(demo) = state.demo.as_mut() {
        // Double-buffered offscreen DC to eliminate flicker & reduce DWM stalls
        if demo.back_buffer.is_none() {
            let ref_dc = GetDC(hwnd);
            if !ref_dc.is_null() {
                demo.back_buffer = create_gdi_surface(ref_dc, w, h);
                ReleaseDC(hwnd, ref_dc);
            }
        }
        let target_dc = demo.back_buffer.as_ref().map(|s| s.dc).unwrap_or(dc);

        match &demo.background {
            DemoBackground::Transparent => {
                let bg = CreateSolidBrush(TRANSPARENT_KEY);
                FillRect(target_dc, &client, bg);
                DeleteObject(bg as _);
            }
            DemoBackground::Solid(col) => {
                let bg = CreateSolidBrush(*col);
                FillRect(target_dc, &client, bg);
                DeleteObject(bg as _);
            }
            DemoBackground::Slides { images, index } => {
                let bg = CreateSolidBrush(0x0016_1618);
                FillRect(target_dc, &client, bg);
                DeleteObject(bg as _);
                if let Some((iw, ih, pixels)) = images.get(*index) {
                    let scale =
                        (w as f64 / (*iw).max(1) as f64).min(h as f64 / (*ih).max(1) as f64);
                    let dw = ((*iw as f64 * scale).round() as i32).max(1);
                    let dh = ((*ih as f64 * scale).round() as i32).max(1);
                    draw_rgba_preview(
                        target_dc,
                        pixels,
                        *iw as i32,
                        *ih as i32,
                        (w - dw) / 2,
                        (h - dh) / 2,
                        dw,
                        dh,
                    );
                }
            }
        }

        let dummy_screen = ScreenCapture {
            x: demo.screen_rect.left,
            y: demo.screen_rect.top,
            width: w,
            height: h,
            pixels: Vec::new(),
            dimmed_pixels: Vec::new(),
        };
        for ann in &demo.annotations {
            draw_single_annotation_gdi(
                target_dc,
                ann,
                &dummy_screen,
                demo.screen_rect.left,
                demo.screen_rect.top,
            );
        }
        if let (Some(start), Some(end)) = (demo.drag_start, demo.current_pos) {
            let rect = ScreenRect::normalized(start, end);
            let preview = match demo.tool {
                Tool::Rectangle => Some(Annotation::Rect {
                    rect,
                    color: demo.color,
                    width: demo.stroke_width,
                    style: StrokeStyle::Solid,
                    filled: false,
                    rounded: false,
                }),
                Tool::Ellipse => Some(Annotation::Ellipse {
                    rect,
                    color: demo.color,
                    width: demo.stroke_width,
                    style: StrokeStyle::Solid,
                    filled: false,
                }),
                Tool::Arrow => Some(Annotation::Arrow {
                    start,
                    end,
                    color: demo.color,
                    width: demo.stroke_width,
                    style: StrokeStyle::Solid,
                    variant: ArrowVariant::Single,
                }),
                _ => None,
            };
            if let Some(p) = preview {
                draw_single_annotation_gdi(
                    target_dc,
                    &p,
                    &dummy_screen,
                    demo.screen_rect.left,
                    demo.screen_rect.top,
                );
            }
        }

        if demo.bg_menu_open {
            let menu_l = demo_bg_menu_layout(demo);
            draw_toolbar_menu_gdi(
                target_dc,
                menu_l,
                ToolbarMenu::DemoBg,
                None,
                demo.bg_menu_hover,
                &theme,
            );
        }

        if let Some(buf) = demo.back_buffer.as_ref() {
            BitBlt(dc, 0, 0, w, h, buf.dc, 0, 0, SRCCOPY);
        }
    }

    EndPaint(hwnd, &ps);
}
