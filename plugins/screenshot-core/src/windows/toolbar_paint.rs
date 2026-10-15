use super::draw::{draw_ellipse_border, draw_line_gdi_style, gdi_color};
use super::{wide, DEFAULT_GUI_FONT};
use crate::model::{ScreenPoint, ScreenRect, StrokeStyle};
use crate::scroll::MatchAxis;
use crate::toolbar::{
    button_origin as toolbar_button_origin, layout_for_screen_width, toolbar_icon_index,
    SecondaryBarItem, SecondaryBarLayout, Tool, ToolbarAction, ToolbarLayout, ToolbarMenu,
    AnimationFormatKey, RecordingSetupLayout, COLOR_SWATCHES, MOSAIC_SIZES, STROKE_SIZES, TEXT_SIZES,
    TOOLBAR_BUTTON_HEIGHT, TOOLBAR_BUTTON_WIDTH, TOOLBAR_VERTICAL_PADDING,
};

pub(super) const SCROLL_TOOLBAR_ACTIONS: [ToolbarAction; 7] = [
    ToolbarAction::Copy,
    ToolbarAction::Ocr,
    ToolbarAction::Save,
    ToolbarAction::Pin,
    ToolbarAction::Edit,
    ToolbarAction::Cancel,
    ToolbarAction::Finish,
];

use std::ptr::null_mut;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, Ellipse, GetStockObject,
    LineTo, MoveToEx, Polygon, RoundRect, SelectObject, SetBkMode, SetTextColor, HDC, HGDIOBJ,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    GdipCreateFromHDC, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics,
    GdipDeletePen, GdipDrawArc, GdipDrawBezier, GdipDrawEllipse, GdipDrawLine,
    GdipFillEllipse, GdipFillRectangle, GdipSetPenLineCap197819, GdipSetPenLineJoin,
    GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdiplusStartup, GdiplusStartupInput,
    GpBrush, GpGraphics, GpPen, GpSolidFill, DashCapRound, LineCapRound, LineJoinRound,
    PixelOffsetModeHalf, SmoothingModeAntiAlias8x8, UnitPixel,
};

pub(crate) fn toolbar_layout(screen: ScreenRect) -> ToolbarLayout {
    layout_for_screen_width(screen.width() as i32)
}

#[allow(dead_code)]
unsafe fn draw_recording_setup_legacy_gdi(
    dc: HDC,
    layout: RecordingSetupLayout,
    format: AnimationFormatKey,
    fps: u32,
) {
    let shadow = CreateSolidBrush(0x00303030);
    let old_shadow = SelectObject(dc, shadow as HGDIOBJ);
    RoundRect(
        dc,
        layout.left + 2,
        layout.top + 4,
        layout.left + layout.width + 2,
        layout.top + layout.height + 4,
        16,
        16,
    );
    SelectObject(dc, old_shadow);
    DeleteObject(shadow as HGDIOBJ);

    let panel = CreateSolidBrush(0x00ffffff);
    let outline = CreatePen(0, 1, 0x00d9dee7);
    let old_panel = SelectObject(dc, panel as HGDIOBJ);
    let old_outline = SelectObject(dc, outline as HGDIOBJ);
    RoundRect(
        dc,
        layout.left,
        layout.top,
        layout.left + layout.width,
        layout.top + layout.height,
        16,
        16,
    );
    SelectObject(dc, old_panel);
    SelectObject(dc, old_outline);
    DeleteObject(panel as HGDIOBJ);
    DeleteObject(outline as HGDIOBJ);

    draw_menu_text(
        dc,
        layout.left + 20,
        layout.top + 1,
        layout.left + layout.width - 52,
        layout.top + 25,
        "录制设置",
    );
    draw_menu_muted_text(
        dc,
        layout.left + 20,
        layout.top + 20,
        layout.left + layout.width - 52,
        layout.top + 42,
        "选择格式和帧率后开始录制",
    );
    draw_menu_text(
        dc,
        layout.left + layout.width - 40,
        layout.top + 1,
        layout.left + layout.width - 12,
        layout.top + 33,
        "×",
    );

    let start = layout.start_rect();
    let primary = CreateSolidBrush(0x00f76f16);
    let no_outline = CreatePen(0, 1, 0x00f76f16);
    let old_primary = SelectObject(dc, primary as HGDIOBJ);
    let old_pen = SelectObject(dc, no_outline as HGDIOBJ);
    RoundRect(dc, start.0, start.1, start.2, start.3, 10, 10);
    SelectObject(dc, old_primary);
    SelectObject(dc, old_pen);
    DeleteObject(primary as HGDIOBJ);
    DeleteObject(no_outline as HGDIOBJ);
    draw_setup_label(dc, start, "开始录制", true, true);

    for index in 0..2 {
        let is_selected = (index == 0 && format == AnimationFormatKey::Mp4)
            || (index == 1 && format == AnimationFormatKey::Gif)
            || (index == 2 && format == AnimationFormatKey::Webp);
        let rect = layout.format_rect(index).unwrap();
        draw_setup_label(dc, rect, match index { 0 => "MP4", 1 => "GIF 动图", _ => "WebP" }, is_selected, false);
        let fps_rect = layout.fps_rect(index).unwrap();
        let selected = (index == 0 && fps == 15) || (index == 1 && fps == 30);
        draw_setup_label(dc, fps_rect, if index == 0 { "标准帧率  ·  15 FPS" } else { "流畅帧率  ·  30 FPS" }, selected, false);
    }
}

pub(super) unsafe fn draw_recording_setup_gdi(
    dc: HDC,
    layout: RecordingSetupLayout,
    format: AnimationFormatKey,
    show_cursor: bool,
) {
    let shadow = CreateSolidBrush(0x00303030);
    let old_shadow = SelectObject(dc, shadow as HGDIOBJ);
    RoundRect(
        dc,
        layout.left + 2,
        layout.top + 4,
        layout.left + layout.width + 2,
        layout.top + layout.height + 4,
        14,
        14,
    );
    SelectObject(dc, old_shadow);
    DeleteObject(shadow as HGDIOBJ);

    let panel = CreateSolidBrush(0x00ffffff);
    let outline = CreatePen(0, 1, 0x00d9dee7);
    let old_panel = SelectObject(dc, panel as HGDIOBJ);
    let old_outline = SelectObject(dc, outline as HGDIOBJ);
    RoundRect(
        dc,
        layout.left,
        layout.top,
        layout.left + layout.width,
        layout.top + layout.height,
        14,
        14,
    );
    SelectObject(dc, old_panel);
    SelectObject(dc, old_outline);
    DeleteObject(panel as HGDIOBJ);
    DeleteObject(outline as HGDIOBJ);

    let start = layout.start_rect();
    let primary = CreateSolidBrush(0x00f76f16);
    let no_outline = CreatePen(0, 1, 0x00f76f16);
    let old_primary = SelectObject(dc, primary as HGDIOBJ);
    let old_pen = SelectObject(dc, no_outline as HGDIOBJ);
    RoundRect(dc, start.0, start.1, start.2, start.3, 12, 12);
    SelectObject(dc, old_primary);
    SelectObject(dc, old_pen);
    DeleteObject(primary as HGDIOBJ);
    DeleteObject(no_outline as HGDIOBJ);
    draw_setup_text(dc, start, "开始录制", 0x00ffffff, true);

    let separator = CreatePen(0, 1, 0x00e7e9ed);
    let old_separator = SelectObject(dc, separator as HGDIOBJ);
    MoveToEx(dc, layout.left + 1, layout.top + 82, null_mut());
    LineTo(dc, layout.left + layout.width - 1, layout.top + 82);
    SelectObject(dc, old_separator);
    DeleteObject(separator as HGDIOBJ);

    draw_setup_text(
        dc,
        (
            layout.left + 28,
            layout.top + 88,
            layout.left + 132,
            layout.top + 126,
        ),
        "选择格式",
        0x00505b68,
        false,
    );
    draw_setup_radio(
        dc,
        layout.format_rect(0).unwrap(),
        "MP4",
        format == AnimationFormatKey::Mp4,
    );
    draw_setup_radio(
        dc,
        layout.format_rect(1).unwrap(),
        "GIF",
        format == AnimationFormatKey::Gif,
    );
    draw_setup_radio(
        dc,
        layout.format_rect(2).unwrap(),
        "WebP",
        format == AnimationFormatKey::Webp,
    );

    let controls = ["扬声器", "麦克风", "摄像头", "鼠标设置"];
    for (index, label) in controls.into_iter().enumerate() {
        let rect = layout.control_rect(index).unwrap();
        let center_x = (rect.0 + rect.2) / 2;
        let icon_top = layout.top + 146;
        let enabled = index == 3;
        let color = if enabled { 0x00343a45 } else { 0x00939da8 };
        match index {
            0 => draw_setup_speaker_icon(dc, center_x, icon_top, color),
            1 => draw_microphone_setup_icon(dc, center_x, icon_top, color),
            2 => draw_camera_setup_icon(dc, center_x, icon_top, color),
            _ => draw_cursor_setup_icon(dc, center_x - 5, icon_top + 3, color),
        }
        draw_setup_text(
            dc,
            (rect.0, layout.top + 178, rect.2, layout.top + 200),
            label,
            color,
            false,
        );
        let detail = match index {
            0 | 1 | 2 => "后续支持",
            _ if show_cursor => "显示鼠标",
            _ => "隐藏鼠标",
        };
        draw_setup_text(
            dc,
            (rect.0, layout.top + 199, rect.2, layout.top + 219),
            detail,
            if enabled { 0x0076818f } else { 0x00a5adb7 },
            false,
        );
        if index == 3 {
            draw_setup_dropdown(dc, rect.2 - 12, icon_top + 11, color);
            let indicator = CreateSolidBrush(if show_cursor { 0x00f76f16 } else { 0x00adb5bf });
            let old_indicator = SelectObject(dc, indicator as HGDIOBJ);
            Ellipse(dc, rect.2 - 25, icon_top + 2, rect.2 - 17, icon_top + 10);
            SelectObject(dc, old_indicator);
            DeleteObject(indicator as HGDIOBJ);
        }
    }

    let close = layout.close_rect();
    draw_setup_text(dc, close, "×", 0x0077808c, false);
}

