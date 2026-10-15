/// Decode QR codes from an RGBA screenshot, retrying with image enhancements when needed.
pub(super) fn decode_rgba(width: usize, height: usize, pixels: &[u8]) -> Result<String, String> {
    let expected_len = width
        .checked_mul(height)
        .and_then(|pixel_count| pixel_count.checked_mul(4))
        .ok_or_else(|| "截图图像尺寸无效，无法识别二维码".to_string())?;
    if width == 0 || height == 0 || pixels.len() != expected_len {
        return Err("截图图像不可用，无法识别二维码".into());
    }

    let grayscale = grayscale_from_rgba(pixels);
    let mut payloads = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Keep the common case fast. Most ordinary QR codes decode from the original image.
    add_payloads(&mut payloads, &mut seen, decode_with_quircs(width, height, &grayscale));
    if payloads.is_empty() {
        if let Some(payload) = decode_with_rxing(width, height, &grayscale) {
            add_payload(&mut payloads, &mut seen, payload);
        }
    }

    // Styled codes often use colored modules, uneven backgrounds, or very low contrast.
    // Retry only after both decoders fail on the original image.
    if payloads.is_empty() {
        let (red, green, blue, chroma) = color_variants(pixels);
        let variants = [
            stretch_contrast(&grayscale),
            otsu_threshold(&grayscale),
            adaptive_threshold(width, height, &grayscale),
            red,
            green,
            blue,
            chroma,
        ];
        for variant in variants {
            add_payloads(
                &mut payloads,
                &mut seen,
                decode_with_quircs(width, height, &variant),
            );
            if let Some(payload) = decode_with_rxing(width, height, &variant) {
                add_payload(&mut payloads, &mut seen, payload);
            }
        }
    }

    match payloads.len() {
        0 => Err("未识别到可读取的二维码。请完整框选二维码，并确保边缘没有被截断。".into()),
        1 => Ok(payloads.pop().unwrap_or_default()),
        _ => Ok(payloads
            .into_iter()
            .enumerate()
            .map(|(index, payload)| format!("二维码 {}：\r\n{}", index + 1, payload))
            .collect::<Vec<_>>()
            .join("\r\n\r\n")),
    }
}

fn grayscale_from_rgba(pixels: &[u8]) -> Vec<u8> {
    pixels
        .chunks_exact(4)
        .map(|rgba| {
            let alpha = u32::from(rgba[3]);
            let composite = |component: u8| {
                (u32::from(component) * alpha + 255 * (255 - alpha) + 127) / 255
            };
            let red = composite(rgba[0]);
            let green = composite(rgba[1]);
            let blue = composite(rgba[2]);
            ((red * 299 + green * 587 + blue * 114) / 1000) as u8
        })
        .collect()
}

