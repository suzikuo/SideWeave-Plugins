use super::editor::open_editor;
use super::output::{copy_capture, finish, finish_with_dialog, pin_last};
use super::scroll_ui::{open_scroll_mode_menu, show_scroll_tip, start_scroll};
use super::toolbar_paint::toolbar_position_for_layout;
use super::{cancel, invalidate, PinnedImageWindow, UiState};
use crate::model::{Annotation, ArrowVariant, ScreenPoint, ScreenRect, StrokeStyle};
use crate::recorder::AnimationFormat;
use crate::scroll::MatchAxis;
use crate::session::{CaptureSession, ResizeEdges};
use crate::toolbar::{
    hit_test as toolbar_hit_test, menu_hit_test, menu_layout, recording_setup_hit_test,
    recording_setup_layout, secondary_bar_hit_test, secondary_bar_layout, InfoCapsuleHit,
    InfoCapsuleLayout, RecordingSetupAction, SecondaryBarItem, SecondaryBarLayout, Tool,
    ToolbarAction, ToolbarLayout, ToolbarMenu, COLOR_SWATCHES, MOSAIC_SIZES, STROKE_SIZES,
    TEXT_SIZES,
};

use arboard::Clipboard;
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
pub(super) const VK_M: u32 = 0x4d;
pub(super) const VK_N: u32 = 0x4e;
pub(super) const VK_O: u32 = 0x4f;
pub(super) const VK_R: u32 = 0x52;
pub(super) const VK_P: u32 = 0x50;
pub(super) const VK_T: u32 = 0x54;
pub(super) const VK_V: u32 = 0x56;
pub(super) const VK_X: u32 = 0x58;
pub(super) const VK_Y: u32 = 0x59;
pub(super) const VK_Z: u32 = 0x5a;
pub(super) const VK_ESCAPE_CODE: u32 = 0x1b;
pub(super) const VK_RETURN_CODE: u32 = 0x0d;
pub(super) const VK_SPACE_CODE: u32 = 0x20;
pub(super) const VK_S_CODE: u32 = 0x53;
pub(super) const VK_CONTROL_CODE: i32 = 0x11;

pub(super) fn toolbar_layout_for(capture: &CaptureSession) -> ToolbarLayout {
    super::toolbar_paint::toolbar_layout_scaled(
        capture.toolbar_button_count(),
        capture.screen_rect,
        capture.ui_scale,
    )
}

pub(super) fn info_capsule_for(capture: &CaptureSession) -> Option<InfoCapsuleLayout> {
    let selection = capture.selection?;
    Some(InfoCapsuleLayout::for_selection_scaled(
        selection,
        capture.screen_rect,
        capture.ui_scale,
    ))
}

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
                toolbar_position_for_layout(
                    selection,
                    capture.screen_rect,
                    toolbar_layout_for(capture),
                )
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

pub(super) fn toolbar_menu_layout_for(
    capture: &CaptureSession,
    menu: ToolbarMenu,
) -> crate::toolbar::ToolbarMenuLayout {
    let origin = toolbar_origin(capture);
    let bar = toolbar_layout_for(capture);
    let mut layout = menu_layout(origin, menu);
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
    let selection = capture.selection?;
    secondary_bar_layout(
        toolbar_origin(capture),
        toolbar_layout_for(capture),
        capture.tool,
        capture.screen_rect,
        selection,
    )
}

pub(super) fn secondary_bar_hit(
    capture: &CaptureSession,
    point: ScreenPoint,
) -> Option<SecondaryBarItem> {
    let layout = secondary_bar_layout_for(capture)?;
    secondary_bar_hit_test(layout, (point.x, point.y))
}