unsafe fn draw_setup_text(
    dc: HDC,
    rect: (i32, i32, i32, i32),
    label: &str,
    color: u32,
    centered: bool,
) {
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let old_font = SelectObject(dc, font);
    SetBkMode(dc, 1);
    SetTextColor(dc, color);
    let text = wide(label);
    let mut target = RECT {
        left: rect.0 + 2,
        top: rect.1,
        right: rect.2 - 2,
        bottom: rect.3,
    };
    DrawTextW(
        dc,
        text.as_ptr(),
        -1,
        &mut target,
        if centered { 1 | 4 | 0x20 } else { 1 | 4 },
    );
    SelectObject(dc, old_font);
}

unsafe fn draw_setup_radio(
    dc: HDC,
    rect: (i32, i32, i32, i32),
    label: &str,
    selected: bool,
) {
    let center_x = rect.0 + 14;
    let center_y = (rect.1 + rect.3) / 2;
    let border = CreatePen(0, 1, if selected { 0x00f76f16 } else { 0x009ba5b1 });
    let white = CreateSolidBrush(0x00ffffff);
    let old_border = SelectObject(dc, border as HGDIOBJ);
    let old_white = SelectObject(dc, white as HGDIOBJ);
    Ellipse(dc, center_x - 11, center_y - 11, center_x + 11, center_y + 11);
    SelectObject(dc, old_border);
    SelectObject(dc, old_white);
    DeleteObject(border as HGDIOBJ);
    DeleteObject(white as HGDIOBJ);
    if selected {
        let dot = CreateSolidBrush(0x00f76f16);
        let old_dot = SelectObject(dc, dot as HGDIOBJ);
        Ellipse(dc, center_x - 5, center_y - 5, center_x + 5, center_y + 5);
        SelectObject(dc, old_dot);
        DeleteObject(dot as HGDIOBJ);
    }
    draw_setup_text(
        dc,
        (rect.0 + 30, rect.1, rect.2, rect.3),
        label,
        0x002d3742,
        false,
    );
}

unsafe fn draw_setup_speaker_icon(dc: HDC, center_x: i32, top: i32, color: u32) {
    let speaker = [
        POINT { x: center_x - 15, y: top + 11 },
        POINT { x: center_x - 8, y: top + 11 },
        POINT { x: center_x + 1, y: top + 4 },
        POINT { x: center_x + 1, y: top + 30 },
        POINT { x: center_x - 8, y: top + 23 },
        POINT { x: center_x - 15, y: top + 23 },
    ];
    let brush = CreateSolidBrush(color);
    let old = SelectObject(dc, brush as HGDIOBJ);
    Polygon(dc, speaker.as_ptr(), speaker.len() as i32);
    let pen = CreatePen(0, 2, color);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    for offset in [0, 6] {
        MoveToEx(dc, center_x + 7 + offset, top + 11 - offset / 2, null_mut());
        LineTo(dc, center_x + 11 + offset, top + 17);
        LineTo(dc, center_x + 7 + offset, top + 23 + offset / 2);
    }
    SelectObject(dc, old_pen);
    DeleteObject(pen as HGDIOBJ);
    SelectObject(dc, old);
    DeleteObject(brush as HGDIOBJ);
}

unsafe fn draw_microphone_setup_icon(dc: HDC, center_x: i32, top: i32, color: u32) {
    let pen = CreatePen(0, 2, color);
    let brush = CreateSolidBrush(0x00ffffff);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    let old_brush = SelectObject(dc, brush as HGDIOBJ);
    RoundRect(dc, center_x - 7, top + 3, center_x + 7, top + 23, 9, 9);
    MoveToEx(dc, center_x - 13, top + 17, null_mut());
    LineTo(dc, center_x - 13, top + 20);
    LineTo(dc, center_x - 8, top + 26);
    LineTo(dc, center_x, top + 29);
    LineTo(dc, center_x + 8, top + 26);
    LineTo(dc, center_x + 13, top + 20);
    LineTo(dc, center_x + 13, top + 17);
    MoveToEx(dc, center_x, top + 29, null_mut());
    LineTo(dc, center_x, top + 34);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);
}

unsafe fn draw_camera_setup_icon(dc: HDC, center_x: i32, top: i32, color: u32) {
    let pen = CreatePen(0, 2, color);
    let brush = CreateSolidBrush(0x00ffffff);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    let old_brush = SelectObject(dc, brush as HGDIOBJ);
    RoundRect(dc, center_x - 17, top + 7, center_x + 11, top + 27, 4, 4);
    let lens = [
        POINT { x: center_x + 11, y: top + 12 },
        POINT { x: center_x + 19, y: top + 7 },
        POINT { x: center_x + 19, y: top + 27 },
        POINT { x: center_x + 11, y: top + 22 },
    ];
    Polygon(dc, lens.as_ptr(), lens.len() as i32);
    MoveToEx(dc, center_x - 18, top + 3, null_mut());
    LineTo(dc, center_x + 20, top + 31);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);
}

unsafe fn draw_cursor_setup_icon(dc: HDC, left: i32, top: i32, color: u32) {
    let points = [
        POINT { x: left, y: top },
        POINT { x: left + 1, y: top + 31 },
        POINT { x: left + 9, y: top + 23 },
        POINT { x: left + 15, y: top + 35 },
        POINT { x: left + 21, y: top + 32 },
        POINT { x: left + 15, y: top + 20 },
        POINT { x: left + 26, y: top + 20 },
    ];
    let brush = CreateSolidBrush(color);
    let pen = CreatePen(0, 1, 0x00ffffff);
    let old_brush = SelectObject(dc, brush as HGDIOBJ);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    Polygon(dc, points.as_ptr(), points.len() as i32);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);
}

unsafe fn draw_setup_dropdown(dc: HDC, x: i32, y: i32, color: u32) {
    let points = [
        POINT { x, y },
        POINT { x: x + 7, y },
        POINT { x: x + 3, y: y + 4 },
    ];
    let brush = CreateSolidBrush(color);
    let old = SelectObject(dc, brush as HGDIOBJ);
    Polygon(dc, points.as_ptr(), points.len() as i32);
    SelectObject(dc, old);
    DeleteObject(brush as HGDIOBJ);
}

