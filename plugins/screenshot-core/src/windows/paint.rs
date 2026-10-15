use super::draw::{
    draw_annotations_gdi, draw_banner, draw_preview, draw_rect_border, draw_selection_handles_gdi,
    draw_size_badge,
};
use super::input::{
    secondary_bar_layout_for, tool_palette_layout_for, tool_palette_origin,
    toolbar_menu_layout_for, toolbar_origin,
};
use super::toolbar_paint::{
    draw_secondary_bar_gdi, draw_secondary_bar_tooltip, draw_tool_palette_tooltip,
    draw_toolbar_gdi, draw_toolbar_menu_gdi, draw_toolbar_tooltip, toolbar_layout,
};
use super::{wide, UiState, DEFAULT_GUI_FONT};
use crate::capture::ScreenCapture;
use crate::model::ScreenRect;

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{copy_nonoverlapping, null_mut};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection,
    CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint,
    FillRect, GetDC, GetStockObject, Polygon, ReleaseDC, RoundRect,
    SelectObject, SetBkMode, SetTextColor, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HDC, HGDIOBJ, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

pub(super) struct GdiSurface {
    pub(super) dc: HDC,
    pub(super) bitmap: HGDIOBJ,
    pub(super) old_bitmap: HGDIOBJ,
}

impl Drop for GdiSurface {
    fn drop(&mut self) {
        unsafe {
            if !self.dc.is_null() {
                if !self.old_bitmap.is_null() {
                    SelectObject(self.dc, self.old_bitmap);
                }
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
                DeleteDC(self.dc);
            }
        }
    }
}

pub(crate) struct OverlayRender {
    pub(super) original: GdiSurface,
    pub(super) dimmed: GdiSurface,
    pub(super) output: GdiSurface,
}

pub(super) unsafe fn create_gdi_surface(
    reference: HDC,
    width: i32,
    height: i32,
) -> Option<GdiSurface> {
    let dc = CreateCompatibleDC(reference);
    if dc.is_null() {
        return None;
    }
    let bitmap = CreateCompatibleBitmap(reference, width, height);
    if bitmap.is_null() {
        DeleteDC(dc);
        return None;
    }
    let bitmap = bitmap as HGDIOBJ;
    let old_bitmap = SelectObject(dc, bitmap);
    Some(GdiSurface {
        dc,
        bitmap,
        old_bitmap,
    })
}

pub(super) unsafe fn create_pixel_surface(
    reference: HDC,
    width: i32,
    height: i32,
    pixels: &[u8],
) -> Option<GdiSurface> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|value| value.checked_mul(4))?;
    if pixels.len() < expected {
        return None;
    }
    let info = bitmap_info(width, -height);
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(reference, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        if !bitmap.is_null() {
            DeleteObject(bitmap as HGDIOBJ);
        }
        return None;
    }
    let bitmap = bitmap as HGDIOBJ;
    let dc = CreateCompatibleDC(reference);
    if dc.is_null() {
        DeleteObject(bitmap);
        return None;
    }
    let old_bitmap = SelectObject(dc, bitmap);
    copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, expected);
    Some(GdiSurface {
        dc,
        bitmap,
        old_bitmap,
    })
}

pub(super) unsafe fn create_overlay_render(
    hwnd: HWND,
    screen: &ScreenCapture,
) -> Option<OverlayRender> {
    let reference = GetDC(hwnd);
    if reference.is_null() {
        return None;
    }
    let original = create_pixel_surface(reference, screen.width, screen.height, &screen.pixels);
    let dimmed = create_pixel_surface(
        reference,
        screen.width,
        screen.height,
        screen.dimmed_pixels(),
    );
    let output = create_gdi_surface(reference, screen.width, screen.height);
    let result = match (original, dimmed, output) {
        (Some(original), Some(dimmed), Some(output)) => Some(OverlayRender {
            original,
            dimmed,
            output,
        }),
        _ => None,
    };
    ReleaseDC(hwnd, reference);
    result
}

