//! Persistent screenshot, scroll, recording, and image-edit history with custom tags.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_HISTORY_ITEMS: usize = 300;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    /// `"screenshot" | "scroll" | "recording" | "edit"`
    pub kind: String,
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub created_at_ms: u64,
    pub file_size: u64,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub fn history_path() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("SideWeave")
        .join("screenshot-history.json")
}

pub fn load_all() -> Vec<HistoryItem> {
    let Ok(raw) = fs::read_to_string(history_path()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<HistoryItem>>(&raw).unwrap_or_default()
}

fn save_all(items: &[HistoryItem]) -> Result<(), String> {
    let path = history_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?;
    fs::write(path, bytes).map_err(|e| e.to_string())
}

pub fn record_item(kind: &str, path: &str, width: usize, height: usize) {
    if path.trim().is_empty() {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut items = load_all();
    // Deduplicate same path
    items.retain(|it| !it.path.eq_ignore_ascii_case(path));
    items.insert(
        0,
        HistoryItem {
            id: format!("hist_{now}_{}", items.len()),
            kind: kind.to_string(),
            path: path.to_string(),
            width,
            height,
            created_at_ms: now,
            file_size,
            tags: Vec::new(),
        },
    );
    if items.len() > MAX_HISTORY_ITEMS {
        items.truncate(MAX_HISTORY_ITEMS);
    }
    let _ = save_all(&items);
}

pub fn list_items(
    save_dir: &Path,
    kind_filter: Option<&str>,
    tag_filter: Option<&str>,
    query: Option<&str>,
    include_dir_scan: bool,
) -> Vec<HistoryItem> {
    let mut items = load_all();
    // Keep only existing files for clean UX
    items.retain(|it| Path::new(&it.path).is_file());

    if include_dir_scan && save_dir.is_dir() {
        let mut known: HashSet<String> = items
            .iter()
            .map(|it| it.path.to_ascii_lowercase())
            .collect();
        if let Ok(entries) = fs::read_dir(save_dir) {
            let mut extra = Vec::new();
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let kind = match ext.as_str() {
                    "png" | "jpg" | "jpeg" | "webp" | "bmp" => "screenshot",
                    "mp4" | "gif" => "recording",
                    "pdf" | "pptx" => "edit",
                    _ => continue,
                };
                let path_str = p.to_string_lossy().into_owned();
                if !known.insert(path_str.to_ascii_lowercase()) {
                    continue;
                }
                let meta = entry.metadata().ok();
                let created_at_ms = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let file_size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                extra.push(HistoryItem {
                    id: format!("ext_{created_at_ms}_{}", extra.len()),
                    kind: kind.to_string(),
                    path: path_str,
                    width: 0,
                    height: 0,
                    created_at_ms,
                    file_size,
                    tags: Vec::new(),
                });
            }
            items.extend(extra);
        }
    }

    items.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));

    let kind_filter = kind_filter
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all");
    let tag_filter = tag_filter.map(str::trim).filter(|s| !s.is_empty());
    let query_lower = query
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    items
        .into_iter()
        .filter(|it| {
            if let Some(k) = kind_filter {
                if it.kind != k {
                    return false;
                }
            }
            if let Some(t) = tag_filter {
                if !it.tags.iter().any(|existing| existing == t) {
                    return false;
                }
            }
            if let Some(q) = &query_lower {
                let in_path = it.path.to_ascii_lowercase().contains(q);
                let in_tag = it.tags.iter().any(|t| t.to_ascii_lowercase().contains(q));
                if !in_path && !in_tag {
                    return false;
                }
            }
            true
        })
        .take(MAX_HISTORY_ITEMS)
        .collect()
}

pub fn add_tag(id: &str, path: Option<&str>, tag: &str) -> Result<Vec<String>, String> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Err("标签不能为空".into());
    }
    let mut items = load_all();
    let idx = items
        .iter()
        .position(|it| it.id == id || path.is_some_and(|p| it.path.eq_ignore_ascii_case(p)));
    let item = if let Some(i) = idx {
        &mut items[i]
    } else if let Some(p) = path {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let file_size = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        items.insert(
            0,
            HistoryItem {
                id: id.to_string(),
                kind: "screenshot".into(),
                path: p.to_string(),
                width: 0,
                height: 0,
                created_at_ms: now,
                file_size,
                tags: Vec::new(),
            },
        );
        &mut items[0]
    } else {
        return Err("未找到对应的历史记录".into());
    };
    if !item.tags.iter().any(|t| t == tag) {
        item.tags.push(tag.to_string());
    }
    let tags = item.tags.clone();
    save_all(&items)?;
    Ok(tags)
}

pub fn remove_tag(id: &str, path: Option<&str>, tag: &str) -> Result<Vec<String>, String> {
    let mut items = load_all();
    let Some(item) = items
        .iter_mut()
        .find(|it| it.id == id || path.is_some_and(|p| it.path.eq_ignore_ascii_case(p)))
    else {
        return Ok(Vec::new());
    };
    item.tags.retain(|t| t != tag);
    let tags = item.tags.clone();
    save_all(&items)?;
    Ok(tags)
}

pub fn delete_item(id: &str, path: Option<&str>, delete_file: bool) -> Result<(), String> {
    let mut items = load_all();
    let mut removed_path = path.map(|s| s.to_string());
    items.retain(|it| {
        let matched =
            it.id == id || path.is_some_and(|p| it.path.eq_ignore_ascii_case(p));
        if matched && removed_path.is_none() {
            removed_path = Some(it.path.clone());
        }
        !matched
    });
    if delete_file {
        if let Some(ref p) = removed_path {
            if Path::new(p).exists() {
                move_to_recycle_bin(p)?;
            }
            remove_cached_thumbnails_for(p);
        }
    }
    save_all(&items)?;
    Ok(())
}