unsafe fn draw_setup_label(
    dc: HDC,
    rect: (i32, i32, i32, i32),
    label: &str,
    selected: bool,
    primary: bool,
) {
    let fill_color = if primary {
        0x00f76f16
    } else if selected {
        0x00eff6ff
    } else {
        0x00f6f7f9
    };
    let border_color = if primary {
        0x00f76f16
    } else if selected {
        0x00f76f16
    } else {
        0x00e3e7ed
    };
    let brush = CreateSolidBrush(fill_color);
    let pen = CreatePen(0, 1, border_color);
    let old_brush = SelectObject(dc, brush as HGDIOBJ);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    RoundRect(dc, rect.0, rect.1, rect.2, rect.3, 8, 8);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);

    let font = GetStockObject(DEFAULT_GUI_FONT);
    let old_font = SelectObject(dc, font);
    SetBkMode(dc, 1);
    SetTextColor(dc, if primary { 0x00ffffff } else { 0x002c3440 });
    let text = super::wide(label);
    let mut text_rect = RECT {
        left: rect.0 + 6,
        top: rect.1 + 2,
        right: rect.2 - 6,
        bottom: rect.3 - 2,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut text_rect, 1 | 4 | 0x20);
    SelectObject(dc, old_font);
}

pub(super) fn scroll_toolbar_layout() -> ToolbarLayout {
    let columns = SCROLL_TOOLBAR_ACTIONS.len();
    ToolbarLayout {
        item_count: columns,
        columns,
        rows: 1,
        width: columns as i32 * TOOLBAR_BUTTON_WIDTH,
        height: TOOLBAR_BUTTON_HEIGHT + TOOLBAR_VERTICAL_PADDING * 2,
    }
}

pub(super) fn scroll_toolbar_action(index: usize) -> Option<ToolbarAction> {
    SCROLL_TOOLBAR_ACTIONS.get(index).copied()
}

pub(crate) fn toolbar_position(selection: ScreenRect, screen: ScreenRect) -> (i32, i32) {
    toolbar_position_for_layout(selection, screen, toolbar_layout(screen))
}

pub(super) fn scroll_toolbar_position(selection: ScreenRect, screen: ScreenRect) -> (i32, i32) {
    toolbar_position_for_layout(selection, screen, scroll_toolbar_layout())
}

pub(super) fn toolbar_position_for_layout(
    selection: ScreenRect,
    screen: ScreenRect,
    layout: ToolbarLayout,
) -> (i32, i32) {
    let mut x = selection.right - layout.width;
    x = x.max(screen.left + 8);
    if x + layout.width > screen.right - 8 {
        x = (screen.right - layout.width - 8).max(screen.left + 8);
    }
    let mut y = selection.bottom + 8;
    if y + layout.height > screen.bottom - 8 {
        y = (selection.top - layout.height - 8).max(screen.top + 8);
    }
    (x, y)
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_toolbar_gdi(
    dc: HDC,
    x: i32,
    y: i32,
    layout: ToolbarLayout,
    active_tool: Tool,
    hovered: Option<usize>,
    tool_palette_open: bool,
    action_at: fn(usize) -> Option<ToolbarAction>,
    button_count: usize,
    annotation_color: [u8; 4],
    stroke_width: u32,
    brush_size: u32,
    text_size: u32,
    stroke_style: StrokeStyle,
    can_undo: bool,
) {
    // One soft shadow and a light outline keep attention on the selected area.
    let shadow = CreateSolidBrush(0x00e9e9e9);
    let old_shadow = SelectObject(dc, shadow as HGDIOBJ);
    let old_shadow_pen = SelectObject(
        dc,
        GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_PEN as i32),
    );
    RoundRect(
        dc,
        x,
        y + 2,
        x + layout.width,
        y + layout.height + 2,
        12,
        12,
    );
    SelectObject(dc, old_shadow_pen);
    SelectObject(dc, old_shadow);
    DeleteObject(shadow as HGDIOBJ);

    let outer_pen = CreatePen(0, 1, 0x00e1e3e6);
    let outer_brush = CreateSolidBrush(0x00ffffff);
    let old_pen = SelectObject(dc, outer_pen as HGDIOBJ);
    let old_brush = SelectObject(dc, outer_brush as HGDIOBJ);
    RoundRect(dc, x, y, x + layout.width, y + layout.height, 10, 10);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(outer_pen as HGDIOBJ);
    DeleteObject(outer_brush as HGDIOBJ);

    for item_position in 0..button_count {
        let Some(action) = action_at(item_position) else {
            continue;
        };
        let Some((bx, by)) = toolbar_button_origin((x, y), item_position, layout) else {
            continue;
        };
        let center_x = bx + TOOLBAR_BUTTON_WIDTH / 2;
        let center_y = by + TOOLBAR_BUTTON_HEIGHT / 2;

        // Split tools from the output actions with one quiet divider.
        if action == ToolbarAction::Undo {
            let separator = CreatePen(0, 1, 0x00ebe6e5);
            let old = SelectObject(dc, separator as HGDIOBJ);
            MoveToEx(dc, bx - 1, center_y - 8, null_mut());
            LineTo(dc, bx - 1, center_y + 8);
            SelectObject(dc, old);
            DeleteObject(separator as HGDIOBJ);
        }

        let tool = match action {
            ToolbarAction::Tool(tool) => Some(tool),
            _ => None,
        };

        let is_active = tool == Some(active_tool)
            || (action == ToolbarAction::Tools && (tool_palette_open || active_tool != Tool::Select));
        let is_hovered = hovered == Some(item_position);
        let is_undo = action == ToolbarAction::Undo;
        // COLORREF values use BGR order. Keep the completion and close fills light,
        // with a slightly stronger tint on hover.
        let (fill_color, border_color, icon_color) = if is_undo && !can_undo {
            (None, None, 0x00c2bbb6)
        } else {
            match action {
                ToolbarAction::Finish => {
                    let (fill, border) = if is_hovered {
                        (0x00ffe5cf, 0x00f0c9a8)
                    } else {
                        (0x00ffefe1, 0x00f8d3b7)
                    };
                    (Some(fill), Some(border), 0x00cd7032)
                }
                ToolbarAction::Cancel => {
                    let (fill, border) = if is_hovered {
                        (0x00d6d6ff, 0x00bdbdf0)
                    } else {
                        (0x00e7e7ff, 0x00c2c2f8)
                    };
                    (Some(fill), Some(border), 0x005252d2)
                }
                _ if is_active || is_hovered => {
                    (Some(0x00f1f1f1), None, 0x0034312e)
                }
                _ => (None, None, 0x0034312e),
            }
        };

        if let Some(color) = fill_color {
            let fill = CreateSolidBrush(color);
            let border = CreatePen(0, 1, border_color.unwrap_or(color));
            let old_b = SelectObject(dc, fill as HGDIOBJ);
            let old_p = SelectObject(dc, border as HGDIOBJ);
            RoundRect(
                dc,
                bx + 5,
                by + 5,
                bx + TOOLBAR_BUTTON_WIDTH - 5,
                by + TOOLBAR_BUTTON_HEIGHT - 5,
                8,
                8,
            );
            SelectObject(dc, old_b);
            SelectObject(dc, old_p);
            DeleteObject(fill as HGDIOBJ);
            DeleteObject(border as HGDIOBJ);
        }

        let Some(icon_index) = toolbar_icon_index(action) else {
            continue;
        };
        draw_toolbar_icon(
            dc,
            icon_index,
            center_x,
            center_y,
            active_tool,
            icon_color,
            annotation_color,
            stroke_width,
            brush_size,
            text_size,
            stroke_style,
        );
    }
}

