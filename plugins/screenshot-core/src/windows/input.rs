use super::editor::open_editor;
use super::output::{copy_capture, finish, finish_with_dialog, pin_last};
use super::scroll_ui::{open_scroll_mode_menu, start_scroll};
use super::toolbar_paint::{toolbar_layout, toolbar_position};
use super::{cancel, invalidate, UiState};
use crate::model::{Annotation, ScreenPoint, ScreenRect, StrokeStyle};
use crate::recorder::AnimationFormat;
use crate::scroll::MatchAxis;
use crate::session::{CaptureSession, ResizeEdges};
use crate::toolbar::{
    hit_test as toolbar_hit_test, menu_for_action, menu_hit_test, menu_layout,
    recording_setup_hit_test, recording_setup_layout, secondary_bar_hit_test,
    secondary_bar_layout, toolbar_action, tool_palette_action,
    tool_palette_layout_for_screen_width, tool_palette_position, AnimationFormatKey,
    RecordingSetupAction, SecondaryBarItem,
    SecondaryBarLayout, Tool, ToolbarAction, ToolbarLayout, ToolbarMenu, COLOR_SWATCHES,
    MOSAIC_SIZES, STROKE_SIZES, TEXT_SIZES,
};

use std::mem::zeroed;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, IsIconic, IsWindowVisible, LoadCursorW, SetCursor, IDC_ARROW,
    IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE,
    IDC_SIZEWE,
};

pub(super) const VK_A: u32 = 0x41;
pub(super) const VK_E: u32 = 0x45;
pub(super) const VK_H: u32 = 0x48;
pub(super) const VK_I: u32 = 0x49;
pub(super) const VK_L: u32 = 0x4c;
pub(super) const VK_N: u32 = 0x4e;
pub(super) const VK_R: u32 = 0x52;
pub(super) const VK_P: u32 = 0x50;
pub(super) const VK_T: u32 = 0x54;
pub(super) const VK_Z: u32 = 0x5a;
pub(super) const VK_ESCAPE_CODE: u32 = 0x1b;
pub(super) const VK_RETURN_CODE: u32 = 0x0d;
pub(super) const VK_S_CODE: u32 = 0x53;
pub(super) const VK_CONTROL_CODE: i32 = 0x11;

pub(super) fn selection_resize_edges(rect: ScreenRect, point: ScreenPoint) -> Option<ResizeEdges> {
    const TOLERANCE: i32 = 8;
    let near_left = (point.x - rect.left).abs() <= TOLERANCE;
    let near_right = (point.x - rect.right).abs() <= TOLERANCE;
    let near_top = (point.y - rect.top).abs() <= TOLERANCE;
    let near_bottom = (point.y - rect.bottom).abs() <= TOLERANCE;
    let in_horizontal = point.x >= rect.left - TOLERANCE && point.x <= rect.right + TOLERANCE;
    let in_vertical = point.y >= rect.top - TOLERANCE && point.y <= rect.bottom + TOLERANCE;
    let edges = ResizeEdges {
        left: near_left && in_vertical,
        top: near_top && in_horizontal,
        right: near_right && in_vertical,
        bottom: near_bottom && in_horizontal,
    };
    (edges.left || edges.top || edges.right || edges.bottom).then_some(edges)
}

pub(super) fn toolbar_origin(capture: &CaptureSession) -> (i32, i32) {
    capture
        .toolbar_origin
        .map(|point| (point.x, point.y))
        .unwrap_or_else(|| {
            if let Some(selection) = capture.selection {
                toolbar_position(selection, capture.screen_rect)
            } else {
                (capture.screen_rect.left + 8, capture.screen_rect.top + 8)
            }
        })
}

pub(super) fn toolbar_button_at(
    origin: (i32, i32),
    point: ScreenPoint,
    layout: ToolbarLayout,
) -> Option<usize> {
    toolbar_hit_test(origin, (point.x, point.y), layout)
}

pub(super) fn clamp_toolbar_origin(
    point: ScreenPoint,
    screen: ScreenRect,
    layout: ToolbarLayout,
) -> ScreenPoint {
    let min_x = screen.left + 8;
    let min_y = screen.top + 8;
    let max_x = (screen.right - layout.width - 8).max(min_x);
    let max_y = (screen.bottom - layout.height - 8).max(min_y);
    ScreenPoint {
        x: point.x.clamp(min_x, max_x),
        y: point.y.clamp(min_y, max_y),
    }
}

pub(super) fn tool_palette_layout_for(capture: &CaptureSession) -> ToolbarLayout {
    tool_palette_layout_for_screen_width(capture.screen_rect.width() as i32)
}

pub(super) fn tool_palette_origin(capture: &CaptureSession) -> Option<(i32, i32)> {
    let selection = capture.selection?;
    Some(tool_palette_position(
        toolbar_origin(capture),
        toolbar_layout(capture.screen_rect),
        tool_palette_layout_for(capture),
        capture.screen_rect,
        selection,
    ))
}

