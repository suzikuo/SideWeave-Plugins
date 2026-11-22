use super::{wide, DEFAULT_GUI_FONT};
use crate::capture::ScreenCapture;
use crate::export::{bubble_points, burst_points, star_points, triangle_points};
use crate::model::{
    Annotation, ArrowVariant, OcrRegionBox, ScreenPoint, ScreenRect, StrokeStyle,
};
use crate::session::CaptureSession;
use crate::theme::ThemePalette;
use crate::toolbar::{InfoCapsuleLayout, Tool};

use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    AlphaBlend, CombineRgn, CreateCompatibleBitmap, CreateCompatibleDC, CreateEllipticRgn,
    CreateFontW, CreatePen, CreateRectRgn, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW,
    Ellipse, FillRect, GetStockObject, LineTo, MoveToEx, Polygon, Rectangle, RestoreDC, RoundRect,
    SaveDC, SelectClipRgn, SelectObject, SetBkMode, SetTextColor, BLENDFUNCTION, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_SEMIBOLD, HDC, HGDIOBJ,
    NULL_BRUSH, NULL_PEN, OUT_DEFAULT_PRECIS, RGN_DIFF, RGN_OR,
};

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
        draw_spotlight_mask(dc, capture, ox, oy);
        for annotation in &capture.annotations {
            draw_single_annotation_gdi(dc, annotation, screen, ox, oy);
        }
    }
}

