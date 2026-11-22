use crate::model::{
    Annotation, ArrowVariant, OcrRegionBox, ScreenPoint, ScreenRect, Settings, StrokeStyle,
};
use arboard::{Clipboard, ImageData};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use std::borrow::Cow;
use std::io::Cursor;
use std::path::Path;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ComposeOptions {
    pub corner_radius: u32,
    pub drop_shadow: bool,
}

pub(super) fn compose(
    settings: &Settings,
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    annotations: impl IntoIterator<Item = Annotation>,
    selection: ScreenRect,
) -> Vec<u8> {
    compose_with_options(
        settings,
        width,
        height,
        pixels,
        annotations,
        selection,
        ComposeOptions {
            corner_radius: settings.default_corner_radius,
            drop_shadow: settings.default_shadow,
        },
    )
    .2
}

pub(super) fn compose_with_options(
    settings: &Settings,
    width: usize,
    height: usize,
    mut pixels: Vec<u8>,
    annotations: impl IntoIterator<Item = Annotation>,
    selection: ScreenRect,
    options: ComposeOptions,
) -> (usize, usize, Vec<u8>) {
    let annotations = annotations.into_iter().collect::<Vec<_>>();
    apply_spotlight_masks(&mut pixels, width, height, &annotations, selection);
    for annotation in annotations {
        if !matches!(&annotation, Annotation::Spotlight(_)) {
            draw_annotation(&mut pixels, width, height, annotation, selection);
        }
    }
    if !settings.watermark_text.trim().is_empty() {
        draw_corner_watermark(
            &mut pixels,
            width,
            height,
            settings.watermark_text.trim(),
            settings.annotation_rgba(),
        );
    }
    if options.corner_radius > 0 {
        apply_corner_radius(&mut pixels, width, height, options.corner_radius);
    }
    if options.drop_shadow {
        return apply_drop_shadow(&pixels, width, height, 18, options.corner_radius);
    }
    (width, height, pixels)
}

pub(super) fn copy_to_clipboard(width: usize, height: usize, pixels: &[u8]) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .set_image(ImageData {
            width,
            height,
            bytes: Cow::Owned(pixels.to_vec()),
        })
        .map_err(|error| error.to_string())
}

/// Render translated OCR blocks directly back onto the image in-place (SnowAir Image 8 style).
pub(super) fn render_translated_image(
    width: usize,
    height: usize,
    base_pixels: &[u8],
    blocks: &[OcrRegionBox],
) -> Vec<u8> {
    let mut pixels = base_pixels.to_vec();
    for block in blocks {
        let Some(translated) = block.translated.as_deref().filter(|s| !s.trim().is_empty()) else {
            continue;
        };
        let x0 = block.x.clamp(0, width as i32);
        let y0 = block.y.clamp(0, height as i32);
        let x1 = (block.x + block.w).clamp(x0, width as i32);
        let y1 = (block.y + block.h).clamp(y0, height as i32);
        let bw = (x1 - x0).max(4);
        let bh = (y1 - y0).max(4);

        let bg = sample_border_color(base_pixels, width, height, x0, y0, x1, y1);
        let luma = (u32::from(bg[0]) * 299 + u32::from(bg[1]) * 587 + u32::from(bg[2]) * 114) / 1000;
        let fg = if luma > 140 {
            [24, 24, 27, 255]
        } else {
            [248, 250, 252, 255]
        };

        fill_rounded_rect_rgba(
            &mut pixels,
            width,
            height,
            x0,
            y0,
            bw,
            bh,
            4,
            [bg[0], bg[1], bg[2], 242],
        );

        // Fit font size cleanly to box height & text length
        let char_count = translated.chars().count().max(1) as f32;
        let max_by_h = ((bh as f32) * 0.72).clamp(11.0, 42.0);
        let max_by_w = ((bw as f32) / (char_count * 0.85)).clamp(11.0, max_by_h);
        let font_size = max_by_w.min(max_by_h).round() as u32;
        draw_unicode_text_box(
            &mut pixels,
            width,
            height,
            ScreenRect {
                left: x0 + 2,
                top: y0 + 1,
                right: x1 - 2,
                bottom: y1 - 1,
            },
            translated,
            font_size.max(11),
            fg,
            true,
        );
    }
    pixels
}

fn sample_border_color(
    pixels: &[u8],
    width: usize,
    height: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) -> [u8; 4] {
    let mut sum = [0u64; 3];
    let mut count = 0u64;
    let mut sample = |x: i32, y: i32| {
        if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
            let idx = (y as usize * width + x as usize) * 4;
            if idx + 2 < pixels.len() {
                sum[0] += u64::from(pixels[idx]);
                sum[1] += u64::from(pixels[idx + 1]);
                sum[2] += u64::from(pixels[idx + 2]);
                count += 1;
            }
        }
    };
    let step_x = ((x1 - x0) / 12).max(1);
    let mut x = x0;
    while x < x1 {
        sample(x, (y0 - 2).max(0));
        sample(x, (y1 + 1).min(height as i32 - 1));
        x += step_x;
    }
    let step_y = ((y1 - y0) / 8).max(1);
    let mut y = y0;
    while y < y1 {
        sample((x0 - 2).max(0), y);
        sample((x1 + 1).min(width as i32 - 1), y);
        y += step_y;
    }
    if count == 0 {
        return [255, 255, 255, 255];
    }
    [
        (sum[0] / count) as u8,
        (sum[1] / count) as u8,
        (sum[2] / count) as u8,
        255,
    ]
}

/// Render a clipboard text snippet into a clean SnowAir-styled floating card image (for Quick Pin).
pub(super) fn render_text_pin_card(text: &str, dark: bool) -> (usize, usize, Vec<u8>) {
    let lines: Vec<&str> = text.lines().take(24).collect();
    let max_chars = lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(10)
        .clamp(12, 48);
    let width = (max_chars * 13 + 48).clamp(240, 560);
    let height = (lines.len().max(1) * 24 + 54).clamp(96, 520);
    let bg = if dark {
        [0x1E, 0x1E, 0x20, 0xFF]
    } else {
        [0xFA, 0xFA, 0xFC, 0xFF]
    };
    let fg = if dark {
        [0xF4, 0xF4, 0xF6, 0xFF]
    } else {
        [0x1F, 0x23, 0x29, 0xFF]
    };
    let mut pixels = vec![0u8; width * height * 4];
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.copy_from_slice(&bg);
    }
    // Top accent bar (SnowAir green)
    for y in 0..4 {
        for x in 0..width {
            let idx = (y * width + x) * 4;
            pixels[idx..idx + 4].copy_from_slice(&[0x34, 0xC7, 0x59, 0xFF]);
        }
    }
    let joined = lines.join("\n");
    draw_unicode_text_box(
        &mut pixels,
        width,
        height,
        ScreenRect {
            left: 18,
            top: 18,
            right: width as i32 - 18,
            bottom: height as i32 - 16,
        },
        &joined,
        16,
        fg,
        true,
    );
    (width, height, pixels)
}