pub(super) fn paint(hwnd: HWND, state: &mut UiState) {
    unsafe {
        state.paint_pending = false;
        let mut paint_struct: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut paint_struct);
        if let (Some(screen), Some(capture)) = (state.screen.as_ref(), state.capture.as_ref()) {
            let active_rect = capture.selection.or_else(|| {
                let start = capture.drag?;
                let current = capture.current_pos?;
                let rect = ScreenRect::normalized(start, current);
                (rect.width() > 2 && rect.height() > 2).then_some(rect)
            });
            let render = state.render.as_ref();
            let render_dc = render.map(|value| value.output.dc).unwrap_or(dc);

            if let Some(render) = render {
                let source = if active_rect.is_some() {
                    render.dimmed.dc
                } else {
                    render.original.dc
                };
                BitBlt(
                    render.output.dc,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    source,
                    0,
                    0,
                    SRCCOPY,
                );
                if let Some(rect) = active_rect {
                    let rx = rect.left - screen.x;
                    let ry = rect.top - screen.y;
                    BitBlt(
                        render.output.dc,
                        rx,
                        ry,
                        rect.width() as i32,
                        rect.height() as i32,
                        render.original.dc,
                        rx,
                        ry,
                        SRCCOPY,
                    );
                }
            } else {
                draw_pixels(
                    render_dc,
                    if active_rect.is_some() {
                        screen.dimmed_pixels()
                    } else {
                        &screen.pixels
                    },
                    screen.width,
                    screen.height,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    0,
                    0,
                    screen.width,
                    screen.height,
                );
                if let Some(rect) = active_rect {
                    let rx = rect.left - screen.x;
                    let ry = rect.top - screen.y;
                    draw_pixels(
                        render_dc,
                        &screen.pixels,
                        screen.width,
                        screen.height,
                        rx,
                        ry,
                        rect.width() as i32,
                        rect.height() as i32,
                        rx,
                        ry,
                        rect.width() as i32,
                        rect.height() as i32,
                    );
                }
            }

            if let Some(rect) = active_rect {
                let rx = rect.left - screen.x;
                let ry = rect.top - screen.y;
                draw_rect_border(
                    render_dc,
                    rx,
                    ry,
                    rect.width() as i32,
                    rect.height() as i32,
                    0x00f76f16,
                    2,
                );
                if capture.selection.is_some() && !state.recording_overlay {
                    draw_selection_handles_gdi(
                        render_dc,
                        rx,
                        ry,
                        rect.width() as i32,
                        rect.height() as i32,
                    );
                    draw_size_badge(render_dc, rx, ry, rect.width() as i32, rect.height() as i32);
                    draw_banner(render_dc, 18, 18, "Enter 完成 · Esc 重选");
                }
                if capture.selection.is_some() && !state.recording_overlay {
                    draw_annotations_gdi(render_dc, capture, screen, screen.x, screen.y);
                    if let (Some(start), Some(current)) = (capture.drag, capture.current_pos) {
                        draw_preview(
                            render_dc,
                            screen,
                            capture.tool,
                            start,
                            current,
                            screen.x,
                            screen.y,
                            capture.stroke_width,
                            capture.stroke_style,
                            capture.brush_size,
                        );
                    }
                    let (toolbar_x, toolbar_y) = toolbar_origin(capture);
                    let toolbar_layout = toolbar_layout(capture.screen_rect);
                    let can_undo = !capture.annotations.is_empty();
                    let recording_setup_open =
                        capture.menu == Some(crate::toolbar::ToolbarMenu::RecordingFps);
                    if !recording_setup_open {
                        draw_toolbar_gdi(
                            render_dc,
                            toolbar_x - screen.x,
                            toolbar_y - screen.y,
                            toolbar_layout,
                            capture.tool,
                            capture.toolbar_hover,
                            capture.tool_palette_open,
                            crate::toolbar::toolbar_action,
                            crate::toolbar::TOOLBAR_BUTTON_COUNT,
                            capture.annotation_color,
                            capture.stroke_width,
                            capture.brush_size,
                            capture.text_size,
                            capture.stroke_style,
                            can_undo,
                        );
                    }
                    if !recording_setup_open && capture.tool_palette_open {
                        if let Some((palette_x, palette_y)) = tool_palette_origin(capture) {
                            let palette_layout = tool_palette_layout_for(capture);
                            draw_toolbar_gdi(
                                render_dc,
                                palette_x - screen.x,
                                palette_y - screen.y,
                                palette_layout,
                                capture.tool,
                                capture.tool_palette_hover,
                                false,
                                crate::toolbar::tool_palette_action,
                                crate::toolbar::TOOL_PALETTE_ACTIONS.len(),
                                capture.annotation_color,
                                capture.stroke_width,
                                capture.brush_size,
                                capture.text_size,
                                capture.stroke_style,
                                can_undo,
                            );
                            if let Some(index) = capture.tool_palette_hover {
                                draw_tool_palette_tooltip(
                                    render_dc,
                                    palette_x - screen.x,
                                    palette_y - screen.y,
                                    palette_layout,
                                    screen.width,
                                    screen.height,
                                    index,
                                );
                            }
                        }
                    }
                    if !recording_setup_open {
                        if let Some(mut sec_layout) = secondary_bar_layout_for(capture) {
                            sec_layout.left -= screen.x;
                            sec_layout.top -= screen.y;
                            draw_secondary_bar_gdi(
                                render_dc,
                                sec_layout,
                                capture.tool,
                                capture.secondary_hover,
                                capture.annotation_color,
                                capture.stroke_width,
                                capture.brush_size,
                                capture.text_size,
                            );
                            if let Some(sec_hover) = capture.secondary_hover {
                                draw_secondary_bar_tooltip(
                                    render_dc,
                                    sec_layout,
                                    screen.width,
                                    screen.height,
                                    sec_hover,
                                    capture.tool,
                                );
                            }
                        }
                    }
                    if let Some(menu) = capture.menu.filter(|menu| {
                        *menu != crate::toolbar::ToolbarMenu::RecordingFps
                    }) {
                        let mut menu_layout = toolbar_menu_layout_for(capture, menu);
                        menu_layout.left -= screen.x;
                        menu_layout.top -= screen.y;
                        draw_toolbar_menu_gdi(
                            render_dc,
                            menu_layout,
                            menu,
                            capture.tool,
                            capture.scroll_axis,
                            capture.menu_hover,
                            capture.annotation_color,
                            capture.stroke_width,
                            capture.brush_size,
                            capture.text_size,
                            capture.stroke_style,
                        );
                    }
                    if recording_setup_open {
                        let setup_layout = crate::toolbar::recording_setup_layout(
                            capture.selection.unwrap(),
                            capture.screen_rect,
                        );
                        super::toolbar_paint::draw_recording_setup_gdi(
                            render_dc,
                            crate::toolbar::RecordingSetupLayout {
                                left: setup_layout.left - screen.x,
                                top: setup_layout.top - screen.y,
                                ..setup_layout
                            },
                            match state.recording_setup_format {
                                crate::recorder::AnimationFormat::Mp4 => {
                                    crate::toolbar::AnimationFormatKey::Mp4
                                }
                                crate::recorder::AnimationFormat::Webp => {
                                    crate::toolbar::AnimationFormatKey::Webp
                                }
                                crate::recorder::AnimationFormat::Gif => {
                                    crate::toolbar::AnimationFormatKey::Gif
                                }
                            },
                            state.recording_show_cursor,
                        );
                    }
                    if !recording_setup_open && !capture.tool_palette_open {
                        if let Some(index) = capture.toolbar_hover {
                            draw_toolbar_tooltip(
                                render_dc,
                                toolbar_x - screen.x,
                                toolbar_y - screen.y,
                                toolbar_layout,
                                screen.width,
                                screen.height,
                                index,
                            );
                        }
                    }
                }
            } else if let Some(hover) = capture.hover {
                draw_rect_border(
                    render_dc,
                    hover.left - screen.x,
                    hover.top - screen.y,
                    hover.width() as i32,
                    hover.height() as i32,
                    0x00f76f16,
                    2,
                );
                draw_banner(
                    render_dc,
                    18,
                    18,
                    "点击选窗 · 拖拽框选",
                );
            } else {
                draw_banner(
                    render_dc,
                    18,
                    18,
                    "拖拽框选 · Esc 取消",
                );
            }

            if let Some(render) = render {
                BitBlt(
                    dc,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    render.output.dc,
                    0,
                    0,
                    SRCCOPY,
                );
            }
        }
        EndPaint(hwnd, &paint_struct);
    }
}