pub(super) unsafe fn draw_single_annotation_gdi(
    dc: HDC,
    annotation: &Annotation,
    screen: &ScreenCapture,
    ox: i32,
    oy: i32,
) {
    let origin = ScreenRect {
        left: ox,
        top: oy,
        right: ox,
        bottom: oy,
    };
    match annotation {
        Annotation::Rect {
            rect,
            color,
            width,
            style,
            filled,
            rounded,
        } => {
            let l = rect.left - ox;
            let t = rect.top - oy;
            let w = rect.width() as i32;
            let h = rect.height() as i32;
            let c = gdi_color(color);
            if *filled {
                let brush = CreateSolidBrush(c);
                let old_b = SelectObject(dc, brush as HGDIOBJ);
                let old_p = SelectObject(dc, GetStockObject(NULL_PEN as i32));
                if *rounded {
                    let rad = (w.min(h) / 3).clamp(12, 36);
                    RoundRect(dc, l, t, l + w, t + h, rad, rad);
                } else {
                    Rectangle(dc, l, t, l + w, t + h);
                }
                SelectObject(dc, old_p);
                SelectObject(dc, old_b);
                DeleteObject(brush as HGDIOBJ);
            } else if *rounded {
                let pen = CreatePen(gdi_pen_style(*style), (*width).max(1) as i32, c);
                let old_p = SelectObject(dc, pen as HGDIOBJ);
                let old_b = SelectObject(dc, GetStockObject(NULL_BRUSH as i32));
                let rad = (w.min(h) / 3).clamp(12, 36);
                RoundRect(dc, l, t, l + w, t + h, rad, rad);
                SelectObject(dc, old_b);
                SelectObject(dc, old_p);
                DeleteObject(pen as HGDIOBJ);
            } else {
                draw_rect_border_style(dc, l, t, w, h, c, (*width).max(1) as i32, *style);
            }
        }
        Annotation::Ellipse {
            rect,
            color,
            width,
            style,
            filled,
        } => {
            let local = ScreenRect {
                left: rect.left - ox,
                top: rect.top - oy,
                right: rect.right - ox,
                bottom: rect.bottom - oy,
            };
            let c = gdi_color(color);
            if *filled {
                let brush = CreateSolidBrush(c);
                let old_b = SelectObject(dc, brush as HGDIOBJ);
                let old_p = SelectObject(dc, GetStockObject(NULL_PEN as i32));
                Ellipse(dc, local.left, local.top, local.right, local.bottom);
                SelectObject(dc, old_p);
                SelectObject(dc, old_b);
                DeleteObject(brush as HGDIOBJ);
            } else {
                draw_ellipse_border_style(dc, local, c, (*width).max(1) as i32, *style);
            }
        }
        Annotation::Triangle {
            rect,
            color,
            width,
            style,
            filled,
        } => {
            let pts = triangle_points(*rect, origin);
            draw_poly_gdi(dc, &pts, gdi_color(color), (*width).max(1) as i32, *style, *filled);
        }
        Annotation::Star {
            rect,
            color,
            width,
            style,
            filled,
        } => {
            let pts = star_points(*rect, origin);
            draw_poly_gdi(dc, &pts, gdi_color(color), (*width).max(1) as i32, *style, *filled);
        }
        Annotation::Bubble {
            rect,
            color,
            width,
            style,
        } => {
            let pts = bubble_points(*rect, origin);
            draw_poly_gdi(dc, &pts, gdi_color(color), (*width).max(1) as i32, *style, false);
        }
        Annotation::Burst {
            rect,
            color,
            width,
            style,
        } => {
            let pts = burst_points(*rect, origin);
            draw_poly_gdi(dc, &pts, gdi_color(color), (*width).max(1) as i32, *style, false);
        }
        Annotation::Arrow {
            start,
            end,
            color,
            width,
            style,
            variant,
        } => {
            let a = ScreenPoint {
                x: start.x - ox,
                y: start.y - oy,
            };
            let b = ScreenPoint {
                x: end.x - ox,
                y: end.y - oy,
            };
            draw_arrow_variant_gdi(
                dc,
                a,
                b,
                gdi_color(color),
                (*width).max(1) as i32,
                *style,
                *variant,
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
        Annotation::Dimension {
            start,
            end,
            color,
            width,
        } => {
            let a = ScreenPoint {
                x: start.x - ox,
                y: start.y - oy,
            };
            let b = ScreenPoint {
                x: end.x - ox,
                y: end.y - oy,
            };
            draw_dimension_gdi(dc, a, b, gdi_color(color), (*width).max(1) as i32);
        }
        Annotation::Angle {
            start,
            end,
            color,
            width,
        } => {
            let a = ScreenPoint {
                x: start.x - ox,
                y: start.y - oy,
            };
            let b = ScreenPoint {
                x: end.x - ox,
                y: end.y - oy,
            };
            draw_angle_gdi(dc, a, b, gdi_color(color), (*width).max(1) as i32);
        }
        Annotation::Wave {
            start,
            end,
            color,
            width,
        } => {
            let a = ScreenPoint {
                x: start.x - ox,
                y: start.y - oy,
            };
            let b = ScreenPoint {
                x: end.x - ox,
                y: end.y - oy,
            };
            draw_wave_gdi(dc, a, b, gdi_color(color), (*width).max(1) as i32);
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
        Annotation::Spotlight(rect) => {
            let pen = CreatePen(0, 1, 0x0059_C734);
            let old_pen = SelectObject(dc, pen as HGDIOBJ);
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH as i32));
            Ellipse(
                dc,
                rect.left - ox,
                rect.top - oy,
                rect.right - ox,
                rect.bottom - oy,
            );
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            DeleteObject(pen as HGDIOBJ);
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
        Annotation::FadePen {
            points,
            color,
            width,
            ..
        } => {
            let pen = CreatePen(0, (*width).max(2) as i32, gdi_color(color));
            let old_pen = SelectObject(dc, pen as HGDIOBJ);
            for pair in points.windows(2) {
                MoveToEx(dc, pair[0].x - ox, pair[0].y - oy, null_mut());
                LineTo(dc, pair[1].x - ox, pair[1].y - oy);
            }
            SelectObject(dc, old_pen);
            DeleteObject(pen as HGDIOBJ);
        }
        Annotation::Mosaic(points, brush_size) => {
            draw_mosaic_brush_preview(dc, screen, points, *brush_size, ox, oy);
        }
        Annotation::Patina { rect, .. } => {
            draw_patina_preview_gdi(dc, screen, *rect, ox, oy);
        }
        Annotation::Watermark { rect, text, color } => {
            let label = if text.trim().is_empty() {
                "SideWeave"
            } else {
                text.trim()
            };
            draw_watermark_preview_gdi(dc, *rect, label, gdi_color(color), ox, oy);
        }
        Annotation::Text(point, text, size, color) if !text.is_empty() => {
            draw_sized_text_gdi(
                dc,
                point.x - ox,
                point.y - oy,
                text,
                *size,
                gdi_color(color),
            );
        }
        Annotation::Text(_, _, _, _) => {}
    }
}

unsafe fn draw_sized_text_gdi(
    dc: HDC,
    x: i32,
    y: i32,
    text: &str,
    size: u32,
    color: u32,
) {
    let font = CreateFontW(
        -(size.clamp(10, 72) as i32),
        0,
        0,
        0,
        FW_SEMIBOLD as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        (DEFAULT_PITCH | FF_DONTCARE) as u32,
        wide("Microsoft YaHei UI").as_ptr(),
    );
    let old_f = if !font.is_null() {
        SelectObject(dc, font as HGDIOBJ)
    } else {
        SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT))
    };
    SetBkMode(dc, 1);
    SetTextColor(dc, color);
    let text = wide(text);
    let mut rect = RECT {
        left: x,
        top: y,
        right: x + 1600,
        bottom: y + (size as i32 * 3).max(28),
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut rect, 0x0800);
    SelectObject(dc, old_f);
    if !font.is_null() {
        DeleteObject(font as HGDIOBJ);
    }
}

unsafe fn draw_poly_gdi(
    dc: HDC,
    pts: &[ScreenPoint],
    color: u32,
    thickness: i32,
    style: StrokeStyle,
    filled: bool,
) {
    if pts.len() < 3 {
        return;
    }
    let gdi_pts: Vec<POINT> = pts.iter().map(|p| POINT { x: p.x, y: p.y }).collect();
    if filled {
        let brush = CreateSolidBrush(color);
        let old_b = SelectObject(dc, brush as HGDIOBJ);
        let old_p = SelectObject(dc, GetStockObject(NULL_PEN as i32));
        Polygon(dc, gdi_pts.as_ptr(), gdi_pts.len() as i32);
        SelectObject(dc, old_p);
        SelectObject(dc, old_b);
        DeleteObject(brush as HGDIOBJ);
    } else {
        let pen = CreatePen(gdi_pen_style(style), thickness.max(1), color);
        let old_p = SelectObject(dc, pen as HGDIOBJ);
        let old_b = SelectObject(dc, GetStockObject(NULL_BRUSH as i32));
        Polygon(dc, gdi_pts.as_ptr(), gdi_pts.len() as i32);
        SelectObject(dc, old_b);
        SelectObject(dc, old_p);
        DeleteObject(pen as HGDIOBJ);
    }
}