pub(super) fn draw_annotation(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    annotation: Annotation,
    selection: ScreenRect,
) {
    match annotation {
        Annotation::Rect {
            rect,
            color,
            width: thickness,
            style,
            filled,
            rounded,
        } => {
            let l = rect.left - selection.left;
            let t = rect.top - selection.top;
            let r = rect.right - selection.left;
            let b = rect.bottom - selection.top;
            let rw = (r - l).max(1);
            let rh = (b - t).max(1);
            let rad = if rounded { (rw.min(rh) / 6).clamp(6, 22) } else { 0 };
            if filled {
                fill_rounded_rect_rgba(pixels, width, height, l, t, rw, rh, rad, color);
            } else if rounded {
                stroke_rounded_rect_rgba(pixels, width, height, l, t, rw, rh, rad, color, thickness, style);
            } else {
                let a = ScreenPoint { x: l, y: t };
                let c = ScreenPoint { x: r, y: b };
                draw_line_styled(pixels, width, height, a, ScreenPoint { x: c.x, y: a.y }, color, thickness, style);
                draw_line_styled(pixels, width, height, ScreenPoint { x: c.x, y: a.y }, c, color, thickness, style);
                draw_line_styled(pixels, width, height, c, ScreenPoint { x: a.x, y: c.y }, color, thickness, style);
                draw_line_styled(pixels, width, height, ScreenPoint { x: a.x, y: c.y }, a, color, thickness, style);
            }
        }
        Annotation::Ellipse {
            rect,
            color,
            width: thickness,
            style,
            filled,
        } => {
            if filled {
                fill_ellipse(pixels, width, height, rect, selection, color);
            } else {
                draw_ellipse(pixels, width, height, rect, selection, color, thickness, style);
            }
        }
        Annotation::Triangle {
            rect,
            color,
            width: thickness,
            style,
            filled,
        } => {
            let pts = triangle_points(rect, selection);
            if filled {
                fill_polygon(pixels, width, height, &pts, color);
            } else {
                draw_polygon_outline(pixels, width, height, &pts, color, thickness, style);
            }
        }
        Annotation::Star {
            rect,
            color,
            width: thickness,
            style,
            filled,
        } => {
            let pts = star_points(rect, selection);
            if filled {
                fill_polygon(pixels, width, height, &pts, color);
            } else {
                draw_polygon_outline(pixels, width, height, &pts, color, thickness, style);
            }
        }
        Annotation::Bubble {
            rect,
            color,
            width: thickness,
            style,
        } => {
            let pts = bubble_points(rect, selection);
            draw_polygon_outline(pixels, width, height, &pts, color, thickness, style);
        }
        Annotation::Burst {
            rect,
            color,
            width: thickness,
            style,
        } => {
            let pts = burst_points(rect, selection);
            draw_polygon_outline(pixels, width, height, &pts, color, thickness, style);
        }
        Annotation::Arrow {
            start,
            end,
            color,
            width: thickness,
            style,
            variant,
        } => {
            let a = ScreenPoint {
                x: start.x - selection.left,
                y: start.y - selection.top,
            };
            let b = ScreenPoint {
                x: end.x - selection.left,
                y: end.y - selection.top,
            };
            match variant {
                ArrowVariant::Single => {
                    draw_arrow(pixels, width, height, a, b, color, thickness, style);
                }
                ArrowVariant::Double => {
                    draw_arrow(pixels, width, height, a, b, color, thickness, style);
                    draw_arrow_head(pixels, width, height, b, a, color, thickness);
                }
                ArrowVariant::Curved => {
                    draw_curved_arrow(pixels, width, height, a, b, color, thickness, style);
                }
            }
        }
        Annotation::Line(a, b, color, thickness, style) => {
            draw_line_styled(
                pixels,
                width,
                height,
                ScreenPoint {
                    x: a.x - selection.left,
                    y: a.y - selection.top,
                },
                ScreenPoint {
                    x: b.x - selection.left,
                    y: b.y - selection.top,
                },
                color,
                thickness,
                style,
            );
        }
        Annotation::Dimension {
            start,
            end,
            color,
            width: thickness,
        } => {
            let a = ScreenPoint {
                x: start.x - selection.left,
                y: start.y - selection.top,
            };
            let b = ScreenPoint {
                x: end.x - selection.left,
                y: end.y - selection.top,
            };
            draw_dimension(pixels, width, height, a, b, color, thickness);
        }
        Annotation::Angle {
            start,
            end,
            color,
            width: thickness,
        } => {
            let a = ScreenPoint {
                x: start.x - selection.left,
                y: start.y - selection.top,
            };
            let b = ScreenPoint {
                x: end.x - selection.left,
                y: end.y - selection.top,
            };
            draw_angle(pixels, width, height, a, b, color, thickness);
        }
        Annotation::Wave {
            start,
            end,
            color,
            width: thickness,
        } => {
            let a = ScreenPoint {
                x: start.x - selection.left,
                y: start.y - selection.top,
            };
            let b = ScreenPoint {
                x: end.x - selection.left,
                y: end.y - selection.top,
            };
            draw_wave(pixels, width, height, a, b, color, thickness);
        }
        Annotation::Highlight(a, b, color, thickness) => {
            draw_line_alpha(
                pixels,
                width,
                height,
                ScreenPoint {
                    x: a.x - selection.left,
                    y: a.y - selection.top,
                },
                ScreenPoint {
                    x: b.x - selection.left,
                    y: b.y - selection.top,
                },
                color,
                thickness,
            );
        }
        Annotation::Spotlight(_) => {}
        Annotation::Pen(points, color, thickness, style) => {
            draw_path(pixels, width, height, &points, selection, color, thickness, style);
        }
        Annotation::FadePen {
            points,
            color,
            width: thickness,
            ..
        } => {
            draw_path(
                pixels,
                width,
                height,
                &points,
                selection,
                color,
                thickness,
                StrokeStyle::Solid,
            );
        }
        Annotation::Mosaic(points, brush_size) => {
            draw_mosaic_path(pixels, width, height, &points, selection, brush_size);
        }
        Annotation::Patina { rect, intensity } => {
            apply_patina_region(pixels, width, height, rect, selection, intensity);
        }
        Annotation::Watermark { rect, text, color } => {
            let label = if text.trim().is_empty() {
                "SideWeave"
            } else {
                text.trim()
            };
            draw_tiled_watermark(pixels, width, height, rect, selection, label, color);
        }
        Annotation::Text(point, text, size, color) => {
            if !text.is_empty() {
                let origin = ScreenPoint {
                    x: point.x - selection.left,
                    y: point.y - selection.top,
                };
                draw_unicode_text_box(
                    pixels,
                    width,
                    height,
                    ScreenRect {
                        left: origin.x,
                        top: origin.y,
                        right: width as i32,
                        bottom: (origin.y + (size as i32) * 3).min(height as i32),
                    },
                    &text,
                    size,
                    color,
                    false,
                );
            }
        }
        Annotation::Number(point, number, color) => {
            let center = ScreenPoint {
                x: point.x - selection.left,
                y: point.y - selection.top,
            };
            draw_filled_circle(pixels, width, height, center, 14, color);
            let text = number.to_string();
            draw_unicode_text_box(
                pixels,
                width,
                height,
                ScreenRect {
                    left: center.x - 13,
                    top: center.y - 11,
                    right: center.x + 13,
                    bottom: center.y + 11,
                },
                &text,
                14,
                [255, 255, 255, 255],
                false,
            );
        }
    }
}