pub(super) fn determine_cursor(capture: &CaptureSession, point: ScreenPoint) -> *const u16 {
    if capture.toolbar_drag.is_some() {
        return IDC_SIZEALL;
    }
    if capture.radius_dragging {
        return IDC_HAND;
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

    if let Some(cap) = info_capsule_for(capture) {
        if cap.hit_test(point.x, point.y).is_some() {
            return IDC_HAND;
        }
    }

    if capture.selection.is_some() && secondary_bar_hit(capture, point).is_some() {
        return IDC_HAND;
    }

    if capture.selection.is_some() && toolbar_menu_hit(capture, point).is_some() {
        return IDC_HAND;
    }

    if capture.selection.is_some() {
        let tb_origin = toolbar_origin(capture);
        let tb_layout = toolbar_layout_for(capture);
        if let Some(index) = toolbar_button_at(tb_origin, point, tb_layout) {
            if capture.toolbar_action(index) == Some(ToolbarAction::MoveToolbar) {
                return IDC_SIZEALL;
            }
            return IDC_HAND;
        }
    }

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
        if selection.contains(point) {
            return match capture.tool {
                Tool::Select => IDC_SIZEALL,
                Tool::Text => IDC_IBEAM,
                _ => IDC_CROSS,
            };
        }
    } else {
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
            0 => super::ocr_ui::start_text_extraction(state),
            1 => super::ocr_ui::start_in_place_translation(state),
            2 => super::ocr_ui::start_qr_recognition(state),
            _ => {}
        }
        return;
    }
    if menu == ToolbarMenu::RecordingFps {
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
        if let Some(axis) = scroll_axis {
            capture.scroll_axis = Some(axis);
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
                let fps = super::configured_recording_fps(&state.settings);
                if let Some(capture) = state.capture.as_mut() {
                    capture.close_menu();
                }
                super::start_recording_from_toolbar(state, fps);
            }
            Some(RecordingSetupAction::ToggleCursor) => {
                state.recording_show_cursor = !state.recording_show_cursor;
                invalidate(state);
            }
            Some(RecordingSetupAction::Ignore) | None => {}
        }
        return;
    }

    // 1. Info Capsule hit (Aspect lock, corner radius slider, drop shadow toggle)
    let capsule_hit = state
        .capture
        .as_ref()
        .and_then(|capture| info_capsule_for(capture)?.hit_test(point.x, point.y));
    if let Some(hit) = capsule_hit {
        if let Some(capture) = state.capture.as_mut() {
            match hit {
                InfoCapsuleHit::LockAspect => capture.aspect_locked = !capture.aspect_locked,
                InfoCapsuleHit::RadiusSlider(val) => {
                    capture.corner_radius = val;
                    capture.radius_dragging = true;
                }
                InfoCapsuleHit::ShadowToggle => capture.drop_shadow = !capture.drop_shadow,
            }
        }
        invalidate(state);
        return;
    }

    // 2. Secondary Property Bar hit
    let secondary_click = state
        .capture
        .as_ref()
        .and_then(|capture| secondary_bar_hit(capture, point));
    if let Some(item) = secondary_click {
        if let Some(capture) = state.capture.as_mut() {
            match item {
                SecondaryBarItem::Variant(variant) => {
                    capture.tool = variant;
                    capture.editing_text = None;
                }
                SecondaryBarItem::ToggleFilled => {
                    capture.shape_filled = !capture.shape_filled;
                }
                SecondaryBarItem::ToggleDashed => {
                    capture.stroke_style = if capture.stroke_style == StrokeStyle::Dashed {
                        StrokeStyle::Solid
                    } else {
                        StrokeStyle::Dashed
                    };
                }
                SecondaryBarItem::ToggleRounded => {
                    capture.shape_rounded = !capture.shape_rounded;
                }
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
                    } else if matches!(capture.tool, Tool::Mosaic | Tool::Patina) {
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

    // 3. Dropdown menu hit
    let menu_action = state
        .capture
        .as_ref()
        .and_then(|capture| toolbar_menu_hit(capture, point));
    if let Some((menu, index)) = menu_action {
        apply_toolbar_menu(state, menu, index);
        return;
    }
    if state.capture.as_ref().is_some_and(|c| c.menu.is_some()) {
        if let Some(capture) = state.capture.as_mut() {
            capture.close_menu();
        }
        invalidate(state);
        // Continue to allow clicking toolbar buttons immediately
    }

    // 4. OCR block click-to-copy when OCR overlay is active and Select tool is active
    let ocr_click = state.capture.as_ref().and_then(|capture| {
        if capture.tool != Tool::Select || capture.ocr_blocks.is_empty() || capture.show_translated_overlay {
            return None;
        }
        let sel = capture.selection?;
        let lx = point.x - sel.left;
        let ly = point.y - sel.top;
        capture
            .ocr_blocks
            .iter()
            .enumerate()
            .find(|(_, b)| lx >= b.x && lx <= b.x + b.w && ly >= b.y && ly <= b.y + b.h)
            .map(|(i, b)| (i, b.text.clone()))
    });
    if let Some((idx, text)) = ocr_click {
        if let Some(capture) = state.capture.as_mut() {
            capture.ocr_selected_block = Some(idx);
        }
        let _ = Clipboard::new().and_then(|mut cb| cb.set_text(text));
        show_scroll_tip(state, "已复制该行 OCR 文字".into());
        invalidate(state);
        return;
    }

    // 5. Main toolbar & canvas drawing
    let action = {
        let Some(capture) = state.capture.as_mut() else {
            return;
        };
        if let Some(selection) = capture.selection {
            let toolbar = toolbar_origin(capture);
            let layout = toolbar_layout_for(capture);
            if let Some(index) = toolbar_button_at(toolbar, point, layout) {
                match capture.toolbar_action(index) {
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
                capture.push_annotation(Annotation::Number(
                    point,
                    number,
                    capture.annotation_color,
                ));
                capture.number_next = capture.number_next.saturating_add(1);
                capture.current_pos = Some(point);
                Some(ToolbarAction::PlaceNumber)
            } else if capture.tool == Tool::Eraser && selection.contains(point) {
                capture.drag = Some(point);
                capture.current_pos = Some(point);
                capture.erase_at(point);
                Some(ToolbarAction::PlaceNumber) // reuse to trigger invalidate
            } else if capture.tool != Tool::Select && selection.contains(point) {
                capture.drag = Some(point);
                capture.current_pos = Some(point);
                match capture.tool {
                    Tool::Pen => capture.push_annotation(Annotation::Pen(
                        vec![point],
                        capture.annotation_color,
                        capture.stroke_width,
                        capture.stroke_style,
                    )),
                    Tool::FadePen => capture.push_annotation(Annotation::FadePen {
                        points: vec![point],
                        color: capture.annotation_color,
                        width: capture.stroke_width,
                        created_ms: 0,
                    }),
                    Tool::Mosaic => capture.push_annotation(Annotation::Mosaic(
                        vec![point],
                        capture.brush_size,
                    )),
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
        Some(
            ToolbarAction::Tool(_)
            | ToolbarAction::PlaceNumber
            | ToolbarAction::Pin
            | ToolbarAction::Undo
            | ToolbarAction::Redo
            | ToolbarAction::Record,
        ) if recording_preview => {}
        Some(ToolbarAction::Tool(tool)) => {
            // Clicking an already-active grouped tool keeps its current variant, otherwise selects it
            let keep_variant = state.capture.as_ref().is_some_and(|c| {
                (tool == Tool::Rectangle && c.tool.is_shape_group())
                    || (tool == Tool::Arrow && c.tool.is_arrow_group())
                    || (tool == Tool::Pen && c.tool.is_pen_group())
                    || (tool == Tool::Mosaic && c.tool.is_effect_group())
            });
            if !keep_variant {
                set_tool(state, tool);
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
        Some(ToolbarAction::Redo) => redo(state),
        Some(ToolbarAction::Record) => open_toolbar_menu(state, ToolbarMenu::RecordingFps),
        Some(ToolbarAction::Ocr) => super::ocr_ui::start_text_extraction(state),
        Some(ToolbarAction::Translate) => {
            let already = state
                .capture
                .as_ref()
                .is_some_and(|c| c.translated_image.is_some());
            if already {
                if let Some(c) = state.capture.as_mut() {
                    c.show_translated_overlay = !c.show_translated_overlay;
                }
                invalidate(state);
            } else {
                super::ocr_ui::start_in_place_translation(state);
            }
        }
        Some(ToolbarAction::Qr) => super::ocr_ui::start_qr_recognition(state),
        Some(ToolbarAction::Copy) if recording_preview => {
            open_toolbar_menu(state, ToolbarMenu::RecordingExport)
        }
        Some(ToolbarAction::Copy) => copy_capture(state),
        Some(ToolbarAction::Save) if recording_preview => {
            open_toolbar_menu(state, ToolbarMenu::RecordingExport)
        }
        Some(ToolbarAction::Save) => finish_with_dialog(state),
        Some(ToolbarAction::Cancel | ToolbarAction::Finish) if recording_preview => cancel(state),
        Some(ToolbarAction::Cancel) => cancel(state),
        Some(ToolbarAction::Finish) => finish(state),
        _ => {}
    }
}

pub(super) fn mouse_move(state: &mut UiState, mut point: ScreenPoint, held: bool) {
    let editor_mode = state.editor.is_some();
    let detect_window = state
        .settings
        .lock()
        .map(|s| s.detect_window)
        .unwrap_or(true);
    let should_query_hover = detect_window
        && !held
        && state
            .capture
            .as_ref()
            .is_some_and(|capture| capture.selection.is_none())
        && state
            .last_hover_query
            .is_none_or(|(last, at)| last != point && at.elapsed() >= Duration::from_millis(30));
    let hover = should_query_hover.then(|| {
        state.last_hover_query = Some((point, Instant::now()));
        find_hover_window(point, state.overlay, &state.pinned_images)
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

    // Corner radius slider drag
    if held && capture.radius_dragging {
        if let Some(cap) = info_capsule_for(capture) {
            let s = cap.slider_track_rect();
            let ratio = ((point.x - s.0) as f32 / (s.2 - s.0).max(1) as f32).clamp(0.0, 1.0);
            capture.corner_radius = (ratio * 60.0).round() as u32;
            invalidate(state);
        }
        return;
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
                toolbar_layout_for(capture),
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
            toolbar_button_at(toolbar_origin(capture), point, toolbar_layout_for(capture))
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
            if capture.aspect_locked {
                let ow = (origin_rect.right - origin_rect.left).max(1) as f32;
                let oh = (origin_rect.bottom - origin_rect.top).max(1) as f32;
                let ratio = ow / oh;
                if edges.left || edges.right {
                    let nw = (right - left) as f32;
                    let nh = (nw / ratio).round().max(MIN_SIZE as f32) as i32;
                    if edges.top {
                        top = (bottom - nh).max(capture.screen_rect.top);
                    } else {
                        bottom = (top + nh).min(capture.screen_rect.bottom);
                    }
                } else {
                    let nh = (bottom - top) as f32;
                    let nw = (nh * ratio).round().max(MIN_SIZE as f32) as i32;
                    right = (left + nw).min(capture.screen_rect.right);
                }
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
        if should_query_hover || toolbar_hover_changed || secondary_hover_changed {
            invalidate(state);
        }
        return;
    }

    if capture.tool == Tool::Eraser && capture.selection.is_some() {
        if capture.erase_at(point) {
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
        Some(Annotation::FadePen { points, .. })
            if capture.tool == Tool::FadePen && capture.selection.is_some() =>
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
    if capture.radius_dragging {
        capture.radius_dragging = false;
        invalidate(state);
        return;
    }
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
        let filled = capture.shape_filled;
        let rounded = capture.shape_rounded;
        let rect = ScreenRect::normalized(start, point);
        match capture.tool {
            Tool::Rectangle if rect.width() > 2 && rect.height() > 2 => {
                capture.push_annotation(Annotation::Rect {
                    rect,
                    color,
                    width,
                    style,
                    filled,
                    rounded,
                });
            }
            Tool::Ellipse if rect.width() > 2 && rect.height() > 2 => {
                capture.push_annotation(Annotation::Ellipse {
                    rect,
                    color,
                    width,
                    style,
                    filled,
                });
            }
            Tool::Triangle if rect.width() > 2 && rect.height() > 2 => {
                capture.push_annotation(Annotation::Triangle {
                    rect,
                    color,
                    width,
                    style,
                    filled,
                });
            }
            Tool::Star if rect.width() > 4 && rect.height() > 4 => {
                capture.push_annotation(Annotation::Star {
                    rect,
                    color,
                    width,
                    style,
                    filled,
                });
            }
            Tool::Bubble if rect.width() > 8 && rect.height() > 8 => {
                capture.push_annotation(Annotation::Bubble {
                    rect,
                    color,
                    width,
                    style,
                });
            }
            Tool::Burst if rect.width() > 8 && rect.height() > 8 => {
                capture.push_annotation(Annotation::Burst {
                    rect,
                    color,
                    width,
                    style,
                });
            }
            Tool::Arrow if (start.x - point.x).abs() > 3 || (start.y - point.y).abs() > 3 => {
                capture.push_annotation(Annotation::Arrow {
                    start,
                    end: point,
                    color,
                    width,
                    style,
                    variant: ArrowVariant::Single,
                });
            }
            Tool::DoubleArrow if (start.x - point.x).abs() > 3 || (start.y - point.y).abs() > 3 => {
                capture.push_annotation(Annotation::Arrow {
                    start,
                    end: point,
                    color,
                    width,
                    style,
                    variant: ArrowVariant::Double,
                });
            }
            Tool::CurvedArrow if (start.x - point.x).abs() > 3 || (start.y - point.y).abs() > 3 => {
                capture.push_annotation(Annotation::Arrow {
                    start,
                    end: point,
                    color,
                    width,
                    style,
                    variant: ArrowVariant::Curved,
                });
            }
            Tool::Line if (start.x - point.x).abs() > 3 || (start.y - point.y).abs() > 3 => {
                capture.push_annotation(Annotation::Line(start, point, color, width, style));
            }
            Tool::Dimension if (start.x - point.x).abs() > 5 || (start.y - point.y).abs() > 5 => {
                capture.push_annotation(Annotation::Dimension {
                    start,
                    end: point,
                    color,
                    width,
                });
            }
            Tool::Angle if (start.x - point.x).abs() > 8 || (start.y - point.y).abs() > 8 => {
                capture.push_annotation(Annotation::Angle {
                    start,
                    end: point,
                    color,
                    width,
                });
            }
            Tool::Wave if (start.x - point.x).abs() > 5 || (start.y - point.y).abs() > 5 => {
                capture.push_annotation(Annotation::Wave {
                    start,
                    end: point,
                    color,
                    width,
                });
            }
            Tool::Highlight if (start.x - point.x).abs() > 3 || (start.y - point.y).abs() > 3 => {
                capture.push_annotation(Annotation::Highlight(
                    start,
                    point,
                    [color[0], color[1], color[2], 105],
                    width.saturating_mul(5).max(10),
                ));
            }
            Tool::Spotlight if rect.width() > 5 && rect.height() > 5 => {
                capture.push_annotation(Annotation::Spotlight(rect));
            }
            Tool::Patina if rect.width() > 6 && rect.height() > 6 => {
                capture.push_annotation(Annotation::Patina { rect, intensity: 3 });
            }
            Tool::Watermark if rect.width() > 12 && rect.height() > 12 => {
                let text = capture.watermark_text.clone();
                capture.push_annotation(Annotation::Watermark { rect, text, color });
            }
            Tool::Text => {
                capture.push_annotation(Annotation::Text(
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
        } else if key == VK_SPACE_CODE && state.recording.snapshot().phase != "exporting" {
            super::editor::toggle_recording_preview(state);
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
    {
        if key == VK_RETURN_CODE {
            if let Some(capture) = state.capture.as_mut() {
                capture.finish_text_editing();
            }
            invalidate(state);
        }
        return;
    }
    let ctrl_down = unsafe { GetKeyState(VK_CONTROL_CODE) } < 0;
    if ctrl_down && key == VK_Z {
        undo(state);
        return;
    }
    if ctrl_down && key == VK_Y {
        redo(state);
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
    if has_selection && key == 0x43 && ctrl_down {
        copy_capture(state);
        return;
    }
    if has_selection && key == VK_S_CODE && ctrl_down {
        finish_with_dialog(state);
        return;
    }
    match key {
        VK_A => set_tool(state, Tool::Arrow),
        VK_R => set_tool(state, Tool::Rectangle),
        VK_E => set_tool(state, Tool::Ellipse),
        VK_H => set_tool(state, Tool::Highlight),
        VK_I => set_tool(state, Tool::Line),
        VK_L => open_scroll_mode_menu(state),
        VK_M => set_tool(state, Tool::Mosaic),
        VK_N => set_tool(state, Tool::Number),
        VK_O => set_tool(state, Tool::Spotlight),
        VK_P => set_tool(state, Tool::Pen),
        VK_T => set_tool(state, Tool::Text),
        VK_V => set_tool(state, Tool::Select),
        VK_X => set_tool(state, Tool::Eraser),
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

pub(super) fn redo(state: &mut UiState) {
    if let Some(capture) = state.capture.as_mut() {
        capture.redo();
        invalidate(state);
    }
}

struct HoverQuery<'a> {
    point: ScreenPoint,
    overlay: HWND,
    pinned_images: &'a [PinnedImageWindow],
    best: Option<ScreenRect>,
}

unsafe extern "system" fn hover_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let query = &mut *(lparam as *mut HoverQuery<'_>);
    if hwnd == query.overlay
        || query.pinned_images.iter().any(|pin| pin.hwnd == hwnd)
        || IsWindowVisible(hwnd) == 0
        || IsIconic(hwnd) != 0
    {
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
    pinned_images: &[PinnedImageWindow],
) -> Option<ScreenRect> {
    let mut query = HoverQuery {
        point,
        overlay,
        pinned_images,
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