unsafe fn draw_arrow_variant_gdi(
    dc: HDC,
    a: ScreenPoint,
    b: ScreenPoint,
    color: u32,
    thickness: i32,
    style: StrokeStyle,
    variant: ArrowVariant,
) {
    match variant {
        ArrowVariant::Single => {
            draw_line_gdi_style(dc, a, b, color, thickness, style);
            draw_filled_arrowhead_gdi(dc, a, b, color, thickness);
        }
        ArrowVariant::Double => {
            draw_line_gdi_style(dc, a, b, color, thickness, style);
            draw_filled_arrowhead_gdi(dc, a, b, color, thickness);
            draw_filled_arrowhead_gdi(dc, b, a, color, thickness);
        }
        ArrowVariant::Curved => {
            let dx = (b.x - a.x) as f32;
            let dy = (b.y - a.y) as f32;
            let cx = (a.x + b.x) as f32 * 0.5 - dy * 0.28;
            let cy = (a.y + b.y) as f32 * 0.5 + dx * 0.28;
            let pen = CreatePen(gdi_pen_style(style), thickness.max(1), color);
            let old_p = SelectObject(dc, pen as HGDIOBJ);
            MoveToEx(dc, a.x, a.y, null_mut());
            let steps = 32;
            for i in 1..=steps {
                let t = i as f32 / steps as f32;
                let inv = 1.0 - t;
                let x = (inv * inv * a.x as f32 + 2.0 * inv * t * cx + t * t * b.x as f32).round()
                    as i32;
                let y = (inv * inv * a.y as f32 + 2.0 * inv * t * cy + t * t * b.y as f32).round()
                    as i32;
                LineTo(dc, x, y);
            }
            SelectObject(dc, old_p);
            DeleteObject(pen as HGDIOBJ);
            draw_filled_arrowhead_gdi(
                dc,
                ScreenPoint {
                    x: cx.round() as i32,
                    y: cy.round() as i32,
                },
                b,
                color,
                thickness,
            );
        }
    }
}

unsafe fn draw_filled_arrowhead_gdi(
    dc: HDC,
    a: ScreenPoint,
    b: ScreenPoint,
    color: u32,
    thickness: i32,
) {
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let length = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / length;
    let uy = dy / length;
    let head_len = (12.0 + thickness as f32 * 2.2).clamp(12.0, 28.0);
    let half_w = head_len * 0.45;
    let pts = [
        POINT { x: b.x, y: b.y },
        POINT {
            x: b.x - (ux * head_len - uy * half_w).round() as i32,
            y: b.y - (uy * head_len + ux * half_w).round() as i32,
        },
        POINT {
            x: b.x - (ux * head_len + uy * half_w).round() as i32,
            y: b.y - (uy * head_len - ux * half_w).round() as i32,
        },
    ];
    let brush = CreateSolidBrush(color);
    let old_b = SelectObject(dc, brush as HGDIOBJ);
    let old_p = SelectObject(dc, GetStockObject(NULL_PEN as i32));
    Polygon(dc, pts.as_ptr(), 3);
    SelectObject(dc, old_p);
    SelectObject(dc, old_b);
    DeleteObject(brush as HGDIOBJ);
}

unsafe fn draw_dimension_gdi(
    dc: HDC,
    a: ScreenPoint,
    b: ScreenPoint,
    color: u32,
    thickness: i32,
) {
    draw_line_gdi_style(dc, a, b, color, thickness, StrokeStyle::Solid);
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let nx = -dy / len * 10.0;
    let ny = dx / len * 10.0;
    for pt in [a, b] {
        draw_line_gdi_style(
            dc,
            ScreenPoint {
                x: (pt.x as f32 - nx).round() as i32,
                y: (pt.y as f32 - ny).round() as i32,
            },
            ScreenPoint {
                x: (pt.x as f32 + nx).round() as i32,
                y: (pt.y as f32 + ny).round() as i32,
            },
            color,
            thickness,
            StrokeStyle::Solid,
        );
    }
    let label = format!("{}px", len.round() as i32);
    draw_sized_text_gdi(dc, (a.x + b.x) / 2 - 22, (a.y + b.y) / 2 - 22, &label, 14, color);
}