pub(super) fn paint_editor(hwnd: HWND, state: &mut UiState) {
    unsafe {
        state.paint_pending = false;
        let mut paint_struct: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut paint_struct);
        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);

        // Mouse moves and annotation previews invalidate the editor often.
        // Reuse cached GDI surfaces so each update only redraws annotations
        // over the cached screenshot, then present the frame with one blit.
        let canvas_dc = state
            .editor
            .as_ref()
            .and_then(|editor| editor.canvas_surface.as_ref())
            .map(|surface| surface.dc);
        let buffer_dc = state
            .editor
            .as_ref()
            .and_then(|editor| editor.back_buffer.as_ref())
            .map(|surface| surface.dc);
        let paint_dc = buffer_dc.unwrap_or(dc);
        let recording_preview = state
            .editor
            .as_ref()
            .is_some_and(|editor| editor.recording_preview);
        let background = CreateSolidBrush(if recording_preview {
            0x001d1d1f
        } else {
            0x00f4f5f7
        });
        FillRect(paint_dc, &client, background);
        DeleteObject(background as HGDIOBJ);

        if let (Some(screen), Some(capture), Some(editor)) = (
            state.screen.as_ref(),
            state.capture.as_ref(),
            state.editor.as_ref(),
        ) {
            if let Some(canvas_dc) = canvas_dc {
                BitBlt(
                    paint_dc,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    canvas_dc,
                    0,
                    0,
                    SRCCOPY,
                );
            } else {
                draw_pixels(
                    paint_dc,
                    &screen.pixels,
                    screen.width,
                    screen.height,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    0,
                    0,
                    screen.width,
                    screen.height,
                );
            }
            draw_rect_border(
                paint_dc,
                editor.display_rect.left,
                editor.display_rect.top,
                editor.display_rect.width() as i32,
                editor.display_rect.height() as i32,
                if editor.recording_preview {
                    0x003d4148
                } else {
                    0x00d0d5dd
                },
                1,
            );
            draw_annotations_gdi(paint_dc, capture, screen, screen.x, screen.y);
            if let (Some(start), Some(current)) = (capture.drag, capture.current_pos) {
                draw_preview(
                    paint_dc,
                    screen,
                    capture.tool,
                    start,
                    current,
                    screen.x,
                    screen.y,
                    capture.stroke_width,
                    capture.stroke_style,
                    capture.brush_size,
                );
            }

            let (toolbar_x, toolbar_y) = toolbar_origin(capture);
            let layout = toolbar_layout(capture.screen_rect);
            if editor.recording_preview {
                draw_recording_player(paint_dc, screen.width, screen.height, editor, state);
            } else {
                let can_undo = !capture.annotations.is_empty();
                draw_toolbar_gdi(
                    paint_dc,
                    toolbar_x,
                    toolbar_y,
                    layout,
                    capture.tool,
                    capture.toolbar_hover,
                    capture.tool_palette_open,
                    crate::toolbar::toolbar_action,
                    crate::toolbar::TOOLBAR_BUTTON_COUNT,
                    capture.annotation_color,
                    capture.stroke_width,
                    capture.brush_size,
                    capture.text_size,
                    capture.stroke_style,
                    can_undo,
                );
                if capture.tool_palette_open {
                    if let Some((palette_x, palette_y)) = tool_palette_origin(capture) {
                        let palette_layout = tool_palette_layout_for(capture);
                        draw_toolbar_gdi(
                            paint_dc,
                            palette_x,
                            palette_y,
                            palette_layout,
                            capture.tool,
                            capture.tool_palette_hover,
                            false,
                            crate::toolbar::tool_palette_action,
                            crate::toolbar::TOOL_PALETTE_ACTIONS.len(),
                            capture.annotation_color,
                            capture.stroke_width,
                            capture.brush_size,
                            capture.text_size,
                            capture.stroke_style,
                            can_undo,
                        );
                        if let Some(index) = capture.tool_palette_hover {
                            draw_tool_palette_tooltip(
                                paint_dc,
                                palette_x,
                                palette_y,
                                palette_layout,
                                screen.width,
                                screen.height,
                                index,
                            );
                        }
                    }
                }
                if let Some(sec_layout) = secondary_bar_layout_for(capture) {
                    draw_secondary_bar_gdi(
                        paint_dc,
                        sec_layout,
                        capture.tool,
                        capture.secondary_hover,
                        capture.annotation_color,
                        capture.stroke_width,
                        capture.brush_size,
                        capture.text_size,
                    );
                    if let Some(sec_hover) = capture.secondary_hover {
                        draw_secondary_bar_tooltip(
                            paint_dc,
                            sec_layout,
                            screen.width,
                            screen.height,
                            sec_hover,
                            capture.tool,
                        );
                    }
                }
            }
            if let Some(menu) = capture.menu {
                let menu_layout = toolbar_menu_layout_for(capture, menu);
                draw_toolbar_menu_gdi(
                    paint_dc,
                    menu_layout,
                    menu,
                    capture.tool,
                    capture.scroll_axis,
                    capture.menu_hover,
                    capture.annotation_color,
                    capture.stroke_width,
                    capture.brush_size,
                    capture.text_size,
                    capture.stroke_style,
                );
            }
            if !editor.recording_preview {
                if !capture.tool_palette_open {
                    if let Some(index) = capture.toolbar_hover {
                        draw_toolbar_tooltip(
                            paint_dc,
                            toolbar_x,
                            toolbar_y,
                            layout,
                            screen.width,
                            screen.height,
                            index,
                        );
                    }
                }
            }
        }

        if let Some(buffer_dc) = buffer_dc {
            let dirty = paint_struct.rcPaint;
            let width = (dirty.right - dirty.left).max(0);
            let height = (dirty.bottom - dirty.top).max(0);
            if width > 0 && height > 0 {
                BitBlt(
                    dc,
                    dirty.left,
                    dirty.top,
                    width,
                    height,
                    buffer_dc,
                    dirty.left,
                    dirty.top,
                    SRCCOPY,
                );
            }
        }

        EndPaint(hwnd, &paint_struct);
    }
}

