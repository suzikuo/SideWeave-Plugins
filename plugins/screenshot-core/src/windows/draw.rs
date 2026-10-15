use super::{wide, DEFAULT_GUI_FONT};
use crate::capture::ScreenCapture;
use crate::model::{Annotation, ScreenPoint, ScreenRect, StrokeStyle};
use crate::session::CaptureSession;
use crate::toolbar::Tool;

use std::ptr::null_mut;
use windows_sys::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, Ellipse, FillRect, GetStockObject,
    LineTo, MoveToEx, RoundRect, SelectObject, SetBkMode, SetTextColor, HDC, HGDIOBJ,
};
use windows_sys::Win32::Foundation::RECT;

pub(super) fn gdi_color(color: &[u8; 4]) -> u32 {
    u32::from(color[0]) | (u32::from(color[1]) << 8) | (u32::from(color[2]) << 16)
}

pub(super) fn gdi_pen_style(style: StrokeStyle) -> i32 {
    match style {
        StrokeStyle::Solid => 0,
        StrokeStyle::Dashed => 1,
        StrokeStyle::Dotted => 2,
    }
}

pub(super) fn draw_annotations_gdi(
    dc: HDC,
    capture: &CaptureSession,
    screen: &ScreenCapture,
    ox: i32,
    oy: i32,
) {
    unsafe {
        for annotation in &capture.annotations {
            match annotation {
                Annotation::Rect(rect, color, width, style) => {
                    draw_rect_border_style(
                        dc,
                        rect.left - ox,
                        rect.top - oy,
                        rect.width() as i32,
                        rect.height() as i32,
                        gdi_color(color),
                        (*width).max(1) as i32,
                        *style,
                    );
                }
                Annotation::Ellipse(rect, color, width, style) => {
                    let local = ScreenRect {
                        left: rect.left - ox,
                        top: rect.top - oy,
                        right: rect.right - ox,
                        bottom: rect.bottom - oy,
                    };
                    draw_ellipse_border_style(
                        dc,
                        local,
                        gdi_color(color),
                        (*width).max(1) as i32,
                        *style,
                    );
                }
                Annotation::Arrow(start, end, color, width, style) => {
                    draw_arrow_gdi_style(
                        dc,
                        ScreenPoint {
                            x: start.x - ox,
                            y: start.y - oy,
                        },
                        ScreenPoint {
                            x: end.x - ox,
                            y: end.y - oy,
                        },
                        gdi_color(color),
                        (*width).max(1) as i32,
                        *style,
                    );
                }
                Annotation::Line(start, end, color, width, style) => {
                    draw_line_gdi_style(
                        dc,
                        ScreenPoint {
                            x: start.x - ox,
                            y: start.y - oy,
                        },
                        ScreenPoint {
                            x: end.x - ox,
                            y: end.y - oy,
                        },
                        gdi_color(color),
                        (*width).max(1) as i32,
                        *style,
                    );
                }
                Annotation::Highlight(start, end, color, width) => {
                    draw_line_gdi_style(
                        dc,
                        ScreenPoint {
                            x: start.x - ox,
                            y: start.y - oy,
                        },
                        ScreenPoint {
                            x: end.x - ox,
                            y: end.y - oy,
                        },
                        gdi_color(color),
                        (*width).max(1) as i32,
                        StrokeStyle::Solid,
                    );
                }
                Annotation::Number(point, number, color) => {
                    draw_number_gdi(
                        dc,
                        ScreenPoint {
                            x: point.x - ox,
                            y: point.y - oy,
                        },
                        *number,
                        gdi_color(color),
                    );
                }
                Annotation::Pen(points, color, width, style) => {
                    let pen = CreatePen(
                        gdi_pen_style(*style),
                        (*width).max(1) as i32,
                        gdi_color(color),
                    );
                    let old_pen = SelectObject(dc, pen as HGDIOBJ);
                    for pair in points.windows(2) {
                        MoveToEx(dc, pair[0].x - ox, pair[0].y - oy, null_mut());
                        LineTo(dc, pair[1].x - ox, pair[1].y - oy);
                    }
                    if points.len() == 1 {
                        Ellipse(
                            dc,
                            points[0].x - ox - (*width as i32 / 2),
                            points[0].y - oy - (*width as i32 / 2),
                            points[0].x - ox + (*width as i32 / 2 + 1),
                            points[0].y - oy + (*width as i32 / 2 + 1),
                        );
                    }
                    SelectObject(dc, old_pen);
                    DeleteObject(pen as HGDIOBJ);
                }
                Annotation::Mosaic(points, brush_size) => {
                    draw_mosaic_brush_preview(dc, screen, points, *brush_size, ox, oy);
                }
                Annotation::Text(point, text, size, color) if !text.is_empty() => {
                    SetBkMode(dc, 1);
                    SetTextColor(dc, gdi_color(color));
                    let text = wide(text);
                    let mut rect = RECT {
                        left: point.x - ox,
                        top: point.y - oy,
                        right: point.x - ox + 1200,
                        bottom: point.y - oy + (*size as i32 * 2).max(24),
                    };
                    let gui_font = GetStockObject(DEFAULT_GUI_FONT);
                    let old_f = SelectObject(dc, gui_font);
                    DrawTextW(dc, text.as_ptr(), -1, &mut rect, 0);
                    SelectObject(dc, old_f);
                }
                Annotation::Text(_, _, _, _) => {}
            }
        }
    }
}

