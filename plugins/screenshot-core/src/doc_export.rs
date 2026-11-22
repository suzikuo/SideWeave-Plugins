//! Zero-dependency PDF 1.4 and OOXML PPTX document exporter (ported from SnowAir's PdfWriter & PptxWriter),
//! plus multi-format image saving (PNG / JPG / WebP) for the Collage & Document Merge studio.

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView, ImageFormat};
use serde::Deserialize;
use std::fs::File;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPageInput {
    /// Either a local file path or a `data:image/...;base64,...` data URL.
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportDocumentRequest {
    pub pages: Vec<DocumentPageInput>,
    /// `"pdf" | "pptx" | "png" | "jpg" | "webp"`
    pub format: String,
    #[serde(default = "default_quality")]
    pub quality: u8,
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub save_directory: Option<String>,
}

fn default_quality() -> u8 {
    92
}

struct EncodedJpegPage {
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
}

pub fn decode_page_image(source: &str) -> Result<DynamicImage, String> {
    let trimmed = source.trim();
    if let Some(rest) = trimmed.strip_prefix("data:") {
        let Some((_meta, b64)) = rest.split_once(',') else {
            return Err("无效的 Base64 图片数据".into());
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("Base64 解码失败：{e}"))?;
        return image::load_from_memory(&bytes).map_err(|e| format!("解析图片失败：{e}"));
    }
    let path = Path::new(trimmed);
    if !path.is_file() {
        return Err(format!("图片文件不存在：{trimmed}"));
    }
    image::open(path).map_err(|e| format!("读取图片失败（{trimmed}）：{e}"))
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<EncodedJpegPage, String> {
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();
    let mut buf = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut buf, quality.clamp(10, 100));
    encoder
        .encode(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("JPEG 编码失败：{e}"))?;
    Ok(EncodedJpegPage {
        jpeg: buf,
        width,
        height,
    })
}

pub fn export_document(req: &ExportDocumentRequest) -> Result<(String, usize, usize), String> {
    if req.pages.is_empty() {
        return Err("至少需要一张图片才能导出".into());
    }
    let fmt = req.format.trim().to_ascii_lowercase();
    let ext = match fmt.as_str() {
        "pdf" => "pdf",
        "pptx" | "ppt" => "pptx",
        "png" => "png",
        "jpg" | "jpeg" => "jpg",
        "webp" => "webp",
        other => return Err(format!("不支持的导出格式：{other}")),
    };

    let target_path = if let Some(path) = req.output_path.as_deref().filter(|s| !s.trim().is_empty()) {
        PathBuf::from(path)
    } else {
        let dir = req
            .save_directory
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(PathBuf::from)
                    .map(|p| p.join("Pictures").join("SideWeave").join("Screenshots"))
                    .unwrap_or_else(|| PathBuf::from("."))
            });
        let _ = std::fs::create_dir_all(&dir);
        let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        dir.join(format!("SideWeave_Edit_{stamp}.{ext}"))
    };
    if let Some(parent) = target_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match ext {
        "pdf" => {
            let mut pages = Vec::with_capacity(req.pages.len());
            for page in &req.pages {
                let img = decode_page_image(&page.source)?;
                pages.push(encode_jpeg(&img, req.quality)?);
            }
            let w = pages[0].width as usize;
            let h = pages[0].height as usize;
            write_pdf(&target_path, &pages)?;
            Ok((target_path.to_string_lossy().into_owned(), w, h))
        }
        "pptx" => {
            let mut pages = Vec::with_capacity(req.pages.len());
            for page in &req.pages {
                let img = decode_page_image(&page.source)?;
                pages.push(encode_jpeg(&img, req.quality)?);
            }
            let w = pages[0].width as usize;
            let h = pages[0].height as usize;
            write_pptx(&target_path, &pages)?;
            Ok((target_path.to_string_lossy().into_owned(), w, h))
        }
        "png" | "jpg" | "webp" => {
            // If multiple pages are passed for an image format, stitch vertically cleanly.
            let images: Vec<DynamicImage> = req
                .pages
                .iter()
                .map(|p| decode_page_image(&p.source))
                .collect::<Result<Vec<_>, _>>()?;
            let combined = if images.len() == 1 {
                images.into_iter().next().unwrap()
            } else {
                stitch_vertical(&images)
            };
            let (w, h) = combined.dimensions();
            match ext {
                "png" => combined
                    .save_with_format(&target_path, ImageFormat::Png)
                    .map_err(|e| format!("保存 PNG 失败：{e}"))?,
                "jpg" => {
                    let encoded = encode_jpeg(&combined, req.quality)?;
                    std::fs::write(&target_path, &encoded.jpeg)
                        .map_err(|e| format!("保存 JPG 失败：{e}"))?;
                }
                "webp" => {
                    let mut out = Cursor::new(Vec::new());
                    combined
                        .write_to(&mut out, ImageFormat::WebP)
                        .map_err(|e| format!("保存 WebP 失败：{e}"))?;
                    std::fs::write(&target_path, out.into_inner())
                        .map_err(|e| format!("写入 WebP 失败：{e}"))?;
                }
                _ => unreachable!(),
            }
            Ok((target_path.to_string_lossy().into_owned(), w as usize, h as usize))
        }
        _ => unreachable!(),
    }
}

