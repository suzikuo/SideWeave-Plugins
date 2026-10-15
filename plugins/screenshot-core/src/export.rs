use crate::model::{Annotation, ScreenPoint, ScreenRect, Settings, StrokeStyle};
use arboard::{Clipboard, ImageData};
use image::{ImageBuffer, Rgba};
use std::borrow::Cow;
use std::path::Path;

pub(super) fn compose(
    settings: &Settings,
    width: usize,
    height: usize,
    mut pixels: Vec<u8>,
    annotations: impl IntoIterator<Item = Annotation>,
    selection: ScreenRect,
) -> Vec<u8> {
    for annotation in annotations {
        draw_annotation(&mut pixels, width, height, annotation, selection);
    }
    if !settings.watermark_text.trim().is_empty() {
        draw_watermark(
            &mut pixels,
            width,
            height,
            settings.watermark_text.trim(),
            settings.annotation_rgba(),
        );
    }
    pixels
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
pub(super) fn draw_annotation(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    annotation: Annotation,
    selection: ScreenRect,
) {
    match annotation {
        Annotation::Rect(rect, color, thickness, style) => {
            let a = ScreenPoint {
                x: rect.left - selection.left,
                y: rect.top - selection.top,
            };
            let b = ScreenPoint {
                x: rect.right - selection.left,
                y: rect.bottom - selection.top,
            };
            draw_line_styled(
                pixels,
                width,
                height,
                a,
                ScreenPoint { x: b.x, y: a.y },
                color,
                thickness,
                style,
            );
            draw_line_styled(
                pixels,
                width,
                height,
                ScreenPoint { x: b.x, y: a.y },
                b,
                color,
                thickness,
                style,
            );
            draw_line_styled(
                pixels,
                width,
                height,
                b,
                ScreenPoint { x: a.x, y: b.y },
                color,
                thickness,
                style,
            );
            draw_line_styled(
                pixels,
                width,
                height,
                ScreenPoint { x: a.x, y: b.y },
                a,
                color,
                thickness,
                style,
            );
        }
        Annotation::Ellipse(rect, color, thickness, style) => {
            draw_ellipse(
                pixels, width, height, rect, selection, color, thickness, style,
            );
        }
        Annotation::Arrow(a, b, color, thickness, style) => {
            draw_arrow(
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
        Annotation::Pen(points, color, thickness, style) => {
            draw_path(
                pixels, width, height, &points, selection, color, thickness, style,
            );
        }
        Annotation::Mosaic(points, brush_size) => {
            draw_mosaic_path(pixels, width, height, &points, selection, brush_size);
        }
        Annotation::Text(point, text, size, color) => {
            draw_text(
                pixels,
                width,
                height,
                ScreenPoint {
                    x: point.x - selection.left,
                    y: point.y - selection.top,
                },
                &text,
                size,
                color,
            );
        }
        Annotation::Number(point, number, color) => {
            let center = ScreenPoint {
                x: point.x - selection.left,
                y: point.y - selection.top,
            };
            draw_filled_circle(pixels, width, height, center, 14, color);
            let text = number.to_string();
            let text_width = text.chars().count() as i32 * 6;
            draw_text(
                pixels,
                width,
                height,
                ScreenPoint {
                    x: center.x - text_width / 2,
                    y: center.y - 7,
                },
                &text,
                16,
                [255, 255, 255, 255],
            );
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
    draw_line_styled(pixels, width, height, b, left, color, thickness, style);
    draw_line_styled(pixels, width, height, b, right, color, thickness, style);
}

fn style_allows(style: StrokeStyle, step: usize) -> bool {
    match style {
        StrokeStyle::Solid => true,
        StrokeStyle::Dashed => (step / 8).is_multiple_of(2),
        StrokeStyle::Dotted => step.is_multiple_of(6),
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
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            set_pixel(pixels, width, height, x + dx, y + dy, color);
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
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 || px as usize >= width || py as usize >= height {
                continue;
            }
            let index = (py as usize * width + px as usize) * 4;
            let alpha = color[3] as u16;
            for channel in 0..3 {
                let source = color[channel] as u16;
                let target = pixels[index + channel] as u16;
                pixels[index + channel] = ((source * alpha + target * (255 - alpha)) / 255) as u8;
            }
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

fn draw_text(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    origin: ScreenPoint,
    text: &str,
    size: u32,
    color: [u8; 4],
) {
    let scale = (size / 8).clamp(1, 8) as i32;
    let advance = 6 * scale;
    for (index, character) in text.chars().enumerate() {
        let glyph = glyph_rows(character);
        let x = origin.x + index as i32 * advance;
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) == 0 {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        set_pixel(
                            pixels,
                            width,
                            height,
                            x + column * scale + sx,
                            origin.y + row as i32 * scale + sy,
                            color,
                        );
                    }
                }
            }
        }
    }
}

fn draw_watermark(pixels: &mut [u8], width: usize, height: usize, text: &str, color: [u8; 4]) {
    let scale = 2;
    let text_width = text.chars().count() as i32 * 6 * scale;
    let x = (width as i32 - text_width - 12).max(4);
    let y = (height as i32 - 10 * scale - 12).max(4);
    draw_text(pixels, width, height, ScreenPoint { x, y }, text, 16, color);
}

fn glyph_rows(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        ' ' => [0; 7],
        _ => [
            0b11111, 0b10001, 0b10101, 0b10001, 0b10101, 0b10001, 0b11111,
        ],
    }
}

pub(super) fn save_png_to_path(
    path: &Path,
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<(), String> {
    let image = ImageBuffer::<Rgba<u8>, _>::from_raw(width as u32, height as u32, pixels.to_vec())
        .ok_or_else(|| "閹搭亜娴橀崓蹇曠鐏忓搫顕弮鐘虫櫏".to_string())?;
    image.save(path).map_err(|error| error.to_string())
}
fn set_pixel(pixels: &mut [u8], width: usize, height: usize, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
        return;
    }
    let index = (y as usize * width + x as usize) * 4;
    pixels[index..index + 4].copy_from_slice(&color);
}

pub(super) fn save_png(
    settings: &Settings,
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Option<String> {
    let root = if settings.save_directory.trim().is_empty() {
        std::env::var_os("USERPROFILE")
            .map(std::path::PathBuf::from)?
            .join("Pictures")
            .join("SideWeave")
            .join("Screenshots")
    } else {
        std::path::PathBuf::from(&settings.save_directory)
    };
    std::fs::create_dir_all(&root).ok()?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default();
    for suffix in 0..100u32 {
        let name = if suffix == 0 {
            format!("AgileTiles_{timestamp}.png")
        } else {
            format!("AgileTiles_{timestamp}_{suffix}.png")
        };
        let path = root.join(name);
        if path.exists() {
            continue;
        }
        let image =
            ImageBuffer::<Rgba<u8>, _>::from_raw(width as u32, height as u32, pixels.to_vec())?;
        if image.save(&path).is_ok() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::mosaic;
    #[test]
    fn mosaic_changes_selected_pixels() {
        let width = 16;
        let height = 16;
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[x as u8, y as u8, 180, 255]);
            }
        }
        let original = pixels.clone();
        mosaic(&mut pixels, width, height, 2, 3, 10, 8);
        assert_ne!(&pixels, &original);
        assert_eq!(&pixels[0..4], &original[0..4]);
    }
}