unsafe fn draw_recording_player(
    dc: HDC,
    width: i32,
    height: i32,
    editor: &super::editor::EditorCanvas,
    state: &UiState,
) {
    let footer_height = super::editor::PLAYER_FOOTER_HEIGHT;
    let footer_top = height - footer_height;
    let timeline_y = footer_top - super::editor::PLAYER_TIMELINE_GAP;
    let left = super::editor::PLAYER_SIDE_PADDING;
    let right = (width - super::editor::PLAYER_SIDE_PADDING).max(left + 1);
    let status = state.recording.snapshot();
    let timeline_frame_count = status.timeline_frame_count();
    let progress = if timeline_frame_count <= 1 {
        0.0
    } else {
        editor.frame_index as f64
            / (timeline_frame_count.saturating_sub(1)) as f64
    }
    .clamp(0.0, 1.0);
    let knob_x = left + ((right - left) as f64 * progress).round() as i32;

    let track = CreateSolidBrush(0x00545a63);
    let played = CreateSolidBrush(0x00f76f16);
    let track_rect = RECT {
        left,
        top: timeline_y - 2,
        right,
        bottom: timeline_y + 2,
    };
    let played_rect = RECT {
        left,
        top: timeline_y - 2,
        right: knob_x.max(left + 1),
        bottom: timeline_y + 2,
    };
    FillRect(dc, &track_rect, track);
    FillRect(dc, &played_rect, played);
    DeleteObject(track as HGDIOBJ);
    DeleteObject(played as HGDIOBJ);

    let knob_brush = CreateSolidBrush(0x00f76f16);
    let knob_pen = CreatePen(0, 1, 0x00ffffff);
    let old_brush = SelectObject(dc, knob_brush as HGDIOBJ);
    let old_pen = SelectObject(dc, knob_pen as HGDIOBJ);
    Ellipse(dc, knob_x - 6, timeline_y - 6, knob_x + 6, timeline_y + 6);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(knob_brush as HGDIOBJ);
    DeleteObject(knob_pen as HGDIOBJ);

    let footer = CreateSolidBrush(0x00ffffff);
    let footer_rect = RECT {
        left: 0,
        top: footer_top,
        right: width,
        bottom: height,
    };
    FillRect(dc, &footer_rect, footer);
    DeleteObject(footer as HGDIOBJ);
    let divider = CreateSolidBrush(0x00e4e7ec);
    let divider_rect = RECT {
        left: 0,
        top: footer_top,
        right: width,
        bottom: footer_top + 1,
    };
    FillRect(dc, &divider_rect, divider);
    DeleteObject(divider as HGDIOBJ);

    let play_brush = CreateSolidBrush(0x00f3f5f8);
    let play_pen = CreatePen(0, 1, 0x00e2e6ec);
    let old_brush = SelectObject(dc, play_brush as HGDIOBJ);
    let old_pen = SelectObject(dc, play_pen as HGDIOBJ);
    RoundRect(dc, 16, footer_top + 15, 58, height - 9, 9, 9);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(play_brush as HGDIOBJ);
    DeleteObject(play_pen as HGDIOBJ);
    if editor.playing {
        let pause = CreateSolidBrush(0x002a3038);
        let first = RECT {
            left: 30,
            top: footer_top + 23,
            right: 35,
            bottom: footer_top + 39,
        };
        let second = RECT {
            left: 39,
            top: footer_top + 23,
            right: 44,
            bottom: footer_top + 39,
        };
        FillRect(dc, &first, pause);
        FillRect(dc, &second, pause);
        DeleteObject(pause as HGDIOBJ);
    } else {
        let points = [
            POINT { x: 31, y: footer_top + 21 },
            POINT { x: 31, y: footer_top + 41 },
            POINT { x: 45, y: footer_top + 31 },
        ];
        let play = CreateSolidBrush(0x002a3038);
        let old = SelectObject(dc, play as HGDIOBJ);
        Polygon(dc, points.as_ptr(), points.len() as i32);
        SelectObject(dc, old);
        DeleteObject(play as HGDIOBJ);
    }

    let fps = status.fps.max(1) as u64;
    let current_ms = editor.frame_index as u64 * 1000 / fps;
    let duration_ms = status
        .elapsed_ms
        .max(timeline_frame_count.saturating_sub(1) as u64 * 1000 / fps);
    let timestamp = format!(
        "{} / {}",
        format_player_time(current_ms),
        format_player_time(duration_ms)
    );
    draw_player_text(dc, 76, footer_top + 12, 270, height - 8, &timestamp, 0x00635b50, 0);

    let export_rect = (width - 286, footer_top + 10, width - 176, height - 10);
    let copy_rect = (width - 166, footer_top + 10, width - 16, height - 10);
    draw_player_button(dc, export_rect, "导出", true);
    draw_player_button(dc, copy_rect, "复制当前帧", false);
}