/// 图 2：二级属性浮条（Secondary Property Bar）GDI 绘制
pub(super) unsafe fn draw_secondary_bar_gdi(
    dc: HDC,
    layout: SecondaryBarLayout,
    tool: Tool,
    hovered: Option<SecondaryBarItem>,
    annotation_color: [u8; 4],
    stroke_width: u32,
    brush_size: u32,
    text_size: u32,
) {
    // 多层微阴影
    let null_brush = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_BRUSH as i32);
    let old_sb = SelectObject(dc, null_brush);

    let shadow1 = CreatePen(0, 1, 0x00f4f0ee);
    let old_sp = SelectObject(dc, shadow1 as HGDIOBJ);
    RoundRect(
        dc,
        layout.left - 1,
        layout.top + 1,
        layout.left + layout.width + 1,
        layout.top + layout.height + 3,
        10,
        10,
    );

    let shadow2 = CreatePen(0, 1, 0x00e8e3e0);
    SelectObject(dc, shadow2 as HGDIOBJ);
    RoundRect(
        dc,
        layout.left,
        layout.top + 1,
        layout.left + layout.width,
        layout.top + layout.height + 2,
        9,
        9,
    );

    let shadow3 = CreatePen(0, 1, 0x00ded8d5);
    SelectObject(dc, shadow3 as HGDIOBJ);
    RoundRect(
        dc,
        layout.left,
        layout.top + 1,
        layout.left + layout.width,
        layout.top + layout.height + 1,
        8,
        8,
    );

    SelectObject(dc, old_sp);
    SelectObject(dc, old_sb);
    DeleteObject(shadow1 as HGDIOBJ);
    DeleteObject(shadow2 as HGDIOBJ);
    DeleteObject(shadow3 as HGDIOBJ);

    // 纯白浮层 + 浅灰精细边框 (8px 圆角)
    let outer_pen = CreatePen(0, 1, 0x00e3e0de);
    let outer_brush = CreateSolidBrush(0x00ffffff);
    let old_pen = SelectObject(dc, outer_pen as HGDIOBJ);
    let old_brush = SelectObject(dc, outer_brush as HGDIOBJ);
    RoundRect(
        dc,
        layout.left,
        layout.top,
        layout.left + layout.width,
        layout.top + layout.height,
        8,
        8,
    );
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(outer_pen as HGDIOBJ);
    DeleteObject(outer_brush as HGDIOBJ);

    // 1. 绘制 4 档大小 (文字工具显示 14, 18, 24, 32 明确字号；笔触/马赛克显示圆点)
    let current_size_index = if tool == Tool::Text {
        TEXT_SIZES
            .iter()
            .enumerate()
            .min_by_key(|(_, &s)| (s as i32 - text_size as i32).abs())
            .map(|(idx, _)| idx)
            .unwrap_or(1)
    } else if tool == Tool::Mosaic {
        MOSAIC_SIZES
            .iter()
            .enumerate()
            .min_by_key(|(_, &s)| (s as i32 - brush_size as i32).abs())
            .map(|(idx, _)| idx)
            .unwrap_or(1)
    } else {
        STROKE_SIZES
            .iter()
            .enumerate()
            .min_by_key(|(_, &s)| (s as i32 - stroke_width as i32).abs())
            .map(|(idx, _)| idx)
            .unwrap_or(1)
    };

    let dot_radii: [i32; 4] = [2, 3, 5, 7]; // 4 档圆点半径 (直径 4, 6, 10, 14)
    let text_labels = ["14", "18", "24", "32"];

    for i in 0..4 {
        let Some((left, top, right, bottom)) = layout.size_rect(i) else {
            continue;
        };
        let is_selected = i == current_size_index;
        let is_hovered = hovered == Some(SecondaryBarItem::Size(i));

        if tool == Tool::Text {
            // 文字工具：绘制清晰的字号选项卡
            if is_selected {
                // 选中高亮状态：淡蓝底色 #E8F3FF + 飞书品牌蓝微边框 #166FF7
                let sel_brush = CreateSolidBrush(0x00fff3e8); // BGR for #E8F3FF
                let sel_pen = CreatePen(0, 1, 0x00f76f16); // BGR for #166FF7
                let old_b = SelectObject(dc, sel_brush as HGDIOBJ);
                let old_p = SelectObject(dc, sel_pen as HGDIOBJ);
                RoundRect(dc, left + 1, top + 2, right - 1, bottom - 2, 6, 6);
                SelectObject(dc, old_b);
                SelectObject(dc, old_p);
                DeleteObject(sel_brush as HGDIOBJ);
                DeleteObject(sel_pen as HGDIOBJ);
            } else if is_hovered {
                // 悬停未选中：极浅灰背景 #F2F3F5
                let hbrush = CreateSolidBrush(0x00f5f3f2); // BGR for #F2F3F5
                let null_p = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_PEN as i32);
                let old_b = SelectObject(dc, hbrush as HGDIOBJ);
                let old_p = SelectObject(dc, null_p);
                RoundRect(dc, left + 1, top + 2, right - 1, bottom - 2, 6, 6);
                SelectObject(dc, old_b);
                SelectObject(dc, old_p);
                DeleteObject(hbrush as HGDIOBJ);
            }

            // 绘制字号数值 (14, 18, 24, 32)
            let text_color = if is_selected {
                0x00f76f16 // 飞书蓝 #166FF7
            } else if is_hovered {
                0x001d2129 // 深黑 #1D2129
            } else {
                0x00594e4e // 中性深灰 #4E5969
            };

            let gui_font = GetStockObject(DEFAULT_GUI_FONT);
            let old_f = SelectObject(dc, gui_font);
            SetBkMode(dc, 1);
            SetTextColor(dc, text_color);
            let text = wide(text_labels[i]);
            let mut text_rect = RECT {
                left: left + 1,
                top: top + 2,
                right: right - 1,
                bottom: bottom - 2,
            };
            DrawTextW(dc, text.as_ptr(), -1, &mut text_rect, 0x1 | 0x4 | 0x20);
            SelectObject(dc, old_f);
        } else {
            let cx = (left + right) / 2;
            let cy = (top + bottom) / 2;

            if is_hovered && !is_selected {
                let hbrush = CreateSolidBrush(0x00f5f2f0);
                let old_b = SelectObject(dc, hbrush as HGDIOBJ);
                RoundRect(dc, left + 2, top + 2, right - 2, bottom - 2, 6, 6);
                SelectObject(dc, old_b);
                DeleteObject(hbrush as HGDIOBJ);
            }

            let r = dot_radii[i];
            let dot_color = if is_selected {
                0x00f76f16 // 飞书蓝：选中时圆点使用品牌蓝 #166FF7 (图 2 规范)
            } else {
                0x008e958f // 未选中时中性灰 #8F958E
            };
            let brush = CreateSolidBrush(dot_color);
            let null_p = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_PEN as i32);
            let old_b = SelectObject(dc, brush as HGDIOBJ);
            let old_p = SelectObject(dc, null_p);
            Ellipse(dc, cx - r, cy - r, cx + r + 1, cy + r + 1);
            SelectObject(dc, old_b);
            SelectObject(dc, old_p);
            DeleteObject(brush as HGDIOBJ);

            // 选中外环：同心飞书蓝圆环
            if is_selected {
                let ring_r = r + 4;
                let ring_pen = CreatePen(0, 1, 0x00f76f16);
                let old_p = SelectObject(dc, ring_pen as HGDIOBJ);
                let null_b = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_BRUSH as i32);
                let old_nb = SelectObject(dc, null_b);
                Ellipse(dc, cx - ring_r, cy - ring_r, cx + ring_r + 1, cy + ring_r + 1);
                SelectObject(dc, old_p);
                SelectObject(dc, old_nb);
                DeleteObject(ring_pen as HGDIOBJ);
            }
        }
    }

    // 2. 绘制分割线（如果有颜色区）
    if let Some(sx) = layout.separator_x() {
        let cy = layout.top + layout.height / 2;
        let sep_pen = CreatePen(0, 1, 0x00e5e6eb);
        let old_p = SelectObject(dc, sep_pen as HGDIOBJ);
        MoveToEx(dc, sx, cy - 8, null_mut());
        LineTo(dc, sx, cy + 8);
        SelectObject(dc, old_p);
        DeleteObject(sep_pen as HGDIOBJ);
    }

    // 3. 绘制 7 色色卡
    if layout.has_color {
        for (i, &color) in COLOR_SWATCHES.iter().enumerate() {
            let Some((left, top, right, bottom)) = layout.color_rect(i) else {
                continue;
            };
            let is_selected = color == annotation_color;
            let is_hovered = hovered == Some(SecondaryBarItem::Color(i));
            let is_white = color == [255, 255, 255, 255];

            // 悬停未选中微外环
            if is_hovered && !is_selected {
                let hpen = CreatePen(0, 1, 0x00a0a5ad);
                let old_hp = SelectObject(dc, hpen as HGDIOBJ);
                let null_b = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_BRUSH as i32);
                let old_nb = SelectObject(dc, null_b);
                RoundRect(dc, left - 2, top - 2, right + 2, bottom + 2, 5, 5);
                SelectObject(dc, old_hp);
                SelectObject(dc, old_nb);
                DeleteObject(hpen as HGDIOBJ);
            }

            let gdi_c = gdi_color(&color);
            let brush = CreateSolidBrush(gdi_c);
            let border_color = if is_white { 0x00d0d5dd } else { gdi_c };
            let pen = CreatePen(0, 1, border_color);
            let old_b = SelectObject(dc, brush as HGDIOBJ);
            let old_p = SelectObject(dc, pen as HGDIOBJ);
            RoundRect(dc, left, top, right, bottom, 4, 4);
            SelectObject(dc, old_b);
            SelectObject(dc, old_p);
            DeleteObject(brush as HGDIOBJ);
            DeleteObject(pen as HGDIOBJ);

            // 选中对勾 ✓ (白底绘制深灰对勾，其他颜色纯白对勾)
            if is_selected {
                let check_color = if is_white { 0x001d2129 } else { 0x00ffffff };
                let pen = CreatePen(0, 2, check_color);
                let old_p = SelectObject(dc, pen as HGDIOBJ);
                let cx = (left + right) / 2;
                let cy = (top + bottom) / 2;
                MoveToEx(dc, cx - 4, cy, null_mut());
                LineTo(dc, cx - 1, cy + 3);
                LineTo(dc, cx + 5, cy - 3);
                SelectObject(dc, old_p);
                DeleteObject(pen as HGDIOBJ);
            }
        }
    }
}

