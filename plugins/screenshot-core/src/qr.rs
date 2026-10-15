/// Decode every readable QR code in an RGBA screenshot image.
pub(super) fn decode_rgba(width: usize, height: usize, pixels: &[u8]) -> Result<String, String> {
    let expected_len = width
        .checked_mul(height)
        .and_then(|pixel_count| pixel_count.checked_mul(4))
        .ok_or_else(|| "截图图像尺寸无效，无法识别二维码".to_string())?;
    if width == 0 || height == 0 || pixels.len() != expected_len {
        return Err("截图图像不可用，无法识别二维码".into());
    }

    let grayscale: Vec<u8> = pixels
        .chunks_exact(4)
        .map(|rgba| {
            ((u32::from(rgba[0]) * 299 + u32::from(rgba[1]) * 587 + u32::from(rgba[2]) * 114)
                / 1000) as u8
        })
        .collect();

    let mut decoder = quircs::Quirc::default();
    let mut payloads = Vec::new();
    for candidate in decoder.identify(width, height, &grayscale) {
        let Ok(code) = candidate else {
            continue;
        };
        let Ok(data) = code.decode() else {
            continue;
        };
        let payload = if data.data_type == Some(quircs::DataType::Kanji) {
            encoding_rs::SHIFT_JIS.decode(&data.payload).0.into_owned()
        } else {
            String::from_utf8_lossy(&data.payload).into_owned()
        };
        if !payload.trim().is_empty() {
            payloads.push(payload);
        }
    }

    if payloads.is_empty() {
        if let Some(payload) = decode_with_rxing(width, height, &grayscale) {
            payloads.push(payload);
        }
    }

    match payloads.len() {
        0 => Err("选区中没有识别到可读取的二维码".into()),
        1 => Ok(payloads.pop().unwrap_or_default()),
        _ => Ok(payloads
            .into_iter()
            .enumerate()
            .map(|(index, payload)| format!("二维码{}：\r\n{}", index + 1, payload))
            .collect::<Vec<_>>()
            .join("\r\n\r\n")),
    }
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