pub(super) unsafe fn draw_line_gdi_style(
    dc: HDC,
    start: ScreenPoint,
    end: ScreenPoint,
    color: u32,
    thickness: i32,
    style: StrokeStyle,
) {
    let pen = CreatePen(gdi_pen_style(style), thickness.max(1), color);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    MoveToEx(dc, start.x, start.y, null_mut());
    LineTo(dc, end.x, end.y);
    SelectObject(dc, old_pen);
    DeleteObject(pen as HGDIOBJ);
}

pub(super) unsafe fn draw_number_gdi(dc: HDC, center: ScreenPoint, number: u32, color: u32) {
    let pen = CreatePen(0, 1, color);
    let brush = CreateSolidBrush(color);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    let old_brush = SelectObject(dc, brush as HGDIOBJ);
    Ellipse(
        dc,
        center.x - 14,
        center.y - 14,
        center.x + 14,
        center.y + 14,
    );
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(pen as HGDIOBJ);
    DeleteObject(brush as HGDIOBJ);

    SetBkMode(dc, 1);
    SetTextColor(dc, 0x00ffffff);
    let text = wide(&number.to_string());
    let mut rect = RECT {
        left: center.x - 12,
        top: center.y - 12,
        right: center.x + 12,
        bottom: center.y + 12,
    };
    let gui_font = GetStockObject(DEFAULT_GUI_FONT);
    let old_f = SelectObject(dc, gui_font);
    DrawTextW(dc, text.as_ptr(), -1, &mut rect, 1 | 4);
    SelectObject(dc, old_f);
}

#[allow(clippy::manual_checked_ops)]
pub(super) fn draw_mosaic_preview(
    dc: HDC,
    screen: &ScreenCapture,
    requested: ScreenRect,
    ox: i32,
    oy: i32,
) {
    let bounds = ScreenRect {
        left: screen.x,
        top: screen.y,
        right: screen.x + screen.width,
        bottom: screen.y + screen.height,
    };
    let Some(rect) = requested.clipped_to(bounds) else {
        return;
    };
    const BLOCK: i32 = 10;
    unsafe {
        let mut by = rect.top;
        while by < rect.bottom {
            let mut bx = rect.left;
            while bx < rect.right {
                let end_x = (bx + BLOCK).min(rect.right);
                let end_y = (by + BLOCK).min(rect.bottom);
                let sx0 = (bx - screen.x) as usize;
                let sy0 = (by - screen.y) as usize;
                let sx1 = (end_x - screen.x) as usize;
                let sy1 = (end_y - screen.y) as usize;
                let mut sum = [0u32; 3];
                let mut count = 0u32;
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        let index = (sy * screen.width as usize + sx) * 4;
                        sum[0] += screen.pixels[index + 2] as u32;
                        sum[1] += screen.pixels[index + 1] as u32;
                        sum[2] += screen.pixels[index] as u32;
                        count += 1;
                    }
                }
                if count > 0 {
                    let color =
                        (sum[0] / count) | ((sum[1] / count) << 8) | ((sum[2] / count) << 16);
                    let brush = CreateSolidBrush(color);
                    let cell = RECT {
                        left: bx - ox,
                        top: by - oy,
                        right: end_x - ox,
                        bottom: end_y - oy,
                    };
                    FillRect(dc, &cell, brush);
                    DeleteObject(brush as HGDIOBJ);
                }
                bx += BLOCK;
            }
            by += BLOCK;
        }
        draw_rect_border(
            dc,
            rect.left - ox,
            rect.top - oy,
            rect.width() as i32,
            rect.height() as i32,
            0x00ff7048,
            2,
        );
    }
}