pub(super) fn toolbar_label(index: usize) -> Option<&'static str> {
    crate::toolbar::toolbar_label(index)
}

#[allow(clippy::too_many_arguments, clippy::if_same_then_else)]
pub(super) unsafe fn draw_toolbar_menu_gdi(
    dc: HDC,
    layout: crate::toolbar::ToolbarMenuLayout,
    menu: ToolbarMenu,
    tool: Tool,
    scroll_axis: Option<MatchAxis>,
    hovered: Option<usize>,
    annotation_color: [u8; 4],
    stroke_width: u32,
    brush_size: u32,
    text_size: u32,
    stroke_style: StrokeStyle,
) {
    // 底部 1px 极柔环境微阴影
    let shadow_pen = CreatePen(0, 1, 0x00d8dbe0);
    let null_brush = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_BRUSH as i32);
    let old_p = SelectObject(dc, shadow_pen as HGDIOBJ);
    let old_b = SelectObject(dc, null_brush);
    RoundRect(
        dc,
        layout.left,
        layout.top + 1,
        layout.left + layout.width,
        layout.top + layout.height() + 2,
        10,
        10,
    );
    SelectObject(dc, old_p);
    SelectObject(dc, old_b);
    DeleteObject(shadow_pen as HGDIOBJ);

    // 纯白浮层 + 飞书浅灰边框 (10px 圆角)
    let outer_pen = CreatePen(0, 1, 0x00dce0e6);
    let outer_brush = CreateSolidBrush(0x00ffffff);
    let old_pen = SelectObject(dc, outer_pen as HGDIOBJ);
    let old_brush = SelectObject(dc, outer_brush as HGDIOBJ);
    RoundRect(
        dc,
        layout.left,
        layout.top,
        layout.left + layout.width,
        layout.top + layout.height(),
        10,
        10,
    );
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(outer_pen as HGDIOBJ);
    DeleteObject(outer_brush as HGDIOBJ);

    if layout.header_height > 0 {
        let title = match menu {
            ToolbarMenu::Recognition => "识别方式",
            ToolbarMenu::RecordingFps => "录制设置",
            ToolbarMenu::RecordingExport => "选择导出格式",
            _ => "",
        };
        let subtitle = match menu {
            ToolbarMenu::Recognition => "选择要识别的内容",
            ToolbarMenu::RecordingFps => "MP4 手动停止 · GIF/WebP 最长 10 秒 · 选择录制帧率",
            ToolbarMenu::RecordingExport => "保存录屏文件",
            _ => "",
        };
        draw_menu_text(
            dc,
            layout.left + 12,
            layout.top - 1,
            layout.left + layout.width - 12,
            layout.top + 30,
            title,
        );
        draw_menu_muted_text(
            dc,
            layout.left + 12,
            layout.top + 22,
            layout.left + layout.width - 12,
            layout.top + 50,
            subtitle,
        );
        let separator = CreatePen(0, 1, 0x00e9edf2);
        let old = SelectObject(dc, separator as HGDIOBJ);
        MoveToEx(
            dc,
            layout.left + 12,
            layout.top + layout.header_height,
            null_mut(),
        );
        LineTo(dc, layout.left + layout.width - 12, layout.top + layout.header_height);
        SelectObject(dc, old);
        DeleteObject(separator as HGDIOBJ);
    }

    for index in 0..layout.count {
        let Some((left, top, right, bottom)) = layout.item_rect(index) else {
            continue;
        };
        let center_y = (top + bottom) / 2;
        let selected = match menu {
            ToolbarMenu::Color => COLOR_SWATCHES
                .get(index)
                .is_some_and(|color| *color == annotation_color),
            ToolbarMenu::Size => {
                let sizes = if tool == Tool::Text {
                    &crate::toolbar::TEXT_SIZES[..]
                } else if tool == Tool::Mosaic {
                    &crate::toolbar::MOSAIC_SIZES[..]
                } else {
                    &crate::toolbar::STROKE_SIZES[..]
                };
                sizes.get(index).is_some_and(|size| {
                    if tool == Tool::Text {
                        *size == text_size
                    } else if tool == Tool::Mosaic {
                        *size == brush_size
                    } else {
                        *size == stroke_width
                    }
                })
            }
            ToolbarMenu::Style => {
                matches!(
                    (index, stroke_style),
                    (0, StrokeStyle::Solid) | (1, StrokeStyle::Dashed) | (2, StrokeStyle::Dotted)
                )
            }
            ToolbarMenu::ScrollMode => {
                scroll_axis
                    == Some(if index == 0 {
                        MatchAxis::Vertical
                    } else {
                        MatchAxis::Horizontal
                    })
            }
            ToolbarMenu::Recognition => false,
            ToolbarMenu::RecordingFps => false,
            ToolbarMenu::RecordingExport => false,
        };

        if selected {
            let brush = CreateSolidBrush(0x00fff3e8);
            let pen = CreatePen(0, 1, 0x00ff7033);
            let old_b = SelectObject(dc, brush as HGDIOBJ);
            let old_p = SelectObject(dc, pen as HGDIOBJ);
            RoundRect(dc, left, top, right, bottom, 8, 8);
            SelectObject(dc, old_b);
            SelectObject(dc, old_p);
            DeleteObject(brush as HGDIOBJ);
            DeleteObject(pen as HGDIOBJ);
        } else if hovered == Some(index) {
            let brush = CreateSolidBrush(0x00f5f3f2);
            let old = SelectObject(dc, brush as HGDIOBJ);
            RoundRect(dc, left, top, right, bottom, 8, 8);
            SelectObject(dc, old);
            DeleteObject(brush as HGDIOBJ);
        }

        match menu {
            ToolbarMenu::Color => {
                let color = COLOR_SWATCHES[index];
                let brush = CreateSolidBrush(gdi_color(&color));
                let old = SelectObject(dc, brush as HGDIOBJ);
                Ellipse(dc, left + 10, center_y - 8, left + 26, center_y + 8);
                SelectObject(dc, old);
                DeleteObject(brush as HGDIOBJ);
                if selected {
                    draw_ellipse_border(
                        dc,
                        ScreenRect {
                            left: left + 8,
                            top: center_y - 10,
                            right: left + 28,
                            bottom: center_y + 10,
                        },
                        0x00ff7033,
                        2,
                    );
                }
                let label = format!("颜色 {}", index + 1);
                draw_menu_text(dc, left + 38, top, right - 8, bottom, &label);
            }
            ToolbarMenu::Size => {
                let sizes = if tool == Tool::Text {
                    &crate::toolbar::TEXT_SIZES[..]
                } else if tool == Tool::Mosaic {
                    &crate::toolbar::MOSAIC_SIZES[..]
                } else {
                    &crate::toolbar::STROKE_SIZES[..]
                };
                let value = sizes[index];
                let line_width = if tool == Tool::Text {
                    2
                } else {
                    value.min(8) as i32
                };
                let pen_color = if selected { 0x00ff7033 } else { 0x004e5969 };
                let pen = CreatePen(0, line_width, pen_color);
                let old = SelectObject(dc, pen as HGDIOBJ);
                MoveToEx(dc, left + 10, center_y, null_mut());
                LineTo(dc, left + 38, center_y);
                SelectObject(dc, old);
                DeleteObject(pen as HGDIOBJ);
                let label = if tool == Tool::Text {
                    format!("{} pt", value)
                } else {
                    format!("{} px", value)
                };
                draw_menu_text(dc, left + 48, top, right - 8, bottom, &label);
            }
            ToolbarMenu::Style => {
                let preview = match index {
                    0 => StrokeStyle::Solid,
                    1 => StrokeStyle::Dashed,
                    _ => StrokeStyle::Dotted,
                };
                let line_color = if selected { 0x00ff7033 } else { 0x004e5969 };
                draw_line_gdi_style(
                    dc,
                    ScreenPoint {
                        x: left + 10,
                        y: center_y,
                    },
                    ScreenPoint {
                        x: left + 44,
                        y: center_y,
                    },
                    line_color,
                    stroke_width.max(2) as i32,
                    preview,
                );
                let label = match index {
                    0 => "实线",
                    1 => "虚线",
                    _ => "点线",
                };
                draw_menu_text(dc, left + 54, top, right - 8, bottom, label);
            }
            ToolbarMenu::RecordingFps => {
                let label = if index == 0 {
                    "标准录制     15 FPS"
                } else {
                    "流畅录制     30 FPS"
                };
                draw_menu_text(dc, left + 12, top, right - 8, bottom, label);
            }
            ToolbarMenu::RecordingExport => {
                let label = match index {
                    0 => "MP4 视频",
                    1 => "GIF 动图",
                    _ => "WebP 动图",
                };
                draw_menu_text(dc, left + 12, top, right - 8, bottom, label);
            }
            ToolbarMenu::Recognition => {
                let label = if index == 0 { "提取文字" } else { "识别二维码" };
                draw_menu_text(dc, left + 12, top, right - 8, bottom, label);
            }
            ToolbarMenu::ScrollMode => {
                let label = if index == 0 { "纵向滚动" } else { "横向滚动" };
                draw_menu_text(dc, left + 14, top, right - 8, bottom, label);
            }
        }
    }
}