unsafe fn draw_player_button(dc: HDC, rect: (i32, i32, i32, i32), label: &str, primary: bool) {
    let fill_color = if primary { 0x00f76f16 } else { 0x00ffffff };
    let border_color = 0x00f76f16;
    let text_color = if primary { 0x00ffffff } else { 0x00f76f16 };
    let brush = CreateSolidBrush(fill_color);
    let pen = CreatePen(0, 1, border_color);
    let old_brush = SelectObject(dc, brush as HGDIOBJ);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    RoundRect(dc, rect.0, rect.1, rect.2, rect.3, 10, 10);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);
    draw_player_text(dc, rect.0 + 4, rect.1, rect.2 - 4, rect.3, label, text_color, 1);
}

unsafe fn draw_player_text(
    dc: HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    value: &str,
    color: u32,
    centered: i32,
) {
    let old_font = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
    SetBkMode(dc, 1);
    SetTextColor(dc, color);
    let text = wide(value);
    let mut rect = RECT {
        left,
        top,
        right,
        bottom,
    };
    DrawTextW(
        dc,
        text.as_ptr(),
        -1,
        &mut rect,
        0x00000004 | 0x00000020 | if centered == 1 { 0x00000001 } else { 0 },
    );
    SelectObject(dc, old_font);
}