pub(super) fn tool_palette_button_at(
    capture: &CaptureSession,
    point: ScreenPoint,
) -> Option<usize> {
    if !capture.tool_palette_open {
        return None;
    }
    toolbar_hit_test(
        tool_palette_origin(capture)?,
        (point.x, point.y),
        tool_palette_layout_for(capture),
    )
}

pub(super) fn tool_palette_contains(capture: &CaptureSession, point: ScreenPoint) -> bool {
    let Some(origin) = tool_palette_origin(capture) else {
        return false;
    };
    let layout = tool_palette_layout_for(capture);
    point.x >= origin.0
        && point.x < origin.0 + layout.width
        && point.y >= origin.1
        && point.y < origin.1 + layout.height
}

pub(super) fn toolbar_menu_layout_for(
    capture: &CaptureSession,
    menu: ToolbarMenu,
) -> crate::toolbar::ToolbarMenuLayout {
    let origin = toolbar_origin(capture);
    let bar = toolbar_layout(capture.screen_rect);
    let mut layout = menu_layout(
        origin,
        bar,
        menu,
        capture.tool,
    );
    const MARGIN: i32 = 8;
    let screen = capture.screen_rect;
    let toolbar_bottom = origin.1 + bar.height;
    let min_top = screen.top + MARGIN;
    let max_top = (screen.bottom - layout.height() - MARGIN).max(min_top);
    let below_top = toolbar_bottom + MARGIN;
    let above_top = origin.1 - layout.height() - MARGIN;
    let below_fits = below_top <= max_top;
    let above_fits = above_top >= min_top;

    let prefer_below = capture.selection.map_or(true, |selection| {
        if origin.1 >= selection.bottom {
            true
        } else if toolbar_bottom <= selection.top {
            false
        } else {
            (screen.bottom - toolbar_bottom) >= (origin.1 - screen.top)
        }
    });

    let desired_top = if prefer_below && below_fits {
        below_top
    } else if !prefer_below && above_fits {
        above_top
    } else if below_fits {
        below_top
    } else if above_fits {
        above_top
    } else if screen.bottom - toolbar_bottom >= origin.1 - screen.top {
        max_top
    } else {
        min_top
    };
    layout.top = desired_top.clamp(min_top, max_top);
    layout.left = layout.left.clamp(
        screen.left + MARGIN,
        (screen.right - layout.width - MARGIN).max(screen.left + MARGIN),
    );
    layout
}

pub(super) fn toolbar_menu_hit(
    capture: &CaptureSession,
    point: ScreenPoint,
) -> Option<(ToolbarMenu, usize)> {
    let menu = capture.menu?;
    let layout = toolbar_menu_layout_for(capture, menu);
    let index = menu_hit_test(layout, (point.x, point.y))?;
    Some((menu, index))
}

pub(super) fn secondary_bar_layout_for(capture: &CaptureSession) -> Option<SecondaryBarLayout> {
    if capture.selection.is_none() || capture.tool_palette_open {
        return None;
    }
    secondary_bar_layout(
        toolbar_origin(capture),
        capture.tool,
        capture.screen_rect,
        capture.selection?,
    )
}

pub(super) fn secondary_bar_hit(
    capture: &CaptureSession,
    point: ScreenPoint,
) -> Option<SecondaryBarItem> {
    if capture.selection.is_none() {
        return None;
    }
    let layout = secondary_bar_layout_for(capture)?;
    secondary_bar_hit_test(layout, (point.x, point.y))
}