pub(super) unsafe fn draw_menu_text(
    dc: HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    label: &str,
) {
    let gui_font = GetStockObject(DEFAULT_GUI_FONT);
    let old_f = SelectObject(dc, gui_font);
    SetBkMode(dc, 1);
    SetTextColor(dc, 0x001d2129);
    let text = wide(label);
    let mut rect = RECT {
        left,
        top: top + 7,
        right,
        bottom: bottom - 5,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut rect, 1 | 4);
    SelectObject(dc, old_f);
}

unsafe fn draw_menu_muted_text(
    dc: HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    label: &str,
) {
    let gui_font = GetStockObject(DEFAULT_GUI_FONT);
    let old_f = SelectObject(dc, gui_font);
    SetBkMode(dc, 1);
    SetTextColor(dc, 0x00818b9a);
    let text = wide(label);
    let mut rect = RECT {
        left,
        top: top + 5,
        right,
        bottom: bottom - 3,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut rect, 1 | 4 | 0x20);
    SelectObject(dc, old_f);
}

pub(super) unsafe fn draw_toolbar_tooltip(
    dc: HDC,
    x: i32,
    y: i32,
    layout: ToolbarLayout,
    screen_width: i32,
    screen_height: i32,
    index: usize,
) {
    let Some(label) = toolbar_label(index) else {
        return;
    };
    draw_toolbar_tooltip_label(
        dc,
        x,
        y,
        layout,
        screen_width,
        screen_height,
        index,
        label,
    );
}

pub(super) unsafe fn draw_tool_palette_tooltip(
    dc: HDC,
    x: i32,
    y: i32,
    layout: ToolbarLayout,
    screen_width: i32,
    screen_height: i32,
    index: usize,
) {
    let Some(label) = crate::toolbar::tool_palette_label(index) else {
        return;
    };
    draw_toolbar_tooltip_label(
        dc,
        x,
        y,
        layout,
        screen_width,
        screen_height,
        index,
        label,
    );
}

unsafe fn draw_toolbar_tooltip_label(
    dc: HDC,
    x: i32,
    y: i32,
    layout: ToolbarLayout,
    screen_width: i32,
    screen_height: i32,
    index: usize,
    label: &str,
) {
    let Some((button_x, button_y)) = toolbar_button_origin((x, y), index, layout) else {
        return;
    };
    let width = if label.chars().count() > 5 { 96 } else { 68 };
    let height = 24;
    let center = button_x + TOOLBAR_BUTTON_WIDTH / 2;
    let left = (center - width / 2).clamp(4, (screen_width - width - 4).max(4));
    let below = button_y + TOOLBAR_BUTTON_HEIGHT + 6;
    let top = if below + height <= screen_height - 4 {
        below
    } else {
        (y - height - 6).max(4)
    };

    // 微投影
    let shadow_pen = CreatePen(0, 1, 0x00d0d3d8);
    let null_brush = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_BRUSH as i32);
    let old_sp = SelectObject(dc, shadow_pen as HGDIOBJ);
    let old_sb = SelectObject(dc, null_brush);
    RoundRect(dc, left, top + 1, left + width, top + height + 2, 6, 6);
    SelectObject(dc, old_sp);
    SelectObject(dc, old_sb);
    DeleteObject(shadow_pen as HGDIOBJ);

    let brush = CreateSolidBrush(0x001d2129);
    let old_brush = SelectObject(dc, brush as HGDIOBJ);
    let pen = CreatePen(0, 1, 0x003b3531);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    RoundRect(dc, left, top, left + width, top + height, 6, 6);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);

    let gui_font = GetStockObject(DEFAULT_GUI_FONT);
    let old_f = SelectObject(dc, gui_font);
    SetBkMode(dc, 1);
    SetTextColor(dc, 0x00ffffff);
    let text = wide(label);
    let mut rect = RECT {
        left,
        top: top + 1,
        right: left + width,
        bottom: top + height,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut rect, 0x1 | 0x4 | 0x20);
    SelectObject(dc, old_f);
}

pub(super) unsafe fn draw_secondary_bar_tooltip(
    dc: HDC,
    layout: SecondaryBarLayout,
    screen_width: i32,
    screen_height: i32,
    item: SecondaryBarItem,
    tool: Tool,
) {
    let (cx, label) = match item {
        SecondaryBarItem::Size(i) => {
            let Some((l, _, r, _)) = layout.size_rect(i) else {
                return;
            };
            let text = if tool == Tool::Text {
                match i {
                    0 => "字号: 14",
                    1 => "字号: 18",
                    2 => "字号: 24",
                    3 => "字号: 32",
                    _ => "字号",
                }
            } else if tool == Tool::Mosaic {
                match i {
                    0 => "画笔: 细",
                    1 => "画笔: 中",
                    2 => "画笔: 粗",
                    3 => "画笔: 特粗",
                    _ => "画笔大小",
                }
            } else {
                match i {
                    0 => "粗细: 细",
                    1 => "粗细: 中",
                    2 => "粗细: 粗",
                    3 => "粗细: 特粗",
                    _ => "粗细",
                }
            };
            ((l + r) / 2, text)
        }
        SecondaryBarItem::Color(i) => {
            let Some((l, _, r, _)) = layout.color_rect(i) else {
                return;
            };
            let text = match i {
                0 => "红色",
                1 => "橙色",
                2 => "绿色",
                3 => "蓝色",
                4 => "紫色",
                5 => "黑色",
                6 => "白色",
                _ => "颜色",
            };
            ((l + r) / 2, text)
        }
    };

    let char_count = label.chars().count();
    let width = if char_count > 4 { 72 } else { 52 };
    let height = 24;
    let left = (cx - width / 2).clamp(4, (screen_width - width - 4).max(4));
    let above = layout.top - height - 6;
    let top = if above >= 4 {
        above
    } else {
        (layout.top + layout.height + 6).min(screen_height - height - 4)
    };

    // 微投影
    let shadow_pen = CreatePen(0, 1, 0x00d0d3d8);
    let null_brush = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_BRUSH as i32);
    let old_sp = SelectObject(dc, shadow_pen as HGDIOBJ);
    let old_sb = SelectObject(dc, null_brush);
    RoundRect(dc, left, top + 1, left + width, top + height + 2, 6, 6);
    SelectObject(dc, old_sp);
    SelectObject(dc, old_sb);
    DeleteObject(shadow_pen as HGDIOBJ);

    let brush = CreateSolidBrush(0x001d2129);
    let old_brush = SelectObject(dc, brush as HGDIOBJ);
    let pen = CreatePen(0, 1, 0x003b3531);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    RoundRect(dc, left, top, left + width, top + height, 6, 6);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);

    let gui_font = GetStockObject(DEFAULT_GUI_FONT);
    let old_f = SelectObject(dc, gui_font);
    SetBkMode(dc, 1);
    SetTextColor(dc, 0x00ffffff);
    let text = wide(label);
    let mut rect = RECT {
        left,
        top: top + 1,
        right: left + width,
        bottom: top + height,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut rect, 0x1 | 0x4 | 0x20);
    SelectObject(dc, old_f);
}

