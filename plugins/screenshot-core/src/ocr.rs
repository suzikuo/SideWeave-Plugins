//! Offline Windows WinRT System OCR (with 2x bilinear upscale + per-line bounding boxes + CJK space tidying),
//! inspired by SnowAir's `SystemOcr.cpp`, alongside Tencent Cloud OCR.

use crate::model::OcrRegionBox;

#[derive(Clone, Debug)]
pub struct OcrOutput {
    pub full_text: String,
    pub blocks: Vec<OcrRegionBox>,
    pub engine: String,
}

/// Remove spurious spaces that Windows OCR inserts between adjacent CJK characters,
/// while preserving spaces in Latin/mixed text.
pub fn tidy_cjk_spaces(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' || c == '\u{3000}' {
            let prev_cjk = out.chars().last().is_some_and(|p| (p as u32) >= 0x2E80);
            let next_cjk = chars
                .get(i + 1)
                .is_some_and(|&n| (n as u32) >= 0x2E80);
            if prev_cjk && next_cjk {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Convert RGBA pixels to top-down BGRA and optionally upscale 2x for small images (< 1000px)
/// to dramatically boost Windows OCR accuracy on UI screenshots.
fn prepare_bgra(width: usize, height: usize, rgba: &[u8]) -> (usize, usize, f32, Vec<u8>) {
    let max_dim = width.max(height);
    let upscale = max_dim > 0 && max_dim < 1000 && max_dim * 2 <= 5000;
    if !upscale {
        let mut bgra = vec![0u8; width * height * 4];
        for i in 0..(width * height) {
            let off = i * 4;
            bgra[off] = rgba[off + 2];
            bgra[off + 1] = rgba[off + 1];
            bgra[off + 2] = rgba[off];
            bgra[off + 3] = 255;
        }
        return (width, height, 1.0, bgra);
    }

    let uw = width * 2;
    let uh = height * 2;
    let mut bgra = vec![0u8; uw * uh * 4];
    for y in 0..uh {
        let sy = (y as f32 + 0.5) * 0.5 - 0.5;
        let y0 = (sy.floor() as isize).clamp(0, height as isize - 1) as usize;
        let y1 = (y0 + 1).min(height - 1);
        let fy = (sy - y0 as f32).clamp(0.0, 1.0);
        for x in 0..uw {
            let sx = (x as f32 + 0.5) * 0.5 - 0.5;
            let x0 = (sx.floor() as isize).clamp(0, width as isize - 1) as usize;
            let x1 = (x0 + 1).min(width - 1);
            let fx = (sx - x0 as f32).clamp(0.0, 1.0);

            let i00 = (y0 * width + x0) * 4;
            let i10 = (y0 * width + x1) * 4;
            let i01 = (y1 * width + x0) * 4;
            let i11 = (y1 * width + x1) * 4;
            let dst = (y * uw + x) * 4;

            for (dst_c, src_c) in [(0usize, 2usize), (1, 1), (2, 0)] {
                let c00 = f32::from(rgba[i00 + src_c]);
                let c10 = f32::from(rgba[i10 + src_c]);
                let c01 = f32::from(rgba[i01 + src_c]);
                let c11 = f32::from(rgba[i11 + src_c]);
                let top = c00 + (c10 - c00) * fx;
                let bot = c01 + (c11 - c01) * fx;
                bgra[dst + dst_c] = (top + (bot - top) * fy).round().clamp(0.0, 255.0) as u8;
            }
            bgra[dst + 3] = 255;
        }
    }
    (uw, uh, 0.5, bgra)
}

#[cfg(windows)]
pub fn recognize_system_rgba(width: usize, height: usize, rgba: &[u8]) -> Result<OcrOutput, String> {
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return Err("截图数据无效，无法进行 OCR 识别".into());
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let (uw, uh, inv, bgra) = prepare_bgra(width, height, rgba);
    let writer = DataWriter::new().map_err(|e| format!("初始化 OCR 缓冲区失败：{e}"))?;
    writer
        .WriteBytes(&bgra)
        .map_err(|e| format!("写入 OCR 像素缓冲失败：{e}"))?;
    let buffer = writer
        .DetachBuffer()
        .map_err(|e| format!("获取 OCR 缓冲对象失败：{e}"))?;

    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        uw as i32,
        uh as i32,
        BitmapAlphaMode::Ignore,
    )
    .map_err(|e| format!("创建 Windows OCR 位图失败：{e}"))?;

    let engine = try_create_engine()
        .ok_or_else(|| "系统未安装可用的 Windows OCR 语言包（可在 Windows 设置中添加中文或英文语言包）".to_string())?;

    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| format!("启动系统 OCR 失败：{e}"))?
        .get()
        .map_err(|e| format!("系统 OCR 识别失败：{e}"))?;

    let lines = result
        .Lines()
        .map_err(|e| format!("读取系统 OCR 文本行失败：{e}"))?;

    let mut blocks = Vec::new();
    let mut full_lines = Vec::new();
    for line in lines {
        let raw_text = line.Text().map(|s| s.to_string()).unwrap_or_default();
        let text = tidy_cjk_spaces(raw_text.trim());
        if text.is_empty() {
            continue;
        }
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_r = f32::MIN;
        let mut max_b = f32::MIN;
        let mut any = false;
        if let Ok(words) = line.Words() {
            for word in words {
                if let Ok(br) = word.BoundingRect() {
                    min_x = min_x.min(br.X);
                    min_y = min_y.min(br.Y);
                    max_r = max_r.max(br.X + br.Width);
                    max_b = max_b.max(br.Y + br.Height);
                    any = true;
                }
            }
        }
        let (x, y, w, h) = if any {
            let bw = (max_r - min_x).max(2.0);
            let bh = (max_b - min_y).max(2.0);
            let pad = bh * 0.14;
            let rx = ((min_x - pad).max(0.0) * inv).floor() as i32;
            let ry = ((min_y - pad).max(0.0) * inv).floor() as i32;
            let rw = (((bw + pad * 2.0) * inv).ceil() as i32)
                .max(4)
                .min((width as i32 - rx).max(4));
            let rh = (((bh + pad * 2.0) * inv).ceil() as i32)
                .max(4)
                .min((height as i32 - ry).max(4));
            (rx, ry, rw, rh)
        } else {
            (0, (blocks.len() as i32) * 24, width as i32, 22)
        };
        blocks.push(OcrRegionBox {
            x,
            y,
            w,
            h,
            text: text.clone(),
            translated: None,
        });
        full_lines.push(text);
    }

    if full_lines.is_empty() {
        return Err("未识别到文字".into());
    }

    Ok(OcrOutput {
        full_text: full_lines.join("\n"),
        blocks,
        engine: "Windows 本地离线 OCR".into(),
    })
}

#[cfg(windows)]
fn try_create_engine() -> Option<windows::Media::Ocr::OcrEngine> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Media::Ocr::OcrEngine;

    for tag in ["zh-Hans-CN", "zh-CN", "en-US", "en"] {
        if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(tag)) {
            if OcrEngine::IsLanguageSupported(&lang).unwrap_or(false) {
                if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                    return Some(engine);
                }
            }
        }
    }
    OcrEngine::TryCreateFromUserProfileLanguages().ok()
}

#[cfg(not(windows))]
pub fn recognize_system_rgba(
    _width: usize,
    _height: usize,
    _rgba: &[u8],
) -> Result<OcrOutput, String> {
    Err("Windows 本地离线 OCR 仅支持 Windows".into())
}

#[cfg(test)]
mod tests {
    use super::tidy_cjk_spaces;

    #[test]
    fn tidies_cjk_spaces_while_keeping_english_spaces() {
        assert_eq!(tidy_cjk_spaces("轻 雪 截 图 Snow Air"), "轻雪截图 Snow Air");
    }
}