pub(super) fn determine_cursor(capture: &CaptureSession, point: ScreenPoint) -> *const u16 {
    // 1. 拖拽移动 / 缩放中状态
    if capture.toolbar_drag.is_some() {
        return IDC_SIZEALL;
    }
    if let Some((_, _, edges)) = capture.selection_resize {
        if (edges.left && edges.top) || (edges.right && edges.bottom) {
            return IDC_SIZENWSE;
        }
        if (edges.right && edges.top) || (edges.left && edges.bottom) {
            return IDC_SIZENESW;
        }
        if edges.top || edges.bottom {
            return IDC_SIZENS;
        }
        if edges.left || edges.right {
            return IDC_SIZEWE;
        }
    }
    if capture.selection_move.is_some() {
        return IDC_SIZEALL;
    }
    if capture.drag.is_some() {
        if capture.selection.is_none() {
            return IDC_CROSS;
        }
        return match capture.tool {
            Tool::Text => IDC_IBEAM,
            Tool::Select => IDC_SIZEALL,
            _ => IDC_CROSS,
        };
    }

    // 2. Annotation tool palette buttons.
    if tool_palette_button_at(capture, point).is_some() {
        return IDC_HAND;
    }

    // 3. 二级属性栏悬停
    if capture.selection.is_some() && secondary_bar_hit(capture, point).is_some() {
        return IDC_HAND;
    }

    // 4. 下拉菜单悬停
    if capture.selection.is_some() && toolbar_menu_hit(capture, point).is_some() {
        return IDC_HAND;
    }

    // 5. 工具栏按钮悬停
    if capture.selection.is_some() {
        let tb_origin = toolbar_origin(capture);
        let tb_layout = toolbar_layout(capture.screen_rect);
        if let Some(index) = toolbar_button_at(tb_origin, point, tb_layout) {
            if let Some(ToolbarAction::MoveToolbar) = toolbar_action(index) {
                return IDC_SIZEALL;
            }
            return IDC_HAND;
        }
    }

    // 6. 选区控制点 (Handles) 悬停
    if let Some(selection) = capture.selection {
        if let Some(edges) = selection_resize_edges(selection, point) {
            if (edges.left && edges.top) || (edges.right && edges.bottom) {
                return IDC_SIZENWSE;
            }
            if (edges.right && edges.top) || (edges.left && edges.bottom) {
                return IDC_SIZENESW;
            }
            if edges.top || edges.bottom {
                return IDC_SIZENS;
            }
            if edges.left || edges.right {
                return IDC_SIZEWE;
            }
        }

        // 7. 选区内部悬停
        if selection.contains(point) {
            return match capture.tool {
                Tool::Select => IDC_SIZEALL,
                Tool::Text => IDC_IBEAM,
                _ => IDC_CROSS,
            };
        }
    } else {
        // 未框选状态：十字准星指示可拖拽框选
        return IDC_CROSS;
    }

    IDC_ARROW
}

pub(super) unsafe fn update_capture_cursor(capture: &CaptureSession, point: ScreenPoint) {
    let cursor_id = determine_cursor(capture, point);
    let cursor = LoadCursorW(null_mut(), cursor_id);
    if !cursor.is_null() {
        SetCursor(cursor);
    }
}

pub(super) unsafe fn update_session_cursor(state: &UiState, point: ScreenPoint) {
    if let Some(capture) = state.capture.as_ref() {
        update_capture_cursor(capture, point);
    } else {
        let cursor = LoadCursorW(null_mut(), IDC_ARROW);
        if !cursor.is_null() {
            SetCursor(cursor);
        }
    }
}

pub(super) fn open_toolbar_menu(state: &mut UiState, menu: ToolbarMenu) {
    if let Some(capture) = state.capture.as_mut() {
        capture.open_menu(menu);
        invalidate(state);
    }
}

pub(super) fn apply_toolbar_menu(state: &mut UiState, menu: ToolbarMenu, index: usize) {
    if menu == ToolbarMenu::Recognition {
        if let Some(capture) = state.capture.as_mut() {
            capture.close_menu();
        }
        invalidate(state);
        match index {
            0 => super::ocr_ui::start_qr_recognition(state),
            1 => super::ocr_ui::start_text_extraction(state),
            _ => {}
        }
        return;
    }
    if menu == ToolbarMenu::RecordingFps {
        let _ = index;
        return;
    }
    if menu == ToolbarMenu::RecordingExport {
        if let Some(capture) = state.capture.as_mut() {
            capture.close_menu();
        }
        let format = match index {
            0 => AnimationFormat::Mp4,
            1 => AnimationFormat::Gif,
            2 => AnimationFormat::Webp,
            _ => return,
        };
        super::export_recording_from_editor(state, format);
        return;
    }
    let scroll_axis = match (menu, index) {
        (ToolbarMenu::ScrollMode, 0) => Some(MatchAxis::Vertical),
        (ToolbarMenu::ScrollMode, 1) => Some(MatchAxis::Horizontal),
        _ => None,
    };
    if let Some(capture) = state.capture.as_mut() {
        match menu {
            ToolbarMenu::Color => {
                if let Some(color) = COLOR_SWATCHES.get(index) {
                    capture.annotation_color = *color;
                }
            }
            ToolbarMenu::Size => {
                if capture.tool == Tool::Text {
                    if let Some(size) = crate::toolbar::TEXT_SIZES.get(index) {
                        capture.text_size = *size;
                    }
                } else if capture.tool == Tool::Mosaic {
                    if let Some(size) = crate::toolbar::MOSAIC_SIZES.get(index) {
                        capture.brush_size = *size;
                    }
                } else if let Some(size) = crate::toolbar::STROKE_SIZES.get(index) {
                    capture.stroke_width = *size;
                }
            }
            ToolbarMenu::Style => {
                capture.stroke_style = match index {
                    0 => StrokeStyle::Solid,
                    1 => StrokeStyle::Dashed,
                    2 => StrokeStyle::Dotted,
                    _ => capture.stroke_style,
                };
            }
            ToolbarMenu::ScrollMode => capture.scroll_axis = scroll_axis,
            ToolbarMenu::Recognition => unreachable!(),
            ToolbarMenu::RecordingFps => unreachable!(),
            ToolbarMenu::RecordingExport => unreachable!(),
        }
        capture.close_menu();
    }
    if let Some(axis) = scroll_axis {
        start_scroll(state, axis);
    } else {
        invalidate(state);
    }
}