static GDI_PLUS_TOKEN: OnceLock<usize> = OnceLock::new();
const TOOLBAR_ICON_SCALE: f32 = 1.3;

fn toolbar_icon_coordinate(value: f32) -> f32 {
    12.0 + (value - 12.0) * TOOLBAR_ICON_SCALE
}

unsafe fn gdiplus_graphics(dc: HDC) -> *mut GpGraphics {
    let token = GDI_PLUS_TOKEN.get_or_init(|| {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: 0,
            SuppressExternalCodecs: 0,
        };
        let mut token = 0usize;
        let status = GdiplusStartup(&mut token, &input, null_mut());
        if status == 0 { token } else { 0 }
    });
    if *token == 0 {
        return null_mut();
    }

    let mut graphics = null_mut();
    if GdipCreateFromHDC(dc, &mut graphics) == 0 {
        GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias8x8);
        GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
        graphics
    } else {
        null_mut()
    }
}

unsafe fn icon_line(
    graphics: *mut GpGraphics,
    pen: *mut GpPen,
    left: i32,
    top: i32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) {
    GdipDrawLine(
        graphics,
        pen,
        left as f32 + toolbar_icon_coordinate(x1),
        top as f32 + toolbar_icon_coordinate(y1),
        left as f32 + toolbar_icon_coordinate(x2),
        top as f32 + toolbar_icon_coordinate(y2),
    );
}

unsafe fn icon_polyline(
    graphics: *mut GpGraphics,
    pen: *mut GpPen,
    left: i32,
    top: i32,
    points: &[(f32, f32)],
) {
    for pair in points.windows(2) {
        icon_line(
            graphics, pen, left, top, pair[0].0, pair[0].1, pair[1].0, pair[1].1,
        );
    }
}

unsafe fn icon_bezier(
    graphics: *mut GpGraphics,
    pen: *mut GpPen,
    left: i32,
    top: i32,
    points: [(f32, f32); 4],
) {
    GdipDrawBezier(
        graphics,
        pen,
        left as f32 + toolbar_icon_coordinate(points[0].0),
        top as f32 + toolbar_icon_coordinate(points[0].1),
        left as f32 + toolbar_icon_coordinate(points[1].0),
        top as f32 + toolbar_icon_coordinate(points[1].1),
        left as f32 + toolbar_icon_coordinate(points[2].0),
        top as f32 + toolbar_icon_coordinate(points[2].1),
        left as f32 + toolbar_icon_coordinate(points[3].0),
        top as f32 + toolbar_icon_coordinate(points[3].1),
    );
}

unsafe fn icon_ellipse(
    graphics: *mut GpGraphics,
    pen: *mut GpPen,
    left: i32,
    top: i32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    GdipDrawEllipse(
        graphics,
        pen,
        left as f32 + toolbar_icon_coordinate(x),
        top as f32 + toolbar_icon_coordinate(y),
        width * TOOLBAR_ICON_SCALE,
        height * TOOLBAR_ICON_SCALE,
    );
}

unsafe fn icon_round_rectangle(
    graphics: *mut GpGraphics,
    pen: *mut GpPen,
    left: i32,
    top: i32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
) {
    let scaled_x = toolbar_icon_coordinate(x);
    let scaled_y = toolbar_icon_coordinate(y);
    let scaled_width = width * TOOLBAR_ICON_SCALE;
    let scaled_height = height * TOOLBAR_ICON_SCALE;
    let diameter = radius * 2.0 * TOOLBAR_ICON_SCALE;
    icon_line(graphics, pen, left, top, x + radius, y, x + width - radius, y);
    GdipDrawArc(
        graphics,
        pen,
        left as f32 + scaled_x + scaled_width - diameter,
        top as f32 + scaled_y,
        diameter,
        diameter,
        -90.0,
        90.0,
    );
    icon_line(
        graphics,
        pen,
        left,
        top,
        x + width,
        y + radius,
        x + width,
        y + height - radius,
    );
    GdipDrawArc(
        graphics,
        pen,
        left as f32 + scaled_x + scaled_width - diameter,
        top as f32 + scaled_y + scaled_height - diameter,
        diameter,
        diameter,
        0.0,
        90.0,
    );
    icon_line(
        graphics,
        pen,
        left,
        top,
        x + width - radius,
        y + height,
        x + radius,
        y + height,
    );
    GdipDrawArc(
        graphics,
        pen,
        left as f32 + scaled_x,
        top as f32 + scaled_y + scaled_height - diameter,
        diameter,
        diameter,
        90.0,
        90.0,
    );
    icon_line(
        graphics,
        pen,
        left,
        top,
        x,
        y + height - radius,
        x,
        y + radius,
    );
    GdipDrawArc(
        graphics,
        pen,
        left as f32 + scaled_x,
        top as f32 + scaled_y,
        diameter,
        diameter,
        180.0,
        90.0,
    );
}

unsafe fn icon_filled_dot(
    graphics: *mut GpGraphics,
    color: u32,
    left: i32,
    top: i32,
    x: f32,
    y: f32,
    size: f32,
) {
    let mut brush: *mut GpSolidFill = null_mut();
    if GdipCreateSolidFill(color, &mut brush) == 0 {
        GdipFillEllipse(
            graphics,
            brush as *mut GpBrush,
            left as f32 + toolbar_icon_coordinate(x),
            top as f32 + toolbar_icon_coordinate(y),
            size * TOOLBAR_ICON_SCALE,
            size * TOOLBAR_ICON_SCALE,
        );
        GdipDeleteBrush(brush as *mut GpBrush);
    }
}

unsafe fn icon_filled_rectangle(
    graphics: *mut GpGraphics,
    color: u32,
    left: i32,
    top: i32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    let mut brush: *mut GpSolidFill = null_mut();
    if GdipCreateSolidFill(color, &mut brush) == 0 {
        GdipFillRectangle(
            graphics,
            brush as *mut GpBrush,
            left as f32 + toolbar_icon_coordinate(x),
            top as f32 + toolbar_icon_coordinate(y),
            width * TOOLBAR_ICON_SCALE,
            height * TOOLBAR_ICON_SCALE,
        );
        GdipDeleteBrush(brush as *mut GpBrush);
    }
}