pub(crate) fn triangle_points(rect: ScreenRect, origin: ScreenRect) -> Vec<ScreenPoint> {
    let l = rect.left - origin.left;
    let t = rect.top - origin.top;
    let r = rect.right - origin.left;
    let b = rect.bottom - origin.top;
    vec![
        ScreenPoint { x: (l + r) / 2, y: t },
        ScreenPoint { x: r, y: b },
        ScreenPoint { x: l, y: b },
    ]
}

pub(crate) fn star_points(rect: ScreenRect, origin: ScreenRect) -> Vec<ScreenPoint> {
    let l = (rect.left - origin.left) as f32;
    let t = (rect.top - origin.top) as f32;
    let r = (rect.right - origin.left) as f32;
    let b = (rect.bottom - origin.top) as f32;
    let cx = (l + r) * 0.5;
    let cy = (t + b) * 0.5;
    let rx = ((r - l) * 0.5).max(4.0);
    let ry = ((b - t) * 0.5).max(4.0);
    let mut pts = Vec::with_capacity(10);
    for i in 0..10 {
        let angle = -std::f32::consts::FRAC_PI_2 + (i as f32) * std::f32::consts::PI / 5.0;
        let ratio = if i % 2 == 0 { 1.0 } else { 0.42 };
        pts.push(ScreenPoint {
            x: (cx + rx * ratio * angle.cos()).round() as i32,
            y: (cy + ry * ratio * angle.sin()).round() as i32,
        });
    }
    pts
}

pub(crate) fn bubble_points(rect: ScreenRect, origin: ScreenRect) -> Vec<ScreenPoint> {
    let l = rect.left - origin.left;
    let t = rect.top - origin.top;
    let r = rect.right - origin.left;
    let b = rect.bottom - origin.top;
    let h = (b - t).max(16);
    let w = (r - l).max(20);
    let body_b = b - (h / 5).clamp(8, 22);
    let tail_x1 = l + w / 4;
    let tail_x2 = l + w / 4 + (w / 6).clamp(10, 26);
    let tail_tip = l + w / 5;
    vec![
        ScreenPoint { x: l, y: t },
        ScreenPoint { x: r, y: t },
        ScreenPoint { x: r, y: body_b },
        ScreenPoint { x: tail_x2, y: body_b },
        ScreenPoint { x: tail_tip, y: b },
        ScreenPoint { x: tail_x1, y: body_b },
        ScreenPoint { x: l, y: body_b },
    ]
}

pub(crate) fn burst_points(rect: ScreenRect, origin: ScreenRect) -> Vec<ScreenPoint> {
    let l = (rect.left - origin.left) as f32;
    let t = (rect.top - origin.top) as f32;
    let r = (rect.right - origin.left) as f32;
    let b = (rect.bottom - origin.top) as f32;
    let cx = (l + r) * 0.5;
    let cy = (t + b) * 0.5;
    let rx = ((r - l) * 0.5).max(6.0);
    let ry = ((b - t) * 0.5).max(6.0);
    let spikes = 16;
    let mut pts = Vec::with_capacity(spikes);
    for i in 0..spikes {
        let angle = (i as f32) * std::f32::consts::TAU / spikes as f32;
        let ratio = if i % 2 == 0 { 1.0 } else { 0.68 };
        pts.push(ScreenPoint {
            x: (cx + rx * ratio * angle.cos()).round() as i32,
            y: (cy + ry * ratio * angle.sin()).round() as i32,
        });
    }
    pts
}

fn draw_polygon_outline(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    pts: &[ScreenPoint],
    color: [u8; 4],
    thickness: u32,
    style: StrokeStyle,
) {
    if pts.len() < 2 {
        return;
    }
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        draw_line_styled(pixels, width, height, a, b, color, thickness, style);
    }
}

fn fill_polygon(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    pts: &[ScreenPoint],
    color: [u8; 4],
) {
    if pts.len() < 3 {
        return;
    }
    let min_y = pts.iter().map(|p| p.y).min().unwrap_or(0).max(0);
    let max_y = pts
        .iter()
        .map(|p| p.y)
        .max()
        .unwrap_or(0)
        .min(height as i32 - 1);
    for y in min_y..=max_y {
        let mut nodes = Vec::new();
        let mut j = pts.len() - 1;
        for i in 0..pts.len() {
            let yi = pts[i].y;
            let yj = pts[j].y;
            if (yi < y && yj >= y) || (yj < y && yi >= y) {
                let x = pts[i].x as f32
                    + (y - yi) as f32 / (yj - yi) as f32 * (pts[j].x - pts[i].x) as f32;
                nodes.push(x.round() as i32);
            }
            j = i;
        }
        nodes.sort_unstable();
        for pair in nodes.chunks_exact(2) {
            for x in pair[0].max(0)..=pair[1].min(width as i32 - 1) {
                set_pixel(pixels, width, height, x, y, color);
            }
        }
    }
}