pub(super) fn mouse_down(state: &mut UiState, point: ScreenPoint) {
    let recording_setup = state.capture.as_ref().and_then(|capture| {
        (capture.menu == Some(ToolbarMenu::RecordingFps))
            .then(|| capture.selection.map(|selection| (selection, capture.screen_rect)))
            .flatten()
    });
    if let Some((selection, screen)) = recording_setup {
        let layout = recording_setup_layout(selection, screen);
        match recording_setup_hit_test(layout, (point.x, point.y)) {
            Some(RecordingSetupAction::Close) => {
                if let Some(capture) = state.capture.as_mut() {
                    capture.close_menu();
                }
                invalidate(state);
            }
            Some(RecordingSetupAction::Start) => {
                let fps = super::configured_recording_fps(
                    &state.settings,
                    state.recording_setup_format,
                );
                if let Some(capture) = state.capture.as_mut() {
                    capture.close_menu();
                }
                super::start_recording_from_toolbar(state, fps);
            }
            Some(RecordingSetupAction::Format(format)) => {
                state.recording_setup_format = match format {
                    AnimationFormatKey::Mp4 => AnimationFormat::Mp4,
                    AnimationFormatKey::Gif => AnimationFormat::Gif,
                    AnimationFormatKey::Webp => AnimationFormat::Webp,
                };
                invalidate(state);
            }
            Some(RecordingSetupAction::ToggleCursor) => {
                state.recording_show_cursor = !state.recording_show_cursor;
                invalidate(state);
            }
            Some(RecordingSetupAction::Ignore) | None => {}
        }
        // The setup card is modal to the selection flow. Outside clicks must not
        // dismiss it or be interpreted as selection changes.
        return;
    }
    let open_palette = state
        .capture
        .as_ref()
        .is_some_and(|capture| capture.tool_palette_open);
    if open_palette {
        let palette_state = state.capture.as_ref().map(|capture| {
            let palette_hit = tool_palette_button_at(capture, point);
            let palette_contains = tool_palette_contains(capture, point);
            let clicked_action = toolbar_button_at(
                toolbar_origin(capture),
                point,
                toolbar_layout(capture.screen_rect),
            )
            .and_then(toolbar_action);
            (palette_hit, palette_contains, clicked_action)
        });
        if let Some((palette_hit, palette_contains, clicked_action)) = palette_state {
            if let Some(ToolbarAction::Tool(tool)) = palette_hit.and_then(tool_palette_action) {
                if let Some(capture) = state.capture.as_mut() {
                    capture.select_tool(tool);
                }
                invalidate(state);
                return;
            }
            if palette_contains {
                return;
            }
            if let Some(capture) = state.capture.as_mut() {
                capture.tool_palette_open = false;
                capture.tool_palette_hover = None;
                capture.toolbar_hover = None;
                capture.secondary_hover = None;
            }
            invalidate(state);
            if clicked_action.is_none() || clicked_action == Some(ToolbarAction::Tools) {
                return;
            }
        }
    }
    let secondary_click = state
        .capture
        .as_ref()
        .and_then(|capture| secondary_bar_hit(capture, point));
    if let Some(item) = secondary_click {
        if let Some(capture) = state.capture.as_mut() {
            match item {
                SecondaryBarItem::Size(index) => {
                    if capture.tool == Tool::Text {
                        if let Some(size) = TEXT_SIZES.get(index) {
                            capture.text_size = *size;
                            if let Some(edit_idx) = capture.editing_text {
                                if let Some(Annotation::Text(_, _, sz, _)) =
                                    capture.annotations.get_mut(edit_idx)
                                {
                                    *sz = *size;
                                }
                            }
                        }
                    } else if capture.tool == Tool::Mosaic {
                        if let Some(size) = MOSAIC_SIZES.get(index) {
                            capture.brush_size = *size;
                        }
                    } else if let Some(size) = STROKE_SIZES.get(index) {
                        capture.stroke_width = *size;
                    }
                }
                SecondaryBarItem::Color(index) => {
                    if let Some(color) = COLOR_SWATCHES.get(index) {
                        capture.annotation_color = *color;
                        if let Some(edit_idx) = capture.editing_text {
                            if let Some(Annotation::Text(_, _, _, col)) =
                                capture.annotations.get_mut(edit_idx)
                            {
                                *col = *color;
                            }
                        }
                    }
                }
            }
        }
        invalidate(state);
        return;
    }
    let menu_action = state
        .capture
        .as_ref()
        .and_then(|capture| toolbar_menu_hit(capture, point));
    if let Some((menu, index)) = menu_action {
        apply_toolbar_menu(state, menu, index);
        return;
    }
    let open_menu = state
        .capture
        .as_ref()
        .and_then(|capture| capture.menu);
    if let Some(open_menu) = open_menu {
        let clicked_action = state.capture.as_ref().and_then(|capture| {
            toolbar_button_at(
                toolbar_origin(capture),
                point,
                toolbar_layout(capture.screen_rect),
            )
            .and_then(toolbar_action)
        });
        if let Some(capture) = state.capture.as_mut() {
            capture.close_menu();
        }
        invalidate(state);
        if let Some(next_menu) = clicked_action.and_then(menu_for_action) {
            if next_menu != open_menu {
                open_toolbar_menu(state, next_menu);
            }
            return;
        }
        if clicked_action.is_none() {
            return;
        }
    }
    let action = {
        let Some(capture) = state.capture.as_mut() else {
            return;
        };
        if let Some(selection) = capture.selection {
            let toolbar = toolbar_origin(capture);
            let layout = toolbar_layout(capture.screen_rect);
            if let Some(index) = toolbar_button_at(toolbar, point, layout) {
                match toolbar_action(index) {
                    Some(ToolbarAction::MoveToolbar) => {
                        capture.toolbar_drag = Some((
                            point,
                            ScreenPoint {
                                x: toolbar.0,
                                y: toolbar.1,
                            },
                        ));
                        capture.toolbar_hover = None;
                        None
                    }
                    action => action,
                }
            } else if capture.tool == Tool::Number && selection.contains(point) {
                let number = capture.number_next;
                capture.annotations.push(Annotation::Number(
                    point,
                    number,
                    capture.annotation_color,
                ));
                capture.number_next = capture.number_next.saturating_add(1);
                capture.current_pos = Some(point);
                Some(ToolbarAction::PlaceNumber)
            } else if capture.tool != Tool::Select && selection.contains(point) {
                capture.drag = Some(point);
                capture.current_pos = Some(point);
                match capture.tool {
                    Tool::Pen => capture.annotations.push(Annotation::Pen(
                        vec![point],
                        capture.annotation_color,
                        capture.stroke_width,
                        capture.stroke_style,
                    )),
                    Tool::Mosaic => capture
                        .annotations
                        .push(Annotation::Mosaic(vec![point], capture.brush_size)),
                    _ => {}
                }
                None
            } else if state.editor.is_some() {
                None
            } else if let Some(edges) = selection_resize_edges(selection, point) {
                capture.selection_resize = Some((point, selection, edges));
                capture.current_pos = Some(point);
                None
            } else if selection.contains(point) {
                capture.selection_move = Some((point, selection));
                capture.current_pos = Some(point);
                None
            } else {
                None
            }
        } else {
            capture.drag = Some(point);
            capture.current_pos = Some(point);
            None
        }
    };
    let recording_preview = state
        .editor
        .as_ref()
        .is_some_and(|editor| editor.recording_preview);
    match action {
        Some(ToolbarAction::Tool(_)) if recording_preview => {}
        Some(ToolbarAction::Tools) if recording_preview => {}
        Some(ToolbarAction::PlaceNumber) if recording_preview => {}
        Some(ToolbarAction::Pin) if recording_preview => {}
        Some(ToolbarAction::Undo) if recording_preview => {}
        Some(ToolbarAction::Record) if recording_preview => {}
        Some(ToolbarAction::Tool(tool)) => set_tool(state, tool),
        Some(ToolbarAction::Tools) => {
            if let Some(capture) = state.capture.as_mut() {
                capture.tool_palette_open = !capture.tool_palette_open;
                capture.tool_palette_hover = None;
                capture.toolbar_hover = None;
                capture.secondary_hover = None;
                capture.close_menu();
                invalidate(state);
            }
        }
        Some(ToolbarAction::PlaceNumber) => invalidate(state),
        Some(ToolbarAction::Pin) => {
            finish(state);
            pin_last(state);
        }
        Some(ToolbarAction::Scroll) if state.editor.is_none() => open_scroll_mode_menu(state),
        Some(ToolbarAction::Scroll) => {}
        Some(ToolbarAction::Edit) if state.editor.is_none() => open_editor(state),
        Some(ToolbarAction::Edit) => {}
        Some(ToolbarAction::Undo) => undo(state),
        Some(ToolbarAction::Record) => {
            if let Some(menu) = menu_for_action(ToolbarAction::Record) {
                open_toolbar_menu(state, menu);
            }
        }
        Some(action @ ToolbarAction::Color) => {
            if let Some(menu) = menu_for_action(action) {
                open_toolbar_menu(state, menu);
            }
        }
        Some(action @ ToolbarAction::Size) => {
            if let Some(menu) = menu_for_action(action) {
                open_toolbar_menu(state, menu);
            }
        }
        Some(action @ ToolbarAction::Style) => {
            if let Some(menu) = menu_for_action(action) {
                open_toolbar_menu(state, menu);
            }
        }
        Some(ToolbarAction::Copy) if recording_preview => {
            open_toolbar_menu(state, ToolbarMenu::RecordingExport)
        }
        Some(ToolbarAction::Ocr) => open_toolbar_menu(state, ToolbarMenu::Recognition),
        Some(ToolbarAction::Copy) => copy_capture(state),
        Some(ToolbarAction::Save) if recording_preview => {
            open_toolbar_menu(state, ToolbarMenu::RecordingExport)
        }
        Some(ToolbarAction::Save) => finish_with_dialog(state),
        Some(ToolbarAction::Cancel | ToolbarAction::Finish) if recording_preview => cancel(state),
        Some(ToolbarAction::Cancel) => cancel(state),
        Some(ToolbarAction::Finish) => finish(state),
        Some(ToolbarAction::MoveToolbar) | None => {}
    }
}