unsafe fn draw_angle_gdi(
    dc: HDC,
    a: ScreenPoint,
    b: ScreenPoint,
    color: u32,
    thickness: i32,
) {
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let len = (dx * dx + dy * dy).sqrt().max(16.0);
    let horiz_end = ScreenPoint {
        x: a.x + (if dx >= 0.0 { len } else { -len }).round() as i32,
        y: a.y,
    };
    draw_line_gdi_style(dc, a, b, color, thickness, StrokeStyle::Solid);
    draw_line_gdi_style(dc, a, horiz_end, color, thickness, StrokeStyle::Solid);
    let deg = dy.abs().atan2(dx.abs()).to_degrees().round() as i32;
    let arc_r = (len * 0.35).clamp(16.0, 48.0);
    let a0 = 0.0f32.atan2(if dx >= 0.0 { 1.0 } else { -1.0 });
    let a1 = dy.atan2(dx);
    let mut diff = a1 - a0;
    while diff > std::f32::consts::PI {
        diff -= std::f32::consts::TAU;
    }
    while diff < -std::f32::consts::PI {
        diff += std::f32::consts::TAU;
    }
    let pen = CreatePen(0, thickness.min(2).max(1), color);
    let old_p = SelectObject(dc, pen as HGDIOBJ);
    let steps = 24;
    for i in 0..=steps {
        let ang = a0 + diff * (i as f32 / steps as f32);
        let px = (a.x as f32 + arc_r * ang.cos()).round() as i32;
        let py = (a.y as f32 + arc_r * ang.sin()).round() as i32;
        if i == 0 {
            MoveToEx(dc, px, py, null_mut());
        } else {
            LineTo(dc, px, py);
        }
    }
    SelectObject(dc, old_p);
    DeleteObject(pen as HGDIOBJ);
    let label = format!("{deg}°");
    draw_sized_text_gdi(
        dc,
        a.x + if dx >= 0.0 { 18 } else { -46 },
        a.y + if dy >= 0.0 { 6 } else { -24 },
        &label,
        14,
        color,
    );
}

unsafe fn draw_wave_gdi(dc: HDC, a: ScreenPoint, b: ScreenPoint, color: u32, thickness: i32) {
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;
    let nx = -uy;
    let ny = ux;
    let amp = (thickness as f32 * 1.6).clamp(4.0, 14.0);
    let wavelength = 20.0f32;
    let steps = (len as usize / 2).clamp(16, 360);
    let pen = CreatePen(0, thickness.max(1), color);
    let old_p = SelectObject(dc, pen as HGDIOBJ);
    MoveToEx(dc, a.x, a.y, null_mut());
    for i in 1..=steps {
        let dist = len * (i as f32 / steps as f32);
        let offset = (dist / wavelength * std::f32::consts::TAU).sin() * amp;
        let px = (a.x as f32 + ux * dist + nx * offset).round() as i32;
        let py = (a.y as f32 + uy * dist + ny * offset).round() as i32;
        LineTo(dc, px, py);
    }
    SelectObject(dc, old_p);
    DeleteObject(pen as HGDIOBJ);
}