fn draw_curved_arrow(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    a: ScreenPoint,
    b: ScreenPoint,
    color: [u8; 4],
    thickness: u32,
    style: StrokeStyle,
) {
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let cx = (a.x + b.x) as f32 * 0.5 - dy * 0.28;
    let cy = (a.y + b.y) as f32 * 0.5 + dx * 0.28;
    let steps = 48usize;
    let mut prev = a;
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let inv = 1.0 - t;
        let x = (inv * inv * a.x as f32 + 2.0 * inv * t * cx + t * t * b.x as f32).round() as i32;
        let y = (inv * inv * a.y as f32 + 2.0 * inv * t * cy + t * t * b.y as f32).round() as i32;
        let cur = ScreenPoint { x, y };
        if style_allows(style, i) {
            draw_line_styled(pixels, width, height, prev, cur, color, thickness, StrokeStyle::Solid);
        }
        prev = cur;
    }
    let tangent_from = ScreenPoint {
        x: cx.round() as i32,
        y: cy.round() as i32,
    };
    draw_arrow_head(pixels, width, height, tangent_from, b, color, thickness);
}

fn draw_dimension(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    a: ScreenPoint,
    b: ScreenPoint,
    color: [u8; 4],
    thickness: u32,
) {
    draw_line_styled(pixels, width, height, a, b, color, thickness, StrokeStyle::Solid);
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let nx = -dy / len * 10.0;
    let ny = dx / len * 10.0;
    for pt in [a, b] {
        draw_line_styled(
            pixels,
            width,
            height,
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
    let mid_x = (a.x + b.x) / 2;
    let mid_y = (a.y + b.y) / 2 - 22;
    draw_unicode_text_box(
        pixels,
        width,
        height,
        ScreenRect {
            left: mid_x - 36,
            top: mid_y.max(2),
            right: mid_x + 48,
            bottom: mid_y.max(2) + 22,
        },
        &label,
        14,
        color,
        false,
    );
}

fn draw_angle(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    a: ScreenPoint,
    b: ScreenPoint,
    color: [u8; 4],
    thickness: u32,
) {
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let len = (dx * dx + dy * dy).sqrt().max(16.0);
    let horiz_end = ScreenPoint {
        x: a.x + (if dx >= 0.0 { len } else { -len }).round() as i32,
        y: a.y,
    };
    draw_line_styled(pixels, width, height, a, b, color, thickness, StrokeStyle::Solid);
    draw_line_styled(
        pixels,
        width,
        height,
        a,
        horiz_end,
        color,
        thickness,
        StrokeStyle::Solid,
    );
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
    let steps = 28;
    let mut prev = ScreenPoint {
        x: (a.x as f32 + arc_r * a0.cos()).round() as i32,
        y: (a.y as f32 + arc_r * a0.sin()).round() as i32,
    };
    for i in 1..=steps {
        let ang = a0 + diff * (i as f32 / steps as f32);
        let cur = ScreenPoint {
            x: (a.x as f32 + arc_r * ang.cos()).round() as i32,
            y: (a.y as f32 + arc_r * ang.sin()).round() as i32,
        };
        draw_line_styled(pixels, width, height, prev, cur, color, thickness.min(2), StrokeStyle::Solid);
        prev = cur;
    }
    let label = format!("{deg}°");
    draw_unicode_text_box(
        pixels,
        width,
        height,
        ScreenRect {
            left: a.x + (if dx >= 0.0 { 18 } else { -56 }),
            top: a.y + (if dy >= 0.0 { 8 } else { -26 }),
            right: a.x + (if dx >= 0.0 { 84 } else { -4 }),
            bottom: a.y + (if dy >= 0.0 { 32 } else { -2 }),
        },
        &label,
        14,
        color,
        false,
    );
}

fn draw_wave(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    a: ScreenPoint,
    b: ScreenPoint,
    color: [u8; 4],
    thickness: u32,
) {
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;
    let nx = -uy;
    let ny = ux;
    let amp = (thickness as f32 * 1.6).clamp(4.0, 14.0);
    let wavelength = 20.0f32;
    let steps = (len as usize / 2).clamp(16, 400);
    let mut prev = a;
    for i in 1..=steps {
        let dist = len * (i as f32 / steps as f32);
        let offset = (dist / wavelength * std::f32::consts::TAU).sin() * amp;
        let cur = ScreenPoint {
            x: (a.x as f32 + ux * dist + nx * offset).round() as i32,
            y: (a.y as f32 + uy * dist + ny * offset).round() as i32,
        };
        draw_line_styled(pixels, width, height, prev, cur, color, thickness, StrokeStyle::Solid);
        prev = cur;
    }
}

/// Patina (电子包浆) effect inspired by SnowAir's `PatinaEffect.cpp`:
/// YUV green-shift + low-quality JPEG roundtrip compression artifacts!
fn apply_patina_region(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    rect: ScreenRect,
    selection: ScreenRect,
    intensity: u8,
) {
    let x0 = (rect.left - selection.left).clamp(0, width as i32) as usize;
    let y0 = (rect.top - selection.top).clamp(0, height as i32) as usize;
    let x1 = (rect.right - selection.left).clamp(x0 as i32, width as i32) as usize;
    let y1 = (rect.bottom - selection.top).clamp(y0 as i32, height as i32) as usize;
    let rw = x1 - x0;
    let rh = y1 - y0;
    if rw < 4 || rh < 4 {
        return;
    }
    let mut patch = vec![0u8; rw * rh * 3];
    let amount = (f32::from(intensity.clamp(1, 5)) * 0.14).clamp(0.25, 0.72);
    for py in 0..rh {
        for px in 0..rw {
            let src = ((y0 + py) * width + (x0 + px)) * 4;
            let r = i32::from(pixels[src]);
            let g = i32::from(pixels[src + 1]);
            let b = i32::from(pixels[src + 2]);
            // Greenify via YUV bias (-1 on U & V channels, classic WeChat patina)
            let yy = ((77 * r + 150 * g + 29 * b) >> 8).clamp(0, 255);
            let u = (((-43 * r - 85 * g + 128 * b) >> 8) - 2).clamp(-128, 127);
            let v = (((128 * r - 107 * g - 21 * b) >> 8) - 2).clamp(-128, 127);
            let nr = ((65536 * yy + 91881 * v) >> 16).clamp(0, 255);
            let ng = ((65536 * yy - 22553 * u - 46802 * v) >> 16).clamp(0, 255);
            let nb = ((65536 * yy + 116130 * u) >> 16).clamp(0, 255);
            let dst = (py * rw + px) * 3;
            patch[dst] = (r + ((nr - r) as f32 * amount) as i32).clamp(0, 255) as u8;
            patch[dst + 1] = (g + ((ng - g) as f32 * amount) as i32).clamp(0, 255) as u8;
            patch[dst + 2] = (b + ((nb - b) as f32 * amount) as i32).clamp(0, 255) as u8;
        }
    }
    // Low-quality JPEG compression roundtrip (2 passes) for authentic patina blockiness
    let quality = (26u8).saturating_sub(intensity * 4).max(7);
    let mut current_rgb = patch;
    for _ in 0..2 {
        let mut jpeg_buf = Vec::new();
        let mut enc = JpegEncoder::new_with_quality(&mut jpeg_buf, quality);
        if enc
            .encode(&current_rgb, rw as u32, rh as u32, image::ExtendedColorType::Rgb8)
            .is_ok()
        {
            if let Ok(decoded) = image::load_from_memory(&jpeg_buf) {
                let rgb = decoded.to_rgb8();
                if rgb.width() as usize == rw && rgb.height() as usize == rh {
                    current_rgb = rgb.into_raw();
                }
            }
        }
    }
    for py in 0..rh {
        for px in 0..rw {
            let src = (py * rw + px) * 3;
            let dst = ((y0 + py) * width + (x0 + px)) * 4;
            pixels[dst] = current_rgb[src];
            pixels[dst + 1] = current_rgb[src + 1];
            pixels[dst + 2] = current_rgb[src + 2];
        }
    }
}

fn draw_tiled_watermark(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    rect: ScreenRect,
    selection: ScreenRect,
    text: &str,
    color: [u8; 4],
) {
    let x0 = (rect.left - selection.left).clamp(0, width as i32);
    let y0 = (rect.top - selection.top).clamp(0, height as i32);
    let x1 = (rect.right - selection.left).clamp(x0, width as i32);
    let y1 = (rect.bottom - selection.top).clamp(y0, height as i32);
    let wm_color = [color[0], color[1], color[2], 82];
    let step_x = ((text.chars().count() as i32) * 16 + 72).clamp(110, 260);
    let step_y = 68;
    let mut row = 0;
    let mut y = y0 + 14;
    while y < y1 - 8 {
        let mut x = x0 + if row % 2 == 0 { 12 } else { step_x / 2 };
        while x < x1 - 16 {
            draw_unicode_text_box(
                pixels,
                width,
                height,
                ScreenRect {
                    left: x,
                    top: y,
                    right: (x + step_x).min(x1),
                    bottom: (y + 30).min(y1),
                },
                text,
                16,
                wm_color,
                false,
            );
            x += step_x;
        }
        y += step_y;
        row += 1;
    }
}

fn apply_corner_radius(pixels: &mut [u8], width: usize, height: usize, radius: u32) {
    let r = (radius as f32).min(width.min(height) as f32 * 0.5);
    if r <= 0.5 {
        return;
    }
    let wf = width as f32;
    let hf = height as f32;
    for y in 0..height {
        for x in 0..width {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let cx = if fx < r {
                r
            } else if fx > wf - r {
                wf - r
            } else {
                continue;
            };
            let cy = if fy < r {
                r
            } else if fy > hf - r {
                hf - r
            } else {
                continue;
            };
            let dist = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            if dist > r + 0.5 {
                let idx = (y * width + x) * 4;
                pixels[idx + 3] = 0;
            } else if dist > r - 0.5 {
                let cov = (r + 0.5 - dist).clamp(0.0, 1.0);
                let idx = (y * width + x) * 4;
                pixels[idx + 3] = (f32::from(pixels[idx + 3]) * cov).round() as u8;
            }
        }
    }
}

fn apply_drop_shadow(
    pixels: &[u8],
    width: usize,
    height: usize,
    pad: usize,
    radius: u32,
) -> (usize, usize, Vec<u8>) {
    let out_w = width + pad * 2;
    let out_h = height + pad * 2;
    let mut out = vec![0u8; out_w * out_h * 4];
    let r = radius as f32;
    // Soft drop shadow centered slightly lower (+4px)
    let sx0 = pad as f32;
    let sy0 = (pad + 4) as f32;
    let sx1 = (pad + width) as f32;
    let sy1 = (pad + 4 + height) as f32;
    for y in 0..out_h {
        for x in 0..out_w {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let dx = if fx < sx0 + r {
                (sx0 + r - fx).max(0.0)
            } else if fx > sx1 - r {
                (fx - (sx1 - r)).max(0.0)
            } else {
                0.0
            };
            let dy = if fy < sy0 + r {
                (sy0 + r - fy).max(0.0)
            } else if fy > sy1 - r {
                (fy - (sy1 - r)).max(0.0)
            } else {
                0.0
            };
            let dist = (dx * dx + dy * dy).sqrt() - r;
            if dist < pad as f32 {
                let falloff = (1.0 - (dist.max(0.0) / pad as f32)).powi(2);
                let alpha = (falloff * 96.0).round().clamp(0.0, 255.0) as u8;
                let idx = (y * out_w + x) * 4;
                out[idx + 3] = alpha;
            }
        }
    }
    // Alpha-composite image onto shadow canvas
    for y in 0..height {
        for x in 0..width {
            let src = (y * width + x) * 4;
            let dst = ((y + pad) * out_w + (x + pad)) * 4;
            let sa = f32::from(pixels[src + 3]) / 255.0;
            let da = f32::from(out[dst + 3]) / 255.0;
            let out_a = sa + da * (1.0 - sa);
            if out_a > 0.001 {
                for c in 0..3 {
                    let val = (f32::from(pixels[src + c]) * sa
                        + f32::from(out[dst + c]) * da * (1.0 - sa))
                        / out_a;
                    out[dst + c] = val.round().clamp(0.0, 255.0) as u8;
                }
                out[dst + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    (out_w, out_h, out)
}

fn fill_rounded_rect_rgba(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    left: i32,
    top: i32,
    rw: i32,
    rh: i32,
    radius: i32,
    color: [u8; 4],
) {
    let r = radius.min(rw / 2).min(rh / 2).max(0);
    let x0 = left.max(0);
    let y0 = top.max(0);
    let x1 = (left + rw).min(width as i32);
    let y1 = (top + rh).min(height as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let lx = x - left;
            let ly = y - top;
            let inside = if r == 0 {
                true
            } else {
                let cx = if lx < r {
                    r
                } else if lx >= rw - r {
                    rw - r - 1
                } else {
                    lx
                };
                let cy = if ly < r {
                    r
                } else if ly >= rh - r {
                    rh - r - 1
                } else {
                    ly
                };
                let dx = lx - cx;
                let dy = ly - cy;
                dx * dx + dy * dy <= r * r
            };
            if inside {
                blend_pixel(pixels, width, height, x, y, color);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stroke_rounded_rect_rgba(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    left: i32,
    top: i32,
    rw: i32,
    rh: i32,
    radius: i32,
    color: [u8; 4],
    thickness: u32,
    style: StrokeStyle,
) {
    let r = radius.min(rw / 2).min(rh / 2).max(1);
    let right = left + rw;
    let bottom = top + rh;
    // Straight edges
    draw_line_styled(
        pixels,
        width,
        height,
        ScreenPoint { x: left + r, y: top },
        ScreenPoint { x: right - r, y: top },
        color,
        thickness,
        style,
    );
    draw_line_styled(
        pixels,
        width,
        height,
        ScreenPoint { x: left + r, y: bottom },
        ScreenPoint { x: right - r, y: bottom },
        color,
        thickness,
        style,
    );
    draw_line_styled(
        pixels,
        width,
        height,
        ScreenPoint { x: left, y: top + r },
        ScreenPoint { x: left, y: bottom - r },
        color,
        thickness,
        style,
    );
    draw_line_styled(
        pixels,
        width,
        height,
        ScreenPoint { x: right, y: top + r },
        ScreenPoint { x: right, y: bottom - r },
        color,
        thickness,
        style,
    );
    // Four corner arcs
    let corners = [
        (left + r, top + r, std::f32::consts::PI, std::f32::consts::FRAC_PI_2 * 3.0),
        (right - r, top + r, std::f32::consts::FRAC_PI_2 * 3.0, std::f32::consts::TAU),
        (right - r, bottom - r, 0.0, std::f32::consts::FRAC_PI_2),
        (left + r, bottom - r, std::f32::consts::FRAC_PI_2, std::f32::consts::PI),
    ];
    for (cx, cy, a0, a1) in corners {
        let steps = (r * 3).clamp(12, 48);
        for i in 0..=steps {
            if !style_allows(style, i as usize) {
                continue;
            }
            let ang = a0 + (a1 - a0) * (i as f32 / steps as f32);
            let px = (cx as f32 + r as f32 * ang.cos()).round() as i32;
            let py = (cy as f32 + r as f32 * ang.sin()).round() as i32;
            draw_thick_pixel(pixels, width, height, px, py, color, thickness);
        }
    }
}

fn fill_ellipse(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    rect: ScreenRect,
    selection: ScreenRect,
    color: [u8; 4],
) {
    let left = rect.left - selection.left;
    let top = rect.top - selection.top;
    let right = rect.right - selection.left;
    let bottom = rect.bottom - selection.top;
    let cx = (left + right) as f32 * 0.5;
    let cy = (top + bottom) as f32 * 0.5;
    let rx = ((right - left).abs() as f32 * 0.5).max(1.0);
    let ry = ((bottom - top).abs() as f32 * 0.5).max(1.0);
    for y in top.max(0)..=bottom.min(height as i32 - 1) {
        for x in left.max(0)..=right.min(width as i32 - 1) {
            let nx = (x as f32 - cx) / rx;
            let ny = (y as f32 - cy) / ry;
            if nx * nx + ny * ny <= 1.0 {
                blend_pixel(pixels, width, height, x, y, color);
            }
        }
    }
}

fn apply_spotlight_masks(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    annotations: &[Annotation],
    selection: ScreenRect,
) {
    let spots = annotations
        .iter()
        .filter_map(|annotation| match annotation {
            Annotation::Spotlight(rect) => rect.clipped_to(selection),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected_len = width.checked_mul(height).and_then(|len| len.checked_mul(4));
    if spots.is_empty() || expected_len.map_or(true, |len| pixels.len() < len) {
        return;
    }

    for y in 0..height {
        for x in 0..width {
            let screen_x = selection.left as f64 + x as f64 + 0.5;
            let screen_y = selection.top as f64 + y as f64 + 0.5;
            let inside_focus = spots.iter().any(|rect| {
                let radius_x = ((rect.right - rect.left) as f64 / 2.0).max(1.0);
                let radius_y = ((rect.bottom - rect.top) as f64 / 2.0).max(1.0);
                let dx = (screen_x - (rect.left as f64 + rect.right as f64) / 2.0) / radius_x;
                let dy = (screen_y - (rect.top as f64 + rect.bottom as f64) / 2.0) / radius_y;
                dx * dx + dy * dy <= 1.0
            });
            if inside_focus {
                continue;
            }
            let offset = (y * width + x) * 4;
            for channel in &mut pixels[offset..offset + 3] {
                *channel = (*channel as u16 * 143 / 255) as u8;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_path(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    points: &[ScreenPoint],
    selection: ScreenRect,
    color: [u8; 4],
    thickness: u32,
    style: StrokeStyle,
) {
    for pair in points.windows(2) {
        draw_line_styled(
            pixels,
            width,
            height,
            ScreenPoint {
                x: pair[0].x - selection.left,
                y: pair[0].y - selection.top,
            },
            ScreenPoint {
                x: pair[1].x - selection.left,
                y: pair[1].y - selection.top,
            },
            color,
            thickness,
            style,
        );
    }
    if points.len() == 1 {
        let point = ScreenPoint {
            x: points[0].x - selection.left,
            y: points[0].y - selection.top,
        };
        draw_thick_pixel(pixels, width, height, point.x, point.y, color, thickness);
    }
}

fn draw_mosaic_path(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    points: &[ScreenPoint],
    selection: ScreenRect,
    brush_size: u32,
) {
    for pair in points.windows(2) {
        let a = ScreenPoint {
            x: pair[0].x - selection.left,
            y: pair[0].y - selection.top,
        };
        let b = ScreenPoint {
            x: pair[1].x - selection.left,
            y: pair[1].y - selection.top,
        };
        let distance = (b.x - a.x).abs().max((b.y - a.y).abs()).max(1);
        for step in 0..=distance {
            let x = a.x + (b.x - a.x) * step / distance;
            let y = a.y + (b.y - a.y) * step / distance;
            mosaic_brush(pixels, width, height, x, y, brush_size);
        }
    }
    if let Some(point) = points.first() {
        mosaic_brush(
            pixels,
            width,
            height,
            point.x - selection.left,
            point.y - selection.top,
            brush_size,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_ellipse(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    rect: ScreenRect,
    selection: ScreenRect,
    color: [u8; 4],
    thickness: u32,
    style: StrokeStyle,
) {
    let left = rect.left - selection.left;
    let top = rect.top - selection.top;
    let right = rect.right - selection.left;
    let bottom = rect.bottom - selection.top;
    let cx = (left + right) as f32 / 2.0;
    let cy = (top + bottom) as f32 / 2.0;
    let rx = ((right - left).abs() as f32 / 2.0).max(1.0);
    let ry = ((bottom - top).abs() as f32 / 2.0).max(1.0);
    for step in 0..720 {
        if !style_allows(style, step) {
            continue;
        }
        let angle = step as f32 * std::f32::consts::TAU / 720.0;
        draw_thick_pixel(
            pixels,
            width,
            height,
            (cx + rx * angle.cos()).round() as i32,
            (cy + ry * angle.sin()).round() as i32,
            color,
            thickness,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_arrow(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    a: ScreenPoint,
    b: ScreenPoint,
    color: [u8; 4],
    thickness: u32,
    style: StrokeStyle,
) {
    draw_line_styled(pixels, width, height, a, b, color, thickness, style);
    draw_arrow_head(pixels, width, height, a, b, color, thickness);
}

fn draw_arrow_head(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    a: ScreenPoint,
    b: ScreenPoint,
    color: [u8; 4],
    thickness: u32,
) {
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let length = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / length;
    let uy = dy / length;
    let head_len = (12.0 + thickness as f32 * 2.2).clamp(12.0, 28.0);
    let half_w = head_len * 0.45;
    let left = ScreenPoint {
        x: b.x - (ux * head_len - uy * half_w).round() as i32,
        y: b.y - (uy * head_len + ux * half_w).round() as i32,
    };
    let right = ScreenPoint {
        x: b.x - (ux * head_len + uy * half_w).round() as i32,
        y: b.y - (uy * head_len - ux * half_w).round() as i32,
    };
    fill_polygon(pixels, width, height, &[b, left, right], color);
}

fn style_allows(style: StrokeStyle, step: usize) -> bool {
    match style {
        StrokeStyle::Solid => true,
        StrokeStyle::Dashed => (step / 8) % 2 == 0,
        StrokeStyle::Dotted => step % 6 == 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_line_styled(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    a: ScreenPoint,
    b: ScreenPoint,
    color: [u8; 4],
    thickness: u32,
    style: StrokeStyle,
) {
    let mut x = a.x;
    let mut y = a.y;
    let dx = (b.x - a.x).abs();
    let sx = if a.x < b.x { 1 } else { -1 };
    let dy = -(b.y - a.y).abs();
    let sy = if a.y < b.y { 1 } else { -1 };
    let mut error = dx + dy;
    let mut step = 0usize;
    loop {
        if style_allows(style, step) {
            draw_thick_pixel(pixels, width, height, x, y, color, thickness);
        }
        if x == b.x && y == b.y {
            break;
        }
        let twice = error * 2;
        if twice >= dy {
            error += dy;
            x += sx;
        }
        if twice <= dx {
            error += dx;
            y += sy;
        }
        step += 1;
    }
}

fn draw_line_alpha(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    a: ScreenPoint,
    b: ScreenPoint,
    color: [u8; 4],
    thickness: u32,
) {
    let mut x = a.x;
    let mut y = a.y;
    let dx = (b.x - a.x).abs();
    let sx = if a.x < b.x { 1 } else { -1 };
    let dy = -(b.y - a.y).abs();
    let sy = if a.y < b.y { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        draw_blended_pixel(pixels, width, height, x, y, color, thickness);
        if x == b.x && y == b.y {
            break;
        }
        let twice = error * 2;
        if twice >= dy {
            error += dy;
            x += sx;
        }
        if twice <= dx {
            error += dx;
            y += sy;
        }
    }
}

fn draw_thick_pixel(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    color: [u8; 4],
    thickness: u32,
) {
    let radius = ((thickness.max(1) as i32) - 1) / 2;
    let r2 = radius * radius + radius;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if radius <= 1 || dx * dx + dy * dy <= r2 {
                blend_pixel(pixels, width, height, x + dx, y + dy, color);
            }
        }
    }
}

fn draw_blended_pixel(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    color: [u8; 4],
    thickness: u32,
) {
    let radius = ((thickness.max(1) as i32) - 1) / 2;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            blend_pixel(pixels, width, height, x + dx, y + dy, color);
        }
    }
}

fn draw_filled_circle(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    center: ScreenPoint,
    radius: i32,
    color: [u8; 4],
) {
    let radius_squared = radius * radius;
    for y in center.y - radius..=center.y + radius {
        for x in center.x - radius..=center.x + radius {
            let dx = x - center.x;
            let dy = y - center.y;
            if dx * dx + dy * dy <= radius_squared {
                set_pixel(pixels, width, height, x, y, color);
            }
        }
    }
}

fn mosaic_brush(pixels: &mut [u8], width: usize, height: usize, x: i32, y: i32, brush_size: u32) {
    let size = brush_size.max(8) as usize;
    let half = (size / 2) as i32;
    mosaic(pixels, width, height, x - half, y - half, size, size);
}

fn mosaic(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    left: i32,
    top: i32,
    rw: usize,
    rh: usize,
) {
    let block = 8usize;
    let right = (left.max(0) as usize + rw).min(width);
    let bottom = (top.max(0) as usize + rh).min(height);
    let start_x = left.max(0) as usize;
    let start_y = top.max(0) as usize;
    for by in (start_y..bottom).step_by(block) {
        for bx in (start_x..right).step_by(block) {
            let end_x = (bx + block).min(right);
            let end_y = (by + block).min(bottom);
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            for y in by..end_y {
                for x in bx..end_x {
                    let index = (y * width + x) * 4;
                    sum[0] += pixels[index] as u32;
                    sum[1] += pixels[index + 1] as u32;
                    sum[2] += pixels[index + 2] as u32;
                    count += 1;
                }
            }
            if count == 0 {
                continue;
            }
            let color = [
                (sum[0] / count) as u8,
                (sum[1] / count) as u8,
                (sum[2] / count) as u8,
                255,
            ];
            for y in by..end_y {
                for x in bx..end_x {
                    let index = (y * width + x) * 4;
                    pixels[index..index + 4].copy_from_slice(&color);
                }
            }
        }
    }
}

/// Real Windows GDI Unicode text rendering directly onto the RGBA buffer (supports Chinese, emoji, ClearType!).
#[cfg(windows)]
fn draw_unicode_text_box(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    rect: ScreenRect,
    text: &str,
    font_size: u32,
    color: [u8; 4],
    word_break: bool,
) {
    use std::mem::{size_of, zeroed};
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
        SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS,
        FF_DONTCARE, FW_SEMIBOLD, HGDIOBJ, OUT_DEFAULT_PRECIS,
    };

    let x0 = rect.left.clamp(0, width as i32);
    let y0 = rect.top.clamp(0, height as i32);
    let x1 = rect.right.clamp(x0, width as i32);
    let y1 = rect.bottom.clamp(y0, height as i32);
    let bw = (x1 - x0) as i32;
    let bh = (y1 - y0) as i32;
    if bw <= 2 || bh <= 2 || text.is_empty() {
        return;
    }

    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bw,
                biHeight: -bh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..zeroed()
            },
            ..zeroed()
        };
        let mut bits = null_mut();
        let dib = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if dib.is_null() || bits.is_null() {
            if !dib.is_null() {
                DeleteObject(dib as HGDIOBJ);
            }
            DeleteDC(dc);
            return;
        }
        let old_bmp = SelectObject(dc, dib as HGDIOBJ);
        let len = (bw as usize) * (bh as usize) * 4;
        let buf = std::slice::from_raw_parts_mut(bits as *mut u8, len);
        buf.fill(0);

        let face: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
        let font = CreateFontW(
            -(font_size.clamp(10, 72) as i32),
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
            face.as_ptr(),
        );
        let old_font = if !font.is_null() {
            SelectObject(dc, font as HGDIOBJ)
        } else {
            null_mut()
        };

        // Draw pure white text on black background to extract clean alpha mask!
        SetBkMode(dc, 1);
        SetTextColor(dc, 0x00FF_FFFF);
        let wide_text: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mut draw_rc = RECT {
            left: 0,
            top: 0,
            right: bw,
            bottom: bh,
        };
        let flags = if word_break { 0x0010 | 0x2000 } else { 0x0800 };
        DrawTextW(dc, wide_text.as_ptr(), -1, &mut draw_rc, flags);

        let base_alpha = f32::from(color[3]) / 255.0;
        for py in 0..bh as usize {
            for px in 0..bw as usize {
                let sidx = (py * bw as usize + px) * 4;
                let mask = u16::from(buf[sidx])
                    .max(u16::from(buf[sidx + 1]))
                    .max(u16::from(buf[sidx + 2]));
                if mask == 0 {
                    continue;
                }
                let a = ((f32::from(mask) * base_alpha).round()).clamp(0.0, 255.0) as u8;
                blend_pixel(
                    pixels,
                    width,
                    height,
                    x0 + px as i32,
                    y0 + py as i32,
                    [color[0], color[1], color[2], a],
                );
            }
        }

        if !old_font.is_null() {
            SelectObject(dc, old_font);
        }
        if !font.is_null() {
            DeleteObject(font as HGDIOBJ);
        }
        SelectObject(dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(dc);
    }
}

#[cfg(not(windows))]
fn draw_unicode_text_box(
    _pixels: &mut [u8],
    _width: usize,
    _height: usize,
    _rect: ScreenRect,
    _text: &str,
    _font_size: u32,
    _color: [u8; 4],
    _word_break: bool,
) {
}

fn draw_corner_watermark(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    text: &str,
    color: [u8; 4],
) {
    let est_w = (text.chars().count() as i32 * 14 + 24).min(width as i32 - 12);
    let x = (width as i32 - est_w - 14).max(8);
    let y = (height as i32 - 32).max(6);
    draw_unicode_text_box(
        pixels,
        width,
        height,
        ScreenRect {
            left: x,
            top: y,
            right: width as i32 - 8,
            bottom: height as i32 - 4,
        },
        text,
        16,
        [color[0], color[1], color[2], 190],
        false,
    );
}

fn set_pixel(pixels: &mut [u8], width: usize, height: usize, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
        return;
    }
    let index = (y as usize * width + x as usize) * 4;
    pixels[index..index + 4].copy_from_slice(&color);
}

fn blend_pixel(pixels: &mut [u8], width: usize, height: usize, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
        return;
    }
    let index = (y as usize * width + x as usize) * 4;
    let alpha = u16::from(color[3]);
    if alpha >= 255 {
        pixels[index..index + 4].copy_from_slice(&color);
        return;
    }
    if alpha == 0 {
        return;
    }
    let inv = 255 - alpha;
    for c in 0..3 {
        let src = u16::from(color[c]);
        let dst = u16::from(pixels[index + c]);
        pixels[index + c] = ((src * alpha + dst * inv) / 255) as u8;
    }
    pixels[index + 3] = pixels[index + 3].max(color[3]);
}

pub(super) fn save_image_to_path(
    path: &Path,
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width as u32, height as u32, pixels.to_vec())
        .ok_or_else(|| "图像像素尺寸无效".to_string())?;
    match ext.as_str() {
        "jpg" | "jpeg" => {
            let rgb = DynamicImage::ImageRgba8(img).to_rgb8();
            let mut buf = Vec::new();
            let mut enc = JpegEncoder::new_with_quality(&mut buf, 92);
            enc.encode(
                rgb.as_raw(),
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| e.to_string())?;
            std::fs::write(path, buf).map_err(|e| e.to_string())
        }
        "webp" => {
            let mut out = Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(img)
                .write_to(&mut out, ImageFormat::WebP)
                .map_err(|e| e.to_string())?;
            std::fs::write(path, out.into_inner()).map_err(|e| e.to_string())
        }
        _ => img.save(path).map_err(|e| e.to_string()),
    }
}

pub(super) fn save_png_to_path(
    path: &Path,
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<(), String> {
    save_image_to_path(path, pixels, width, height)
}

pub(super) fn expand_filename_template(template: &str) -> String {
    let now = chrono::Local::now();
    let mut name = template
        .replace("{{YYYY}}", &now.format("%Y").to_string())
        .replace("{{MM}}", &now.format("%m").to_string())
        .replace("{{DD}}", &now.format("%d").to_string())
        .replace("{{HH}}", &now.format("%H").to_string())
        .replace("{{mm}}", &now.format("%M").to_string())
        .replace("{{ss}}", &now.format("%S").to_string());
    // Sanitize illegal Windows filename chars
    name = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect();
    if name.trim().is_empty() {
        format!("SideWeave_{}", now.format("%Y%m%d_%H%M%S"))
    } else {
        name
    }
}

pub(super) fn save_png(
    settings: &Settings,
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Option<String> {
    let root = settings.resolved_save_directory();
    std::fs::create_dir_all(&root).ok()?;
    let ext = match settings.save_format.as_str() {
        "jpg" => "jpg",
        "webp" => "webp",
        _ => "png",
    };
    let stem = expand_filename_template(&settings.file_name_template);
    for suffix in 0..100u32 {
        let name = if suffix == 0 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}_{suffix}.{ext}")
        };
        let path = root.join(name);
        if path.exists() {
            continue;
        }
        if save_image_to_path(&path, pixels, width, height).is_ok() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}