pub fn clear_all() -> Result<(), String> {
    save_all(&[])
}

pub fn thumb_cache_dir() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("SideWeave")
        .join("thumb-cache")
}

pub fn clear_thumb_cache() -> Result<(), String> {
    let dir = thumb_cache_dir();
    if dir.is_dir() {
        let _ = fs::remove_dir_all(&dir);
    }
    Ok(())
}

fn short_hash(input: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(input);
    hex::encode(&hash[..8])
}

fn thumb_cache_key(path: &Path, max_dim: u32) -> Option<(PathBuf, u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let normalized = path.to_string_lossy().to_ascii_lowercase();
    let digest = short_hash(format!("{normalized}|{size}|{mtime}|{max_dim}").as_bytes());
    let path_hash = short_hash(normalized.as_bytes());
    let file_name = format!("{path_hash}_{digest}.jpg");
    Some((thumb_cache_dir().join(file_name), size, mtime))
}

fn remove_cached_thumbnails_for(path: &str) {
    let dir = thumb_cache_dir();
    if !dir.is_dir() {
        return;
    }
    let prefix = format!("{}_", short_hash(path.to_ascii_lowercase().as_bytes()));
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with(&prefix) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Reads a JPEG thumbnail from the persistent disk cache, or builds & caches it on miss.
/// Returns `(jpeg_bytes, orig_width, orig_height)`.
pub fn read_or_build_thumbnail(path: &Path, max_dim: u32) -> Result<(Vec<u8>, u32, u32), String> {
    if !path.is_file() {
        return Err("文件不存在".into());
    }
    let target_dim = max_dim.clamp(64, 480);
    let cache_path = thumb_cache_key(path, target_dim).map(|(p, _, _)| p);

    if let Some(ref cp) = cache_path {
        if let Ok(cached_bytes) = fs::read(cp) {
            if !cached_bytes.is_empty() {
                let (w, h) = image::image_dimensions(path).unwrap_or((0, 0));
                return Ok((cached_bytes, w, h));
            }
        }
    }

    let img = image::open(path).map_err(|e| format!("读取图片失败：{e}"))?;
    let (orig_w, orig_h) = (img.width(), img.height());
    let thumb = if orig_w > target_dim || orig_h > target_dim {
        img.thumbnail(target_dim, target_dim)
    } else {
        img
    };
    let rgb = thumb.to_rgb8();
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| format!("编码缩略图失败：{e}"))?;

    if let Some(ref cp) = cache_path {
        if let Some(parent) = cp.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(cp, &out);
        prune_thumb_cache_if_needed();
    }
    Ok((out, orig_w, orig_h))
}

fn prune_thumb_cache_if_needed() {
    const MAX_CACHED_THUMBS: usize = 500;
    let dir = thumb_cache_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if !p.is_file() {
                return None;
            }
            let mtime = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Some((p, mtime))
        })
        .collect();
    if files.len() <= MAX_CACHED_THUMBS {
        return;
    }
    files.sort_by_key(|(_, mtime)| *mtime);
    let to_remove = files.len() - (MAX_CACHED_THUMBS * 4 / 5);
    for (p, _) in files.into_iter().take(to_remove) {
        let _ = fs::remove_file(p);
    }
}

#[cfg(windows)]
pub fn move_to_recycle_bin(path: &str) -> Result<(), String> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[cfg(target_pointer_width = "64")]
    #[repr(C)]
    struct ShFileOpStructW {
        hwnd: *mut c_void,
        w_func: u32,
        p_from: *const u16,
        p_to: *const u16,
        f_flags: u16,
        f_any_operations_aborted: i32,
        h_name_mappings: *mut c_void,
        lpsz_progress_title: *const u16,
    }

    #[cfg(target_pointer_width = "32")]
    #[repr(C, packed(1))]
    struct ShFileOpStructW {
        hwnd: *mut c_void,
        w_func: u32,
        p_from: *const u16,
        p_to: *const u16,
        f_flags: u16,
        f_any_operations_aborted: i32,
        h_name_mappings: *mut c_void,
        lpsz_progress_title: *const u16,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(lpfileop: *mut ShFileOpStructW) -> i32;
    }

    const FO_DELETE: u32 = 0x0003;
    const FOF_SILENT: u16 = 0x0004;
    const FOF_NOCONFIRMATION: u16 = 0x0010;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_NOERRORUI: u16 = 0x0400;

    // Strip \\?\ verbatim prefix if present, as SHFileOperationW requires standard Win32 paths
    let cleaned = path.strip_prefix(r"\\?\").unwrap_or(path);
    let mut wide_from: Vec<u16> = Path::new(cleaned)
        .as_os_str()
        .encode_wide()
        .collect();
    // Double null terminator required by SHFileOperationW
    wide_from.push(0);
    wide_from.push(0);

    let mut op = ShFileOpStructW {
        hwnd: std::ptr::null_mut(),
        w_func: FO_DELETE,
        p_from: wide_from.as_ptr(),
        p_to: std::ptr::null(),
        f_flags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI,
        f_any_operations_aborted: 0,
        h_name_mappings: std::ptr::null_mut(),
        lpsz_progress_title: std::ptr::null(),
    };
    let ret = unsafe { SHFileOperationW(&mut op) };
    let aborted = op.f_any_operations_aborted;
    if ret != 0 || aborted != 0 {
        return Err(format!("移入回收站失败（错误码 {ret:#x}）"));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn move_to_recycle_bin(path: &str) -> Result<(), String> {
    fs::remove_file(path).map_err(|e| format!("删除文件失败：{e}"))
}