pub(super) fn mouse_move(state: &mut UiState, mut point: ScreenPoint, held: bool) {
    let editor_mode = state.editor.is_some();
    let should_query_hover = !held
        && state
            .capture
            .as_ref()
            .is_some_and(|capture| capture.selection.is_none())
        && state
            .last_hover_query
            .is_none_or(|(last, at)| last != point && at.elapsed() >= Duration::from_millis(30));
    let hover = should_query_hover.then(|| {
        state.last_hover_query = Some((point, Instant::now()));
        find_hover_window(point, state.overlay, state.pin_window)
    });
    let Some(capture) = state.capture.as_mut() else {
        return;
    };
    capture.current_pos = Some(point);
    unsafe {
        update_capture_cursor(capture, point);
    }
    if let Some(hover) = hover {
        capture.hover = hover;
    }
    let mut palette_hover_changed = false;
    if !held && capture.tool_palette_open {
        let next = tool_palette_button_at(capture, point);
        let changed = capture.tool_palette_hover != next
            || capture.toolbar_hover.is_some()
            || capture.secondary_hover.is_some();
        capture.tool_palette_hover = next;
        capture.toolbar_hover = None;
        capture.secondary_hover = None;
        if tool_palette_contains(capture, point) {
            if changed {
                invalidate(state);
            }
            return;
        }
        palette_hover_changed = changed;
    }
    if !held && capture.menu.is_some() {
        let next = toolbar_menu_hit(capture, point).map(|(_, index)| index);
        let changed = capture.menu_hover != next;
        capture.menu_hover = next;
        if changed {
            invalidate(state);
        }
        return;
    }
    if held {
        if let Some((origin_point, origin_toolbar)) = capture.toolbar_drag {
            let moved = clamp_toolbar_origin(
                ScreenPoint {
                    x: origin_toolbar.x + point.x - origin_point.x,
                    y: origin_toolbar.y + point.y - origin_point.y,
                },
                capture.screen_rect,
                toolbar_layout(capture.screen_rect),
            );
            capture.toolbar_origin = Some(moved);
            capture.toolbar_hover = None;
            capture.current_pos = Some(point);
            invalidate(state);
            return;
        }
    }
    let toolbar_hover_changed = if !held {
        let next = capture.selection.and_then(|_| {
            toolbar_button_at(
                toolbar_origin(capture),
                point,
                toolbar_layout(capture.screen_rect),
            )
        });
        let changed = capture.toolbar_hover != next;
        capture.toolbar_hover = next;
        changed
    } else {
        false
    };
    let secondary_hover_changed = if !held {
        let next = capture.selection.and_then(|_| secondary_bar_hit(capture, point));
        let changed = capture.secondary_hover != next;
        capture.secondary_hover = next;
        changed
    } else {
        false
    };
    if held {
        if let Some((origin_point, origin_rect, edges)) = capture.selection_resize {
            const MIN_SIZE: i32 = 8;
            let dx = point.x - origin_point.x;
            let dy = point.y - origin_point.y;
            let mut left = origin_rect.left;
            let mut top = origin_rect.top;
            let mut right = origin_rect.right;
            let mut bottom = origin_rect.bottom;
            if edges.left {
                left = (origin_rect.left + dx).clamp(capture.screen_rect.left, right - MIN_SIZE);
            }
            if edges.top {
                top = (origin_rect.top + dy).clamp(capture.screen_rect.top, bottom - MIN_SIZE);
            }
            if edges.right {
                right = (origin_rect.right + dx).clamp(left + MIN_SIZE, capture.screen_rect.right);
            }
            if edges.bottom {
                bottom =
                    (origin_rect.bottom + dy).clamp(top + MIN_SIZE, capture.screen_rect.bottom);
            }
            capture.selection = Some(ScreenRect {
                left,
                top,
                right,
                bottom,
            });
            capture.current_pos = Some(point);
            invalidate(state);
            return;
        }
        if let Some((origin_point, origin_rect)) = capture.selection_move {
            let dx = point.x - origin_point.x;
            let dy = point.y - origin_point.y;
            let width = origin_rect.right - origin_rect.left;
            let height = origin_rect.bottom - origin_rect.top;
            let left = (origin_rect.left + dx)
                .clamp(capture.screen_rect.left, capture.screen_rect.right - width);
            let top = (origin_rect.top + dy)
                .clamp(capture.screen_rect.top, capture.screen_rect.bottom - height);
            capture.selection = Some(ScreenRect {
                left,
                top,
                right: left + width,
                bottom: top + height,
            });
            capture.current_pos = Some(point);
            invalidate(state);
            return;
        }
    }
    if editor_mode && held {
        if let Some(selection) = capture.selection {
            point.x = point.x.clamp(selection.left, selection.right);
            point.y = point.y.clamp(selection.top, selection.bottom);
        }
    }
    capture.current_pos = Some(point);
    if !held {
        if should_query_hover
            || toolbar_hover_changed
            || secondary_hover_changed
            || palette_hover_changed
        {
            invalidate(state);
        }
        return;
    }
    match capture.annotations.last_mut() {
        Some(Annotation::Pen(points, _, _, _))
            if capture.tool == Tool::Pen && capture.selection.is_some() =>
        {
            points.push(point);
        }
        Some(Annotation::Mosaic(points, _))
            if capture.tool == Tool::Mosaic && capture.selection.is_some() =>
        {
            points.push(point);
        }
        _ => {}
    }
    invalidate(state);
}