fn format_player_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

pub(crate) fn rgba_preview_bgra(
    pixels: &[u8],
    source_width: i32,
    source_height: i32,
    target_width: i32,
    target_height: i32,
) -> Option<Vec<u8>> {
    let source_width = usize::try_from(source_width).ok()?;
    let source_height = usize::try_from(source_height).ok()?;
    let target_width = usize::try_from(target_width).ok()?;
    let target_height = usize::try_from(target_height).ok()?;
    if source_width == 0 || source_height == 0 || target_width == 0 || target_height == 0 {
        return None;
    }
    let source_len = source_width.checked_mul(source_height)?.checked_mul(4)?;
    if pixels.len() != source_len {
        return None;
    }
    let target_len = target_width.checked_mul(target_height)?.checked_mul(4)?;
    let mut bgra = vec![0u8; target_len];
    for y in 0..target_height {
        let source_y = y.saturating_mul(source_height) / target_height;
        for x in 0..target_width {
            let source_x = x.saturating_mul(source_width) / target_width;
            let source = (source_y * source_width + source_x) * 4;
            let target = (y * target_width + x) * 4;
            bgra[target] = pixels[source + 2];
            bgra[target + 1] = pixels[source + 1];
            bgra[target + 2] = pixels[source];
            bgra[target + 3] = pixels[source + 3];
        }
    }
    Some(bgra)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_rgba_preview(
    dc: HDC,
    pixels: &[u8],
    source_width: i32,
    source_height: i32,
    destination_x: i32,
    destination_y: i32,
    destination_width: i32,
    destination_height: i32,
) {
    let Some(bgra) = rgba_preview_bgra(
        pixels,
        source_width,
        source_height,
        destination_width,
        destination_height,
    ) else {
        return;
    };
    draw_pixels(
        dc,
        &bgra,
        destination_width,
        destination_height,
        0,
        0,
        destination_width,
        destination_height,
        destination_x,
        destination_y,
        destination_width,
        destination_height,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_pixels(
    dc: HDC,
    pixels: &[u8],
    source_width: i32,
    source_height: i32,
    source_x: i32,
    source_y: i32,
    source_width_rect: i32,
    source_height_rect: i32,
    destination_x: i32,
    destination_y: i32,
    destination_width: i32,
    destination_height: i32,
) {
    if dc.is_null()
        || pixels.is_empty()
        || source_width <= 0
        || source_height <= 0
        || source_width_rect <= 0
        || source_height_rect <= 0
        || destination_width <= 0
        || destination_height <= 0
    {
        return;
    }
    let info = bitmap_info(source_width, -source_height);
    unsafe {
        StretchDIBits(
            dc,
            destination_x,
            destination_y,
            destination_width,
            destination_height,
            source_x,
            source_y,
            source_width_rect,
            source_height_rect,
            pixels.as_ptr() as *const c_void,
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

pub(super) fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { zeroed() }
        },
        bmiColors: [unsafe { zeroed() }],
    }
}