unsafe fn draw_patina_preview_gdi(
    dc: HDC,
    screen: &ScreenCapture,
    requested: ScreenRect,
    ox: i32,
    oy: i32,
) {
    // Fast preview: greenish-tinted coarse blocks over region
    let bounds = ScreenRect {
        left: screen.x,
        top: screen.y,
        right: screen.x + screen.width,
        bottom: screen.y + screen.height,
    };
    let Some(rect) = requested.clipped_to(bounds) else {
        return;
    };
    const BLOCK: i32 = 8;
    let mut by = rect.top;
    while by < rect.bottom {
        let mut bx = rect.left;
        while bx < rect.right {
            let end_x = (bx + BLOCK).min(rect.right);
            let end_y = (by + BLOCK).min(rect.bottom);
            let sx = (bx - screen.x) as usize;
            let sy = (by - screen.y) as usize;
            let idx = (sy * screen.width as usize + sx) * 4;
            if idx + 2 < screen.pixels.len() {
                let b = screen.pixels[idx] as u32;
                let g = (screen.pixels[idx + 1] as u32 + 24).min(255);
                let r = (screen.pixels[idx + 2] as u32).saturating_sub(14);
                let color = r | (g << 8) | ((b.saturating_sub(10)) << 16);
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
        0x0059_C734,
        1,
    );
}

unsafe fn draw_watermark_preview_gdi(
    dc: HDC,
    rect: ScreenRect,
    text: &str,
    color: u32,
    ox: i32,
    oy: i32,
) {
    let x0 = rect.left - ox;
    let y0 = rect.top - oy;
    let x1 = rect.right - ox;
    let y1 = rect.bottom - oy;
    let step_x = ((text.chars().count() as i32) * 16 + 72).clamp(110, 260);
    let step_y = 68;
    let mut row = 0;
    let mut y = y0 + 14;
    while y < y1 - 8 {
        let mut x = x0 + if row % 2 == 0 { 12 } else { step_x / 2 };
        while x < x1 - 16 {
            draw_sized_text_gdi(dc, x, y, text, 16, color);
            x += step_x;
        }
        y += step_y;
        row += 1;
    }
}

/// Darken the selected crop outside the union of spotlight ellipses.
unsafe fn draw_spotlight_mask(dc: HDC, capture: &CaptureSession, ox: i32, oy: i32) {
    let Some(selection) = capture.selection else {
        return;
    };
    let mut focus_regions = capture
        .annotations
        .iter()
        .filter_map(|annotation| match annotation {
            Annotation::Spotlight(rect) => rect.clipped_to(selection),
            _ => None,
        })
        .filter(|rect| rect.width() > 2 && rect.height() > 2)
        .collect::<Vec<_>>();
    if capture.tool == Tool::Spotlight {
        if let (Some(start), Some(current)) = (capture.drag, capture.current_pos) {
            let rect = ScreenRect::normalized(start, current);
            if rect.width() > 2 && rect.height() > 2 {
                if let Some(rect) = rect.clipped_to(selection) {
                    focus_regions.push(rect);
                }
            }
        }
    }
    if focus_regions.is_empty() {
        return;
    }

    let selection_region = CreateRectRgn(
        selection.left - ox,
        selection.top - oy,
        selection.right - ox,
        selection.bottom - oy,
    );
    let focus_union = CreateRectRgn(0, 0, 0, 0);
    if selection_region.is_null() || focus_union.is_null() {
        if !selection_region.is_null() {
            DeleteObject(selection_region as HGDIOBJ);
        }
        if !focus_union.is_null() {
            DeleteObject(focus_union as HGDIOBJ);
        }
        return;
    }
    for rect in focus_regions {
        let focus = CreateEllipticRgn(
            rect.left - ox,
            rect.top - oy,
            rect.right - ox,
            rect.bottom - oy,
        );
        if !focus.is_null() {
            CombineRgn(focus_union, focus_union, focus, RGN_OR);
            DeleteObject(focus as HGDIOBJ);
        }
    }
    let dim_region = CreateRectRgn(0, 0, 0, 0);
    if dim_region.is_null() {
        DeleteObject(selection_region as HGDIOBJ);
        DeleteObject(focus_union as HGDIOBJ);
        return;
    }
    let region_kind = CombineRgn(dim_region, selection_region, focus_union, RGN_DIFF);
    DeleteObject(selection_region as HGDIOBJ);
    DeleteObject(focus_union as HGDIOBJ);
    if region_kind == 0 {
        DeleteObject(dim_region as HGDIOBJ);
        return;
    }

    let source_dc = CreateCompatibleDC(dc);
    let source_bitmap = CreateCompatibleBitmap(dc, 1, 1);
    if source_dc.is_null() || source_bitmap.is_null() {
        if !source_dc.is_null() {
            DeleteDC(source_dc);
        }
        if !source_bitmap.is_null() {
            DeleteObject(source_bitmap as HGDIOBJ);
        }
        DeleteObject(dim_region as HGDIOBJ);
        return;
    }
    let old_bitmap = SelectObject(source_dc, source_bitmap as HGDIOBJ);
    let black = CreateSolidBrush(0);
    let pixel = RECT {
        left: 0,
        top: 0,
        right: 1,
        bottom: 1,
    };
    FillRect(source_dc, &pixel, black);
    DeleteObject(black as HGDIOBJ);

    let saved_dc = SaveDC(dc);
    if saved_dc != 0 {
        SelectClipRgn(dc, dim_region);
        AlphaBlend(
            dc,
            selection.left - ox,
            selection.top - oy,
            selection.width() as i32,
            selection.height() as i32,
            source_dc,
            0,
            0,
            1,
            1,
            BLENDFUNCTION {
                BlendOp: 0,
                BlendFlags: 0,
                SourceConstantAlpha: 112,
                AlphaFormat: 0,
            },
        );
        RestoreDC(dc, saved_dc);
    }
    SelectObject(source_dc, old_bitmap);
    DeleteObject(source_bitmap as HGDIOBJ);
    DeleteDC(source_dc);
    DeleteObject(dim_region as HGDIOBJ);
}

/// Draw SnowAir-style in-place selectable OCR bounding boxes over the screenshot.
pub(super) unsafe fn draw_ocr_blocks_overlay(
    dc: HDC,
    selection: ScreenRect,
    blocks: &[OcrRegionBox],
    selected_idx: Option<usize>,
    ox: i32,
    oy: i32,
) {
    if blocks.is_empty() {
        return;
    }
    let src_dc = CreateCompatibleDC(dc);
    let src_bmp = CreateCompatibleBitmap(dc, 1, 1);
    if src_dc.is_null() || src_bmp.is_null() {
        if !src_dc.is_null() {
            DeleteDC(src_dc);
        }
        if !src_bmp.is_null() {
            DeleteObject(src_bmp as HGDIOBJ);
        }
        return;
    }
    let old_bmp = SelectObject(src_dc, src_bmp as HGDIOBJ);
    let px_rc = RECT {
        left: 0,
        top: 0,
        right: 1,
        bottom: 1,
    };

    for (i, block) in blocks.iter().enumerate() {
        let is_sel = selected_idx == Some(i);
        let fill_color = if is_sel { 0x0059_C734 } else { 0x00F6_823B }; // SnowAir green or blue highlight
        let brush = CreateSolidBrush(fill_color);
        FillRect(src_dc, &px_rc, brush);
        DeleteObject(brush as HGDIOBJ);

        let left = selection.left + block.x - ox;
        let top = selection.top + block.y - oy;
        let w = block.w.max(4);
        let h = block.h.max(4);
        AlphaBlend(
            dc,
            left,
            top,
            w,
            h,
            src_dc,
            0,
            0,
            1,
            1,
            BLENDFUNCTION {
                BlendOp: 0,
                BlendFlags: 0,
                SourceConstantAlpha: if is_sel { 110 } else { 68 },
                AlphaFormat: 0,
            },
        );
        draw_rect_border(dc, left, top, w, h, fill_color, 1);
    }

    SelectObject(src_dc, old_bmp);
    DeleteObject(src_bmp as HGDIOBJ);
    DeleteDC(src_dc);
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
    SetTextColor(dc, 0x00ff_ffff);
    let text = wide(&number.to_string());
    let mut rect = RECT {
        left: center.x - 12,
        top: center.y - 12,
        right: center.x + 12,
        bottom: center.y + 12,
    };
    let gui_font = GetStockObject(DEFAULT_GUI_FONT);
    let old_f = SelectObject(dc, gui_font);
    DrawTextW(dc, text.as_ptr(), -1, &mut rect, 1 | 4 | 0x20);
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
    capture: &CaptureSession,
    start: ScreenPoint,
    current: ScreenPoint,
    ox: i32,
    oy: i32,
) {
    unsafe {
        let color = gdi_color(&capture.annotation_color);
        let thickness = capture.stroke_width.max(1) as i32;
        let style = capture.stroke_style;
        let a = ScreenPoint {
            x: start.x - ox,
            y: start.y - oy,
        };
        let b = ScreenPoint {
            x: current.x - ox,
            y: current.y - oy,
        };
        let rect = ScreenRect::normalized(start, current);
        let local_rect = ScreenRect::normalized(a, b);
        let origin = ScreenRect {
            left: ox,
            top: oy,
            right: ox,
            bottom: oy,
        };
        match capture.tool {
            Tool::Rectangle => {
                let ann = Annotation::Rect {
                    rect,
                    color: capture.annotation_color,
                    width: capture.stroke_width,
                    style,
                    filled: capture.shape_filled,
                    rounded: capture.shape_rounded,
                };
                draw_single_annotation_gdi(dc, &ann, screen, ox, oy);
            }
            Tool::Ellipse => {
                let ann = Annotation::Ellipse {
                    rect,
                    color: capture.annotation_color,
                    width: capture.stroke_width,
                    style,
                    filled: capture.shape_filled,
                };
                draw_single_annotation_gdi(dc, &ann, screen, ox, oy);
            }
            Tool::Triangle => {
                let pts = triangle_points(rect, origin);
                draw_poly_gdi(dc, &pts, color, thickness, style, capture.shape_filled);
            }
            Tool::Star => {
                let pts = star_points(rect, origin);
                draw_poly_gdi(dc, &pts, color, thickness, style, capture.shape_filled);
            }
            Tool::Bubble => {
                let pts = bubble_points(rect, origin);
                draw_poly_gdi(dc, &pts, color, thickness, style, false);
            }
            Tool::Burst => {
                let pts = burst_points(rect, origin);
                draw_poly_gdi(dc, &pts, color, thickness, style, false);
            }
            Tool::Arrow => draw_arrow_variant_gdi(dc, a, b, color, thickness, style, ArrowVariant::Single),
            Tool::DoubleArrow => draw_arrow_variant_gdi(dc, a, b, color, thickness, style, ArrowVariant::Double),
            Tool::CurvedArrow => draw_arrow_variant_gdi(dc, a, b, color, thickness, style, ArrowVariant::Curved),
            Tool::Line => draw_line_gdi_style(dc, a, b, color, thickness, style),
            Tool::Dimension => draw_dimension_gdi(dc, a, b, color, thickness),
            Tool::Angle => draw_angle_gdi(dc, a, b, color, thickness),
            Tool::Wave => draw_wave_gdi(dc, a, b, color, thickness),
            Tool::Highlight => draw_line_gdi_style(
                dc,
                a,
                b,
                color,
                thickness.saturating_mul(5).max(10),
                StrokeStyle::Solid,
            ),
            Tool::Spotlight => {
                let pen = CreatePen(0, 1, 0x0059_C734);
                let old_pen = SelectObject(dc, pen as HGDIOBJ);
                let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH as i32));
                Ellipse(dc, local_rect.left, local_rect.top, local_rect.right, local_rect.bottom);
                SelectObject(dc, old_brush);
                SelectObject(dc, old_pen);
                DeleteObject(pen as HGDIOBJ);
            }
            Tool::Mosaic => {
                draw_mosaic_brush_preview(dc, screen, &[current], capture.brush_size, ox, oy)
            }
            Tool::Patina => draw_patina_preview_gdi(dc, screen, rect, ox, oy),
            Tool::Watermark => {
                let label = if capture.watermark_text.trim().is_empty() {
                    "SideWeave"
                } else {
                    capture.watermark_text.trim()
                };
                draw_watermark_preview_gdi(dc, rect, label, color, ox, oy);
            }
            _ => {}
        }
    }
}