pub(super) fn mouse_up(state: &mut UiState, mut point: ScreenPoint) {
    if state.editor.is_some() {
        if let Some(selection) = state.capture.as_ref().and_then(|capture| capture.selection) {
            point.x = point.x.clamp(selection.left, selection.right);
            point.y = point.y.clamp(selection.top, selection.bottom);
        }
    }
    let Some(capture) = state.capture.as_mut() else {
        return;
    };
    if capture.toolbar_drag.take().is_some() {
        capture.current_pos = None;
        invalidate(state);
        return;
    }
    if capture.selection_resize.take().is_some() {
        capture.current_pos = None;
        invalidate(state);
        return;
    }
    if capture.selection_move.take().is_some() {
        capture.current_pos = None;
        invalidate(state);
        return;
    }
    let Some(start) = capture.drag.take() else {
        return;
    };
    capture.current_pos = None;
    let mut selection_completed = false;
    if capture.selection.is_none() {
        let rect = ScreenRect::normalized(start, point);
        let selected = if rect.width() <= 5 && rect.height() <= 5 {
            capture
                .hover
                .filter(|hover| hover.width() > 5 && hover.height() > 5)
        } else {
            Some(rect)
        };
        if let Some(rect) = selected
            .and_then(|rect| rect.clipped_to(capture.screen_rect))
            .filter(|rect| rect.width() > 4 && rect.height() > 4)
        {
            capture.selection = Some(rect);
            selection_completed = true;
            if capture.scroll.is_some() {
                open_scroll_mode_menu(state);
            }
        }
    } else {
        let color = capture.annotation_color;
        let width = capture.stroke_width;
        let style = capture.stroke_style;
        match capture.tool {
            Tool::Rectangle => {
                let rect = ScreenRect::normalized(start, point);
                if rect.width() > 2 && rect.height() > 2 {
                    capture
                        .annotations
                        .push(Annotation::Rect(rect, color, width, style));
                }
            }
            Tool::Ellipse => {
                let rect = ScreenRect::normalized(start, point);
                if rect.width() > 2 && rect.height() > 2 {
                    capture
                        .annotations
                        .push(Annotation::Ellipse(rect, color, width, style));
                }
            }
            Tool::Arrow => {
                if (start.x - point.x).abs() > 3 || (start.y - point.y).abs() > 3 {
                    capture
                        .annotations
                        .push(Annotation::Arrow(start, point, color, width, style));
                }
            }
            Tool::Line => {
                if (start.x - point.x).abs() > 3 || (start.y - point.y).abs() > 3 {
                    capture
                        .annotations
                        .push(Annotation::Line(start, point, color, width, style));
                }
            }
            Tool::Highlight => {
                if (start.x - point.x).abs() > 3 || (start.y - point.y).abs() > 3 {
                    capture.annotations.push(Annotation::Highlight(
                        start,
                        point,
                        [color[0], color[1], color[2], 105],
                        width.saturating_mul(6).max(10),
                    ));
                }
            }
            Tool::Mosaic => {}
            Tool::Text => {
                capture.annotations.push(Annotation::Text(
                    start,
                    String::new(),
                    capture.text_size,
                    color,
                ));
                capture.editing_text = Some(capture.annotations.len() - 1);
            }
            _ => {}
        }
    }
    invalidate(state);
    if selection_completed && state.recording_requested_fps.is_some() {
        super::start_recording_selected_region(state);
    }
}