fn color_variants(pixels: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let pixel_count = pixels.len() / 4;
    let mut red = Vec::with_capacity(pixel_count);
    let mut green = Vec::with_capacity(pixel_count);
    let mut blue = Vec::with_capacity(pixel_count);
    let mut chroma = Vec::with_capacity(pixel_count);

    for rgba in pixels.chunks_exact(4) {
        let alpha = u32::from(rgba[3]);
        let composite = |component: u8| {
            ((u32::from(component) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
        };
        let r = composite(rgba[0]);
        let g = composite(rgba[1]);
        let b = composite(rgba[2]);
        red.push(r);
        green.push(g);
        blue.push(b);
        chroma.push(r.max(g).max(b) - r.min(g).min(b));
    }

    (red, green, blue, chroma)
}

fn decode_with_quircs(width: usize, height: usize, grayscale: &[u8]) -> Vec<String> {
    let mut decoder = quircs::Quirc::default();
    decoder
        .identify(width, height, grayscale)
        .filter_map(|candidate| candidate.ok())
        .filter_map(|code| code.decode().ok())
        .filter_map(|data| {
            let payload = if data.data_type == Some(quircs::DataType::Kanji) {
                encoding_rs::SHIFT_JIS.decode(&data.payload).0.into_owned()
            } else {
                String::from_utf8_lossy(&data.payload).into_owned()
            };
            (!payload.trim().is_empty()).then_some(payload)
        })
        .collect()
}

fn decode_with_rxing(width: usize, height: usize, grayscale: &[u8]) -> Option<String> {
    use rxing::{
        common::HybridBinarizer, qrcode::QRCodeReader, BinaryBitmap, Luma8LuminanceSource, Reader,
    };

    let decode = |pixels: Vec<u8>| {
        let source = Luma8LuminanceSource::new(
            pixels,
            u32::try_from(width).ok()?,
            u32::try_from(height).ok()?,
        )
        .ok()?;
        let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(source));
        QRCodeReader::default()
            .decode(&mut bitmap)
            .ok()
            .map(|result| result.getText().trim().to_owned())
            .filter(|text| !text.is_empty())
    };

    decode(grayscale.to_vec()).or_else(|| {
        decode(
            grayscale
                .iter()
                .map(|luma| 255_u8.saturating_sub(*luma))
                .collect(),
        )
    })
}

fn add_payloads(
    payloads: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
    candidates: impl IntoIterator<Item = String>,
) {
    for candidate in candidates {
        add_payload(payloads, seen, candidate);
    }
}

fn add_payload(
    payloads: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
    payload: String,
) {
    if !payload.trim().is_empty() && seen.insert(payload.clone()) {
        payloads.push(payload);
    }
}

fn stretch_contrast(grayscale: &[u8]) -> Vec<u8> {
    let mut histogram = [0usize; 256];
    for &value in grayscale {
        histogram[value as usize] += 1;
    }

    let total = grayscale.len();
    let low_target = total / 100;
    let high_target = total - total / 100;
    let mut cumulative = 0usize;
    let mut low = 0usize;
    let mut high = 255usize;
    for (value, count) in histogram.iter().enumerate() {
        cumulative += *count;
        if cumulative > low_target {
            low = value;
            break;
        }
    }
    cumulative = 0;
    for (value, count) in histogram.iter().enumerate() {
        cumulative += *count;
        if cumulative >= high_target {
            high = value;
            break;
        }
    }

    if high <= low {
        return grayscale.to_vec();
    }
    let range = (high - low) as u32;
    grayscale
        .iter()
        .map(|&value| {
            (u32::from(value.saturating_sub(low as u8)) * 255 / range).min(255) as u8
        })
        .collect()
}

fn otsu_threshold(grayscale: &[u8]) -> Vec<u8> {
    let mut histogram = [0usize; 256];
    let mut total_luminance = 0f64;
    for &value in grayscale {
        histogram[value as usize] += 1;
        total_luminance += f64::from(value);
    }

    let mut background_pixels = 0usize;
    let mut background_luminance = 0f64;
    let mut best_variance = -1f64;
    let mut threshold = 127usize;

    for (value, count) in histogram.iter().enumerate() {
        background_pixels += *count;
        if background_pixels == 0 {
            continue;
        }
        let foreground_pixels = grayscale.len() - background_pixels;
        if foreground_pixels == 0 {
            break;
        }
        background_luminance += value as f64 * *count as f64;

        let background_mean = background_luminance / background_pixels as f64;
        let foreground_mean = (total_luminance - background_luminance) / foreground_pixels as f64;
        let difference = background_mean - foreground_mean;
        let variance = background_pixels as f64
            * foreground_pixels as f64
            * difference
            * difference;
        if variance > best_variance {
            best_variance = variance;
            threshold = value;
        }
    }

    grayscale
        .iter()
        .map(|&value| if usize::from(value) > threshold { 255 } else { 0 })
        .collect()
}

fn adaptive_threshold(width: usize, height: usize, grayscale: &[u8]) -> Vec<u8> {
    let stride = width + 1;
    let mut integral = vec![0u64; stride * (height + 1)];
    for y in 0..height {
        let mut row_sum = 0u64;
        for x in 0..width {
            row_sum += u64::from(grayscale[y * width + x]);
            integral[(y + 1) * stride + x + 1] =
                integral[y * stride + x + 1] + row_sum;
        }
    }

    let radius = (width.min(height) / 48).clamp(4, 12);
    let mut output = vec![255u8; grayscale.len()];
    for y in 0..height {
        let top = y.saturating_sub(radius);
        let bottom = (y + radius + 1).min(height);
        for x in 0..width {
            let left = x.saturating_sub(radius);
            let right = (x + radius + 1).min(width);
            let sum = integral[bottom * stride + right]
                + integral[top * stride + left]
                - integral[top * stride + right]
                - integral[bottom * stride + left];
            let area = ((bottom - top) * (right - left)) as u64;
            let local_threshold = ((sum / area).saturating_sub(6)).max(16) as u8;
            if grayscale[y * width + x] < local_threshold {
                output[y * width + x] = 0;
            }
        }
    }
    output
}