pub(super) fn draw_mosaic_brush_preview(
    dc: HDC,
    screen: &ScreenCapture,
    points: &[ScreenPoint],
    brush_size: u32,
    ox: i32,
    oy: i32,
) {
    let size = brush_size.max(8) as i32;
    let half = size / 2;
    for point in points {
        draw_mosaic_preview(
            dc,
            screen,
            ScreenRect {
                left: point.x - half,
                top: point.y - half,
                right: point.x + half,
                bottom: point.y + half,
            },
            ox,
            oy,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_preview(
    dc: HDC,
    screen: &ScreenCapture,
    tool: Tool,
    start: ScreenPoint,
    current: ScreenPoint,
    ox: i32,
    oy: i32,
    stroke_width: u32,
    stroke_style: StrokeStyle,
    brush_size: u32,
) {
    unsafe {
        let a = ScreenPoint {
            x: start.x - ox,
            y: start.y - oy,
        };
        let b = ScreenPoint {
            x: current.x - ox,
            y: current.y - oy,
        };
        let thickness = stroke_width.max(1) as i32;
        match tool {
            Tool::Rectangle => {
                let rect = ScreenRect::normalized(a, b);
                draw_rect_border_style(
                    dc,
                    rect.left,
                    rect.top,
                    rect.width() as i32,
                    rect.height() as i32,
                    0x00ff7048,
                    thickness,
                    stroke_style,
                );
            }
            Tool::Ellipse => {
                let rect = ScreenRect::normalized(a, b);
                draw_ellipse_border_style(dc, rect, 0x00ff7048, thickness, stroke_style);
            }
            Tool::Arrow => draw_arrow_gdi_style(dc, a, b, 0x00ff7048, thickness, stroke_style),
            Tool::Line => draw_line_gdi_style(dc, a, b, 0x00ff7048, thickness, stroke_style),
            Tool::Highlight => draw_line_gdi_style(
                dc,
                a,
                b,
                0x00ff7048,
                thickness.saturating_mul(6).max(10),
                StrokeStyle::Solid,
            ),
            Tool::Mosaic => draw_mosaic_brush_preview(dc, screen, &[current], brush_size, ox, oy),
            _ => {}
        }
    }
}

pub(super) unsafe fn draw_selection_handles_gdi(
    dc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let points = [
        (x, y),
        (x + width / 2, y),
        (x + width, y),
        (x, y + height / 2),
        (x + width, y + height / 2),
        (x, y + height),
        (x + width / 2, y + height),
        (x + width, y + height),
    ];
    let null_pen = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_PEN as i32);
    let blue_brush = CreateSolidBrush(0x00f76f16); // 飞书品牌蓝 #166FF7
    let white_brush = CreateSolidBrush(0x00ffffff);
    let old_p = SelectObject(dc, null_pen);

    // 外层高精蓝圈 (直径 9px)
    let old_b = SelectObject(dc, blue_brush as HGDIOBJ);
    for (center_x, center_y) in points {
        Ellipse(dc, center_x - 4, center_y - 4, center_x + 5, center_y + 5);
    }

    // 内层纯白同心点 (直径 5px，对标飞书视网膜级控制点)
    SelectObject(dc, white_brush as HGDIOBJ);
    for (center_x, center_y) in points {
        Ellipse(dc, center_x - 2, center_y - 2, center_x + 3, center_y + 3);
    }

    SelectObject(dc, old_b);
    SelectObject(dc, old_p);
    DeleteObject(blue_brush as HGDIOBJ);
    DeleteObject(white_brush as HGDIOBJ);
}

pub(super) unsafe fn draw_size_badge(dc: HDC, x: i32, y: i32, width: i32, height: i32) {
    let formatted = format!("{} \u{00D7} {}", width, height);
    let text = wide(&formatted);
    let badge_width = 80.max((formatted.len() as i32) * 8 + 16);
    let badge_height = 24;
    let left = x.max(4);
    let top = if y >= badge_height + 8 { y - badge_height - 6 } else { y + 6 };

    // 微投影
    let shadow_pen = CreatePen(0, 1, 0x00d0d3d8);
    let null_brush = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_BRUSH as i32);
    let old_sp = SelectObject(dc, shadow_pen as HGDIOBJ);
    let old_sb = SelectObject(dc, null_brush);
    RoundRect(dc, left, top + 1, left + badge_width, top + badge_height + 2, 6, 6);
    SelectObject(dc, old_sp);
    SelectObject(dc, old_sb);
    DeleteObject(shadow_pen as HGDIOBJ);

    // 飞书深色高质感胶囊 #1F2329，微圆角 4px，1px 微光边框 #333842
    let brush = CreateSolidBrush(0x001d2129);
    let pen = CreatePen(0, 1, 0x003b3531);
    let old_b = SelectObject(dc, brush as HGDIOBJ);
    let old_p = SelectObject(dc, pen as HGDIOBJ);
    RoundRect(dc, left, top, left + badge_width, top + badge_height, 6, 6);
    SelectObject(dc, old_b);
    SelectObject(dc, old_p);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);

    let gui_font = GetStockObject(DEFAULT_GUI_FONT);
    let old_f = SelectObject(dc, gui_font);
    SetBkMode(dc, 1);
    SetTextColor(dc, 0x00ffffff);
    let mut text_rect = RECT {
        left,
        top: top + 1,
        right: left + badge_width,
        bottom: top + badge_height,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut text_rect, 0x1 | 0x4 | 0x20);
    SelectObject(dc, old_f);
}