pub(super) fn char_input(state: &mut UiState, value: u16) {
    if let Some(capture) = state.capture.as_mut() {
        if capture.append_text_char(value) {
            invalidate(state);
        }
    }
}

pub(super) fn key_down(state: &mut UiState, key: u32) {
    if state
        .editor
        .as_ref()
        .is_some_and(|editor| editor.recording_preview)
    {
        if key == VK_ESCAPE_CODE {
            cancel(state);
        }
        return;
    }
    if key == VK_ESCAPE_CODE {
        cancel(state);
        return;
    }
    if state.capture.is_none() {
        return;
    }
    if state
        .capture
        .as_ref()
        .is_some_and(|capture| capture.editing_text.is_some())
        && key == VK_RETURN_CODE
    {
        if let Some(capture) = state.capture.as_mut() {
            capture.finish_text_editing();
        }
        invalidate(state);
        return;
    }
    if key == VK_Z && unsafe { GetKeyState(VK_CONTROL_CODE) } < 0 {
        undo(state);
        return;
    }
    let has_selection = state
        .capture
        .as_ref()
        .and_then(|capture| capture.selection)
        .is_some();
    if has_selection && key == VK_RETURN_CODE {
        finish(state);
        return;
    }
    if has_selection && key == 0x43 && unsafe { GetKeyState(VK_CONTROL_CODE) } < 0 {
        copy_capture(state);
        return;
    }
    if has_selection && key == VK_S_CODE && unsafe { GetKeyState(VK_CONTROL_CODE) } < 0 {
        finish_with_dialog(state);
        return;
    }
    match key {
        VK_R => set_tool(state, Tool::Rectangle),
        VK_E => set_tool(state, Tool::Ellipse),
        VK_H => set_tool(state, Tool::Highlight),
        VK_I => set_tool(state, Tool::Line),
        VK_L => open_scroll_mode_menu(state),
        VK_N => set_tool(state, Tool::Number),
        VK_P => set_tool(state, Tool::Pen),
        VK_T => set_tool(state, Tool::Text),
        _ => {}
    }
}