fn stitch_vertical(images: &[DynamicImage]) -> DynamicImage {
    let max_w = images.iter().map(|i| i.width()).max().unwrap_or(1);
    let total_h: u32 = images.iter().map(|i| i.height()).sum::<u32>().max(1);
    let mut canvas = image::RgbaImage::from_pixel(max_w, total_h, image::Rgba([255, 255, 255, 255]));
    let mut y_cursor = 0u32;
    for img in images {
        let rgba = img.to_rgba8();
        let x_offset = (max_w - rgba.width()) / 2;
        image::imageops::overlay(&mut canvas, &rgba, i64::from(x_offset), i64::from(y_cursor));
        y_cursor += rgba.height();
    }
    DynamicImage::ImageRgba8(canvas)
}

fn fmt_num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn write_pdf(path: &Path, pages: &[EncodedJpegPage]) -> Result<(), String> {
    let mut out: Vec<u8> = Vec::new();
    let n = pages.len();
    let page_obj_base = 3usize;
    let obj_count = 2 + n * 3;
    let mut offsets = vec![0usize; obj_count + 1];

    out.extend_from_slice(b"%PDF-1.4\n");

    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", page_obj_base + i * 3)).collect();

    offsets[1] = out.len();
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    offsets[2] = out.len();
    write!(
        out,
        "2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n",
        kids.join(" "),
        n
    )
    .map_err(|e| e.to_string())?;

    for (i, page) in pages.iter().enumerate() {
        let page_id = page_obj_base + i * 3;
        let content_id = page_id + 1;
        let image_id = page_id + 2;
        let pw = f64::from(page.width) * 0.75;
        let ph = f64::from(page.height) * 0.75;

        offsets[page_id] = out.len();
        write!(
            out,
            "{page_id} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /XObject << /Im0 {image_id} 0 R >> >> /Contents {content_id} 0 R >>\nendobj\n",
            fmt_num(pw),
            fmt_num(ph)
        )
        .map_err(|e| e.to_string())?;

        let content = format!("q\n{} 0 0 {} 0 0 cm\n/Im0 Do\nQ\n", fmt_num(pw), fmt_num(ph));
        offsets[content_id] = out.len();
        write!(
            out,
            "{content_id} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n",
            content.len()
        )
        .map_err(|e| e.to_string())?;

        offsets[image_id] = out.len();
        write!(
            out,
            "{image_id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            page.width,
            page.height,
            page.jpeg.len()
        )
        .map_err(|e| e.to_string())?;
        out.extend_from_slice(&page.jpeg);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    }

    let xref_pos = out.len();
    write!(out, "xref\n0 {}\n0000000000 65535 f \n", obj_count + 1).map_err(|e| e.to_string())?;
    for offset in offsets.iter().skip(1) {
        write!(out, "{offset:010} 00000 n \n").map_err(|e| e.to_string())?;
    }
    write!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
        obj_count + 1
    )
    .map_err(|e| e.to_string())?;

    let mut file = File::create(path).map_err(|e| format!("创建 PDF 失败：{e}"))?;
    file.write_all(&out).map_err(|e| format!("写入 PDF 失败：{e}"))
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xEDB8_8320u32 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn write_pptx(path: &Path, pages: &[EncodedJpegPage]) -> Result<(), String> {
    let slide_w = pages[0].width as i64;
    let slide_h = pages[0].height as i64;
    let emu = |px: i64| px * 9525;
    let n = pages.len();

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let add_text = |files: &mut Vec<(String, Vec<u8>)>, name: &str, text: String| {
        files.push((name.to_string(), text.into_bytes()));
    };

    let mut ct = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#,
    );
    for i in 0..n {
        ct.push_str(&format!(
            r#"<Override PartName="/ppt/slides/slide{}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#,
            i + 1
        ));
    }
    ct.push_str("</Types>");
    add_text(&mut files, "[Content_Types].xml", ct);

    add_text(
        &mut files,
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#.into(),
    );

    let mut pres_rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>"#,
    );
    let mut sld_ids = String::new();
    for i in 0..n {
        pres_rels.push_str(&format!(
            r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{}.xml"/>"#,
            i + 2,
            i + 1
        ));
        sld_ids.push_str(&format!(
            r#"<p:sldId id="{}" r:id="rId{}"/>"#,
            256 + i,
            i + 2
        ));
    }
    pres_rels.push_str("</Relationships>");
    add_text(&mut files, "ppt/_rels/presentation.xml.rels", pres_rels);

    add_text(
        &mut files,
        "ppt/presentation.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>{sld_ids}</p:sldIdLst><p:sldSz cx="{}" cy="{}" type="custom"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#,
            emu(slide_w),
            emu(slide_h)
        ),
    );

    let empty_tree = r#"<p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree>"#;

    add_text(
        &mut files,
        "ppt/slideMasters/slideMaster1.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld>{empty_tree}</p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr/></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr><a:defRPr/></a:lvl1pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"#
        ),
    );
    add_text(
        &mut files,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#.into(),
    );
    add_text(
        &mut files,
        "ppt/slideLayouts/slideLayout1.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank" preserve="1"><p:cSld name="Blank">{empty_tree}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#
        ),
    );
    add_text(
        &mut files,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#.into(),
    );
    add_text(
        &mut files,
        "ppt/theme/theme1.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="34C759"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#.into(),
    );

    let sld_cx = emu(slide_w) as f64;
    let sld_cy = emu(slide_h) as f64;
    for (i, page) in pages.iter().enumerate() {
        let idx = i + 1;
        let pw = emu(page.width as i64) as f64;
        let ph = emu(page.height as i64) as f64;
        let scale = (sld_cx / pw.max(1.0)).min(sld_cy / ph.max(1.0));
        let cx = (pw * scale).round() as i64;
        let cy = (ph * scale).round() as i64;
        let x = ((sld_cx as i64) - cx) / 2;
        let y = ((sld_cy as i64) - cy) / 2;

        add_text(
            &mut files,
            &format!("ppt/slides/slide{idx}.xml"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr><p:pic><p:nvPicPr><p:cNvPr id="2" name="Picture 1"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#
            ),
        );
        add_text(
            &mut files,
            &format!("ppt/slides/_rels/slide{idx}.xml.rels"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image{idx}.jpeg"/></Relationships>"#
            ),
        );
        files.push((format!("ppt/media/image{idx}.jpeg"), page.jpeg.clone()));
    }

    // Write uncompressed ZIP archive (store method = 0)
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in &files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let name_bytes = name.as_bytes();
        let nlen = name_bytes.len() as u16;

        // Local file header
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // store
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        // Central directory header
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&nlen.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }

    let coff = out.len() as u32;
    let csize = central.len() as u32;
    let count = files.len() as u16;
    out.extend_from_slice(&central);

    // End of central directory
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&csize.to_le_bytes());
    out.extend_from_slice(&coff.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    let mut file = File::create(path).map_err(|e| format!("创建 PPTX 失败：{e}"))?;
    file.write_all(&out).map_err(|e| format!("写入 PPTX 失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_and_pptx_write_valid_headers() {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            32,
            24,
            image::Rgba([52, 199, 89, 255]),
        ));
        let page = encode_jpeg(&img, 85).expect("jpeg");
        let dir = std::env::temp_dir().join(format!("sideweave_doc_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let pdf = dir.join("test.pdf");
        let pptx = dir.join("test.pptx");
        write_pdf(&pdf, &[EncodedJpegPage { jpeg: page.jpeg.clone(), width: page.width, height: page.height }]).expect("pdf");
        write_pptx(&pptx, &[page]).expect("pptx");
        let pdf_bytes = std::fs::read(&pdf).unwrap();
        let pptx_bytes = std::fs::read(&pptx).unwrap();
        assert!(pdf_bytes.starts_with(b"%PDF-1.4\n"));
        assert!(pptx_bytes.starts_with(b"PK\x03\x04"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