pub(super) unsafe fn draw_banner(dc: HDC, x: i32, y: i32, value: &str) {
    let banner_width = 280;
    let banner_height = 30;

    // 柔和微投影
    let shadow_pen = CreatePen(0, 1, 0x00d0d3d8);
    let null_brush = GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_BRUSH as i32);
    let old_sp = SelectObject(dc, shadow_pen as HGDIOBJ);
    let old_sb = SelectObject(dc, null_brush);
    RoundRect(dc, x, y + 1, x + banner_width, y + banner_height + 2, 8, 8);
    SelectObject(dc, old_sp);
    SelectObject(dc, old_sb);
    DeleteObject(shadow_pen as HGDIOBJ);

    let brush = CreateSolidBrush(0x001d2129);
    let pen = CreatePen(0, 1, 0x003b3531);
    let old_b = SelectObject(dc, brush as HGDIOBJ);
    let old_p = SelectObject(dc, pen as HGDIOBJ);
    RoundRect(dc, x, y, x + banner_width, y + banner_height, 8, 8);
    SelectObject(dc, old_b);
    SelectObject(dc, old_p);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);

    let gui_font = GetStockObject(DEFAULT_GUI_FONT);
    let old_f = SelectObject(dc, gui_font);
    SetBkMode(dc, 1);
    SetTextColor(dc, 0x00f0f2f5);
    let text = wide(value);
    let mut text_rect = RECT {
        left: x + 8,
        top: y + 1,
        right: x + banner_width - 8,
        bottom: y + banner_height,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut text_rect, 0x1 | 0x4 | 0x20);
    SelectObject(dc, old_f);
}

pub(super) unsafe fn draw_rect_border(
    dc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: u32,
    thickness: i32,
) {
    draw_rect_border_style(
        dc,
        x,
        y,
        width,
        height,
        color,
        thickness,
        StrokeStyle::Solid,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_rect_border_style(
    dc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: u32,
    thickness: i32,
    style: StrokeStyle,
) {
    let pen = CreatePen(gdi_pen_style(style), thickness.max(1), color);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    let old_brush = SelectObject(dc, GetStockObject(5));
    windows_sys::Win32::Graphics::Gdi::Rectangle(dc, x, y, x + width, y + height);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(pen as HGDIOBJ);
}

pub(super) unsafe fn draw_ellipse_border(dc: HDC, rect: ScreenRect, color: u32, thickness: i32) {
    draw_ellipse_border_style(dc, rect, color, thickness, StrokeStyle::Solid);
}

pub(super) unsafe fn draw_ellipse_border_style(
    dc: HDC,
    rect: ScreenRect,
    color: u32,
    thickness: i32,
    style: StrokeStyle,
) {
    let pen = CreatePen(gdi_pen_style(style), thickness.max(1), color);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    let old_brush = SelectObject(dc, GetStockObject(5));
    Ellipse(
        dc,
        rect.left,
        rect.top,
        rect.right.max(rect.left + 1),
        rect.bottom.max(rect.top + 1),
    );
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(pen as HGDIOBJ);
}

#[allow(dead_code)]
pub(super) unsafe fn draw_arrow_gdi(
    dc: HDC,
    a: ScreenPoint,
    b: ScreenPoint,
    color: u32,
    thickness: i32,
) {
    draw_arrow_gdi_style(dc, a, b, color, thickness, StrokeStyle::Solid);
}

pub(super) unsafe fn draw_arrow_gdi_style(
    dc: HDC,
    a: ScreenPoint,
    b: ScreenPoint,
    color: u32,
    thickness: i32,
    style: StrokeStyle,
) {
    let pen = CreatePen(gdi_pen_style(style), thickness.max(1), color);
    let old_pen = SelectObject(dc, pen as HGDIOBJ);
    MoveToEx(dc, a.x, a.y, null_mut());
    LineTo(dc, b.x, b.y);
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let length = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / length;
    let uy = dy / length;
    let left = ScreenPoint {
        x: b.x - (ux * 12.0 - uy * 6.0) as i32,
        y: b.y - (uy * 12.0 + ux * 6.0) as i32,
    };
    let right = ScreenPoint {
        x: b.x - (ux * 12.0 + uy * 6.0) as i32,
        y: b.y - (uy * 12.0 - ux * 6.0) as i32,
    };
    MoveToEx(dc, b.x, b.y, null_mut());
    LineTo(dc, left.x, left.y);
    MoveToEx(dc, b.x, b.y, null_mut());
    LineTo(dc, right.x, right.y);
    SelectObject(dc, old_pen);
    DeleteObject(pen as HGDIOBJ);
}