/// SnowAir crisp square selection handles (7x7px, white fill + green/selection border).
pub(super) unsafe fn draw_selection_handles_gdi(
    dc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    theme: &ThemePalette,
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
    let sel_color = ThemePalette::colorref(theme.selection);
    let pen = CreatePen(0, 1, sel_color);
    let white = CreateSolidBrush(0x00FF_FFFF);
    let old_p = SelectObject(dc, pen as HGDIOBJ);
    let old_b = SelectObject(dc, white as HGDIOBJ);
    for (cx, cy) in points {
        Rectangle(dc, cx - 3, cy - 3, cx + 4, cy + 4);
    }
    SelectObject(dc, old_b);
    SelectObject(dc, old_p);
    DeleteObject(pen as HGDIOBJ);
    DeleteObject(white as HGDIOBJ);
}

/// SnowAir Top-Left Floating Selection Info Capsule (GDI+ anti-aliased & DPI-scaled):
/// `X 1360, Y 384 | 🔒 | 523 × 200 px | ◯─── 0 | ❐`
pub(super) unsafe fn draw_info_capsule_gdi(
    dc: HDC,
    capsule: InfoCapsuleLayout,
    selection: ScreenRect,
    aspect_locked: bool,
    corner_radius: u32,
    drop_shadow: bool,
    theme: &ThemePalette,
    ox: i32,
    oy: i32,
) {
    use super::toolbar_paint::{
        argb, create_ui_font, draw_round_rect_gdip, draw_soft_shadow_gdip, ensure_gdiplus,
    };
    use windows_sys::Win32::Graphics::GdiPlus::{
        GdipCreateFromHDC, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush,
        GdipDeleteGraphics, GdipDeletePen, GdipDrawLine, GdipFillEllipse, GdipSetPixelOffsetMode,
        GdipSetSmoothingMode, GpBrush, GpGraphics, GpPen, GpSolidFill, PixelOffsetModeHalf,
        SmoothingModeAntiAlias8x8, UnitPixel,
    };

    let s = capsule.scale();
    let px = |dp: f32| (dp * s).round() as i32;
    let l = capsule.left - ox;
    let t = capsule.top - oy;
    let b = t + capsule.height;
    let cy_f = (t + b) as f32 * 0.5;

    let text_c = ThemePalette::colorref(theme.info_text);
    let muted_c = ThemePalette::colorref(theme.info_muted);
    let accent_c = ThemePalette::colorref(theme.accent);

    ensure_gdiplus();
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(dc, &mut g) == 0 && !g.is_null() {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);

        let outer_r = 10.0 * s;
        draw_soft_shadow_gdip(
            g,
            l as f32,
            t as f32,
            capsule.width as f32,
            capsule.height as f32,
            outer_r,
            s,
            theme.is_dark,
        );
        draw_round_rect_gdip(
            g,
            l as f32,
            t as f32,
            capsule.width as f32,
            capsule.height as f32,
            outer_r,
            Some(argb(theme.info_bg, Some(0xF6))),
            Some((argb(theme.icon, Some(0x2A)), (1.0 * s).max(1.0))),
        );

        let accent_argb = argb(theme.accent, Some(0xFF));
        let muted_argb = argb(theme.info_muted, Some(0xFF));
        let div_argb = argb(theme.icon, Some(0x28));

        // Aspect lock icon (interlocking rounded links)
        let lock = capsule.lock_rect();
        let lcx = (lock.0 + lock.2) as f32 * 0.5 - ox as f32;
        let lock_argb = if aspect_locked { accent_argb } else { muted_argb };
        draw_round_rect_gdip(
            g,
            lcx - 8.0 * s,
            cy_f - 4.0 * s,
            9.5 * s,
            8.0 * s,
            3.5 * s,
            None,
            Some((lock_argb, 1.6 * s)),
        );
        draw_round_rect_gdip(
            g,
            lcx - 1.5 * s,
            cy_f - 4.0 * s,
            9.5 * s,
            8.0 * s,
            3.5 * s,
            None,
            Some((lock_argb, 1.6 * s)),
        );

        // Divider before radius controls
        let mut dpen: *mut GpPen = null_mut();
        if GdipCreatePen1(div_argb, (1.0 * s).max(1.0), UnitPixel, &mut dpen) == 0 && !dpen.is_null() {
            let div_x = l as f32 + 258.0 * s;
            GdipDrawLine(g, dpen, div_x, cy_f - 9.0 * s, div_x, cy_f + 9.0 * s);
            GdipDeletePen(dpen);
        }

        // Corner radius icon
        let rad_argb = if corner_radius > 0 { accent_argb } else { muted_argb };
        draw_round_rect_gdip(
            g,
            l as f32 + 266.0 * s,
            cy_f - 6.0 * s,
            12.0 * s,
            12.0 * s,
            4.2 * s,
            None,
            Some((rad_argb, 1.6 * s)),
        );

        // Slider track + active green fill + crisp circular thumb
        let track = capsule.slider_track_rect();
        let tx0 = (track.0 - ox) as f32;
        let tx1 = (track.2 - ox) as f32;
        let track_h = 4.0 * s;
        draw_round_rect_gdip(
            g,
            tx0,
            cy_f - track_h * 0.5,
            (tx1 - tx0).max(4.0),
            track_h,
            track_h * 0.5,
            Some(argb(theme.slider_track, Some(0xFF))),
            None,
        );
        let ratio = (corner_radius.min(60) as f32) / 60.0;
        let knob_x = tx0 + (tx1 - tx0) * ratio;
        if knob_x > tx0 + 2.0 * s {
            draw_round_rect_gdip(
                g,
                tx0,
                cy_f - track_h * 0.5,
                knob_x - tx0,
                track_h,
                track_h * 0.5,
                Some(accent_argb),
                None,
            );
        }
        let thumb_r = 6.2 * s;
        let mut thumb_br: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(argb(theme.slider_thumb, Some(0xFF)), &mut thumb_br) == 0
            && !thumb_br.is_null()
        {
            GdipFillEllipse(
                g,
                thumb_br as *mut GpBrush,
                knob_x - thumb_r,
                cy_f - thumb_r,
                thumb_r * 2.0,
                thumb_r * 2.0,
            );
            GdipDeleteBrush(thumb_br as *mut GpBrush);
        }

        // Shadow toggle icon (two layered cards)
        let sh = capsule.shadow_rect();
        let scx = (sh.0 + sh.2) as f32 * 0.5 - ox as f32;
        let sh_argb = if drop_shadow { accent_argb } else { muted_argb };
        draw_round_rect_gdip(
            g,
            scx - 7.0 * s,
            cy_f - 7.0 * s,
            10.0 * s,
            10.0 * s,
            2.4 * s,
            None,
            Some((sh_argb, 1.6 * s)),
        );
        draw_round_rect_gdip(
            g,
            scx - 2.5 * s,
            cy_f - 2.5 * s,
            10.0 * s,
            10.0 * s,
            2.4 * s,
            if drop_shadow { Some(argb(theme.accent, Some(0x45))) } else { None },
            Some((sh_argb, 1.6 * s)),
        );

        GdipDeleteGraphics(g);
    }

    let font = create_ui_font(13.0, s, true);
    let old_f = if !font.is_null() {
        SelectObject(dc, font as HGDIOBJ)
    } else {
        null_mut()
    };
    SetBkMode(dc, 1);

    // Coordinates: "X 1360, Y 384"
    SetTextColor(dc, text_c);
    let xy_str = wide(&format!("X {}, Y {}", selection.left, selection.top));
    let mut xy_rc = RECT {
        left: l + px(12.0),
        top: t,
        right: l + px(124.0),
        bottom: b,
    };
    DrawTextW(dc, xy_str.as_ptr(), -1, &mut xy_rc, 0x4 | 0x20);

    // Size: "523 × 200 px"
    let wh_str = wide(&format!("{} × {} px", selection.width(), selection.height()));
    let mut wh_rc = RECT {
        left: l + px(154.0),
        top: t,
        right: l + px(256.0),
        bottom: b,
    };
    DrawTextW(dc, wh_str.as_ptr(), -1, &mut wh_rc, 0x4 | 0x20);

    // Radius numeric readout
    let track = capsule.slider_track_rect();
    let tx1 = track.2 - ox;
    SetTextColor(dc, if corner_radius > 0 { accent_c } else { muted_c });
    let rad_str = wide(&corner_radius.to_string());
    let mut rad_rc = RECT {
        left: tx1 + px(5.0),
        top: t,
        right: tx1 + px(30.0),
        bottom: b,
    };
    DrawTextW(dc, rad_str.as_ptr(), -1, &mut rad_rc, 0x1 | 0x4 | 0x20);

    if !old_f.is_null() {
        SelectObject(dc, old_f);
    }
    if !font.is_null() {
        DeleteObject(font as HGDIOBJ);
    }
}