unsafe fn draw_toolbar_icon_shape(
    graphics: *mut GpGraphics,
    pen: *mut GpPen,
    color: u32,
    index: usize,
    left: i32,
    top: i32,
) {
    match index {
        // Six-dot grip like the reference toolbar.
        0 => {
            for y in [6.0, 12.0, 18.0] {
                for x in [8.0, 16.0] {
                    icon_filled_dot(graphics, color, left, top, x - 1.2, y - 1.2, 2.4);
                }
            }
        }
        1 => icon_round_rectangle(graphics, pen, left, top, 4.0, 4.0, 16.0, 16.0, 1.5),
        2 => icon_ellipse(graphics, pen, left, top, 4.0, 4.0, 16.0, 16.0),
        3 => {
            icon_line(graphics, pen, left, top, 3.0, 12.0, 20.0, 12.0);
            icon_polyline(graphics, pen, left, top, &[(14.0, 7.0), (20.0, 12.0), (14.0, 17.0)]);
        }
        // Pen stroke with a small freehand flourish.
        4 => {
            icon_polyline(
                graphics,
                pen,
                left,
                top,
                &[(6.0, 16.0), (15.5, 5.5), (19.0, 9.0), (9.5, 19.5), (6.0, 16.0)],
            );
            icon_line(graphics, pen, left, top, 9.0, 13.0, 12.5, 16.5);
            icon_bezier(graphics, pen, left, top, [(3.0, 20.0), (5.0, 16.5), (7.0, 22.0), (9.0, 18.5)]);
            icon_bezier(graphics, pen, left, top, [(9.0, 18.5), (12.0, 15.5), (13.0, 22.0), (17.0, 18.0)]);
        }
        // Mosaic: a compact tile mark with one filled cell.
        5 => {
            icon_round_rectangle(graphics, pen, left, top, 4.0, 4.0, 16.0, 16.0, 1.5);
            icon_line(graphics, pen, left, top, 12.0, 4.5, 12.0, 19.5);
            icon_line(graphics, pen, left, top, 4.5, 12.0, 19.5, 12.0);
            icon_filled_rectangle(graphics, color, left, top, 13.0, 5.0, 6.0, 6.0);
        }
        6 => {
            icon_line(graphics, pen, left, top, 3.5, 5.0, 20.5, 5.0);
            icon_line(graphics, pen, left, top, 12.0, 5.0, 12.0, 20.0);
        }
        // Tag-shaped number marker.
        7 => {
            icon_polyline(
                graphics,
                pen,
                left,
                top,
                &[(4.0, 6.0), (13.0, 6.0), (20.0, 13.0), (13.0, 20.0), (4.0, 11.0), (4.0, 6.0)],
            );
            icon_ellipse(graphics, pen, left, top, 7.0, 8.0, 2.0, 2.0);
        }
        // Pinned screenshot tile with a clear plus badge.
        8 => {
            icon_round_rectangle(graphics, pen, left, top, 3.5, 5.0, 14.0, 14.0, 1.5);
            icon_ellipse(graphics, pen, left, top, 7.0, 8.0, 2.5, 2.5);
            icon_polyline(graphics, pen, left, top, &[(5.0, 17.0), (9.0, 13.0), (12.0, 16.0), (15.0, 12.5), (17.5, 15.5)]);
            icon_line(graphics, pen, left, top, 19.0, 4.0, 19.0, 11.0);
            icon_line(graphics, pen, left, top, 15.5, 7.5, 22.5, 7.5);
        }
        // Long capture page with a vertical scroll cue.
        9 => {
            icon_round_rectangle(graphics, pen, left, top, 5.0, 3.0, 14.0, 18.0, 1.5);
            icon_line(graphics, pen, left, top, 8.0, 8.0, 14.0, 8.0);
            icon_line(graphics, pen, left, top, 8.0, 11.0, 12.0, 11.0);
            icon_line(graphics, pen, left, top, 16.0, 8.0, 16.0, 17.0);
            icon_polyline(graphics, pen, left, top, &[(14.0, 10.0), (16.0, 8.0), (18.0, 10.0)]);
            icon_polyline(graphics, pen, left, top, &[(14.0, 15.0), (16.0, 17.0), (18.0, 15.0)]);
        }
        10 => {
            icon_bezier(graphics, pen, left, top, [(19.0, 17.0), (20.0, 8.0), (13.0, 5.0), (8.0, 8.0)]);
            icon_polyline(graphics, pen, left, top, &[(8.5, 4.0), (5.0, 8.0), (10.0, 8.5)]);
        }
        // Download arrow and tray.
        11 => {
            icon_line(graphics, pen, left, top, 12.0, 3.5, 12.0, 15.0);
            icon_polyline(graphics, pen, left, top, &[(7.5, 11.0), (12.0, 15.5), (16.5, 11.0)]);
            icon_polyline(graphics, pen, left, top, &[(4.0, 17.0), (4.0, 20.0), (20.0, 20.0), (20.0, 17.0)]);
        }
        12 => {
            icon_line(graphics, pen, left, top, 5.0, 5.0, 19.0, 19.0);
            icon_line(graphics, pen, left, top, 19.0, 5.0, 5.0, 19.0);
        }
        13 => icon_polyline(graphics, pen, left, top, &[(4.5, 12.5), (9.5, 17.5), (20.0, 6.5)]),
        // Pencil used by the edit action.
        14 => {
            icon_polyline(graphics, pen, left, top, &[(5.0, 17.5), (16.5, 6.0), (19.0, 8.5), (7.5, 20.0), (5.0, 20.5), (5.0, 17.5)]);
            icon_line(graphics, pen, left, top, 14.0, 8.5, 16.5, 11.0);
        }
        15 => {
            icon_ellipse(graphics, pen, left, top, 3.5, 3.5, 17.0, 17.0);
            icon_filled_dot(graphics, color, left, top, 9.0, 9.0, 6.0);
        }
        // Scan corners and short text rows for OCR.
        16 => {
            for (x, y, sx, sy) in [
                (4.0, 8.0, 1.0, 1.0),
                (20.0, 8.0, -1.0, 1.0),
                (4.0, 16.0, 1.0, -1.0),
                (20.0, 16.0, -1.0, -1.0),
            ] {
                icon_line(graphics, pen, left, top, x, y, x + sx * 4.0, y);
                icon_line(graphics, pen, left, top, x, y, x, y + sy * 3.0);
            }
            icon_line(graphics, pen, left, top, 9.0, 10.0, 15.0, 10.0);
            icon_line(graphics, pen, left, top, 9.0, 13.0, 15.0, 13.0);
        }
        17 => icon_line(graphics, pen, left, top, 3.5, 12.0, 20.5, 12.0),
        // Broad marker body and the highlight stroke it leaves behind.
        18 => {
            icon_polyline(graphics, pen, left, top, &[(6.0, 15.5), (15.5, 6.0), (19.0, 9.5), (9.5, 19.0), (6.0, 15.5)]);
            icon_line(graphics, pen, left, top, 5.0, 20.0, 10.0, 20.0);
            icon_line(graphics, pen, left, top, 8.0, 13.5, 11.5, 17.0);
        }
        // Plugin mark: crop corners around a lens and two pixel accents.
        19 => {
            icon_polyline(graphics, pen, left, top, &[(8.0, 3.5), (4.0, 3.5), (4.0, 8.0)]);
            icon_polyline(graphics, pen, left, top, &[(16.0, 3.5), (20.0, 3.5), (20.0, 8.0)]);
            icon_polyline(graphics, pen, left, top, &[(20.0, 16.0), (20.0, 20.0), (16.0, 20.0)]);
            icon_polyline(graphics, pen, left, top, &[(8.0, 20.0), (4.0, 20.0), (4.0, 16.0)]);
            icon_ellipse(graphics, pen, left, top, 9.0, 9.0, 6.0, 6.0);
            icon_filled_dot(graphics, color, left, top, 17.0, 6.0, 1.8);
            icon_filled_dot(graphics, color, left, top, 5.5, 17.0, 1.8);
        }
        // Selection cursor.
        20 => {
            icon_polyline(graphics, pen, left, top, &[(5.0, 3.5), (5.0, 19.5), (9.5, 15.0), (12.5, 21.0), (15.0, 19.5), (12.0, 13.5), (19.0, 13.5), (5.0, 3.5)]);
        }
        // Two overlapping frames for copy.
        100 => {
            icon_round_rectangle(graphics, pen, left, top, 8.0, 4.0, 12.0, 14.0, 1.5);
            icon_round_rectangle(graphics, pen, left, top, 4.0, 8.0, 12.0, 12.0, 1.5);
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_toolbar_icon(
    dc: HDC,
    index: usize,
    x: i32,
    y: i32,
    _active_tool: Tool,
    icon_color: u32,
    _annotation_color: [u8; 4],
    _stroke_width: u32,
    _brush_size: u32,
    _text_size: u32,
    _stroke_style: StrokeStyle,
) {
    let graphics = gdiplus_graphics(dc);
    if graphics.is_null() {
        return;
    }

    let red = icon_color & 0xff;
    let green = (icon_color >> 8) & 0xff;
    let blue = (icon_color >> 16) & 0xff;
    let color = 0xff00_0000 | (red << 16) | (green << 8) | blue;
    let mut pen: *mut GpPen = null_mut();
    if GdipCreatePen1(color, 2.0, UnitPixel, &mut pen) == 0 {
        GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapRound);
        GdipSetPenLineJoin(pen, LineJoinRound);
        draw_toolbar_icon_shape(graphics, pen, color, index, x - 12, y - 12);
        GdipDeletePen(pen);
    }
    GdipDeleteGraphics(graphics);
}