pub(super) fn set_tool(state: &mut UiState, tool: Tool) {
    if let Some(capture) = state.capture.as_mut() {
        if capture.select_tool(tool) {
            invalidate(state);
        }
    }
}

pub(super) fn undo(state: &mut UiState) {
    if let Some(capture) = state.capture.as_mut() {
        capture.undo();
        invalidate(state);
    }
}

struct HoverQuery {
    point: ScreenPoint,
    exclude: [HWND; 3],
    best: Option<ScreenRect>,
}

unsafe extern "system" fn hover_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let query = &mut *(lparam as *mut HoverQuery);
    if query.exclude.contains(&hwnd) || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
        return 1;
    }
    let mut rect: RECT = zeroed();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return 1;
    }
    let candidate = ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    if !candidate.contains(query.point) {
        return 1;
    }
    let is_smaller = query
        .best
        .is_none_or(|best| candidate.width() * candidate.height() < best.width() * best.height());
    if is_smaller {
        query.best = Some(candidate);
    }
    1
}

pub(super) fn find_hover_window(
    point: ScreenPoint,
    overlay: HWND,
    pin_window: HWND,
) -> Option<ScreenRect> {
    let mut query = HoverQuery {
        point,
        exclude: [overlay, pin_window, null_mut()],
        best: None,
    };
    unsafe {
        EnumWindows(
            Some(hover_window_proc),
            &mut query as *mut HoverQuery as LPARAM,
        );
    }
    query.best
}

pub(super) fn screen_point_from_lparam(
    hwnd: HWND,
    value: LPARAM,
    editor_mode: bool,
) -> ScreenPoint {
    let mut point = POINT {
        x: (value as i16) as i32,
        y: ((value >> 16) as i16) as i32,
    };
    if editor_mode {
        return ScreenPoint {
            x: point.x,
            y: point.y,
        };
    }
    unsafe {
        if ClientToScreen(hwnd, &mut point) == 0 {
            return ScreenPoint {
                x: (value as i16) as i32,
                y: ((value >> 16) as i16) as i32,
            };
        }
    }
    ScreenPoint {
        x: point.x,
        y: point.y,
    }
}