pub(super) unsafe fn draw_banner(dc: HDC, x: i32, y: i32, value: &str, theme: &ThemePalette) {
    use super::toolbar_paint::{
        argb, create_ui_font, draw_round_rect_gdip, draw_soft_shadow_gdip, ensure_gdiplus,
    };
    use windows_sys::Win32::Graphics::GdiPlus::{
        GdipCreateFromHDC, GdipDeleteGraphics, GdipSetSmoothingMode, GpGraphics,
        SmoothingModeAntiAlias8x8,
    };

    let s = crate::theme::system_dpi_scale();
    let banner_width = ((value.chars().count() as f32 * 13.5 + 32.0) * s)
        .round()
        .clamp(200.0 * s, 480.0 * s) as i32;
    let banner_height = (36.0 * s).round() as i32;

    ensure_gdiplus();
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(dc, &mut g) == 0 && !g.is_null() {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
        draw_soft_shadow_gdip(
            g,
            x as f32,
            y as f32,
            banner_width as f32,
            banner_height as f32,
            10.0 * s,
            s,
            theme.is_dark,
        );
        draw_round_rect_gdip(
            g,
            x as f32,
            y as f32,
            banner_width as f32,
            banner_height as f32,
            10.0 * s,
            Some(argb(theme.info_bg, Some(0xF6))),
            Some((argb(theme.icon, Some(0x2A)), (1.0 * s).max(1.0))),
        );
        GdipDeleteGraphics(g);
    }

    let font = create_ui_font(13.0, s, true);
    let old_f = if !font.is_null() {
        SelectObject(dc, font as HGDIOBJ)
    } else {
        null_mut()
    };
    SetBkMode(dc, 1);
    SetTextColor(dc, ThemePalette::colorref(theme.info_text));
    let text = wide(value);
    let mut text_rect = RECT {
        left: x + 10,
        top: y,
        right: x + banner_width - 10,
        bottom: y + banner_height,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut text_rect, 0x1 | 0x4 | 0x20);
    if !old_f.is_null() {
        SelectObject(dc, old_f);
    }
    if !font.is_null() {
        DeleteObject(font as HGDIOBJ);
    }
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
    draw_rect_border_style(dc, x, y, width, height, color, thickness, StrokeStyle::Solid);
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
    let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH as i32));
    Rectangle(dc, x, y, x + width, y + height);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(pen as HGDIOBJ);
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
    let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH as i32));
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
