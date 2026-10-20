use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::path::PathBuf;

pub const DEFAULT_HOTKEY: &str = "ctrl+shift+a";
pub const RECORDING_FPS_OPTIONS: [u32; 8] = [5, 8, 10, 12, 15, 20, 24, 30];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HdrCaptureMode {
    #[default]
    Auto,
    ForceOn,
    ForceOff,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct Settings {
    pub hotkey: String,
    pub save_directory: String,
    pub auto_copy: bool,
    pub scroll_step: i32,
    pub hdr_capture_mode: HdrCaptureMode,
    pub watermark_text: String,
    pub annotation_color: String,
    pub text_size: u32,
    pub ocr_region: String,
    pub ocr_select_model: u8,
    pub recording_mp4_fps: u32,
    pub recording_mp4_max_height: u32,
    pub recording_mp4_bitrate_mbps: u32,
    pub recording_gif_fps: u32,
    pub recording_webp_fps: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.into(),
            save_directory: String::new(),
            auto_copy: true,
            scroll_step: 640,
            hdr_capture_mode: HdrCaptureMode::Auto,
            watermark_text: String::new(),
            annotation_color: "#ff3a48".into(),
            text_size: 18,
            ocr_region: String::new(),
            ocr_select_model: 0,
            recording_mp4_fps: 15,
            recording_mp4_max_height: 1080,
            recording_mp4_bitrate_mbps: 16,
            recording_gif_fps: 15,
            recording_webp_fps: 15,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("SideWeave")
            .join("screenshot.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
            .map(|mut value: Self| {
                value.normalize();
                value
            })
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(
            path,
            serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    }

    pub fn normalize(&mut self) {
        if self.hotkey.trim().is_empty() {
            self.hotkey = DEFAULT_HOTKEY.into();
        }
        self.scroll_step = self.scroll_step.clamp(120, 1600);
        self.text_size = self.text_size.clamp(10, 72);
        if !RECORDING_FPS_OPTIONS.contains(&self.recording_mp4_fps) {
            self.recording_mp4_fps = 15;
        }
        if ![720, 1080].contains(&self.recording_mp4_max_height) {
            self.recording_mp4_max_height = 1080;
        }
        if ![4, 6, 8, 10, 12, 16, 20, 24].contains(&self.recording_mp4_bitrate_mbps) {
            self.recording_mp4_bitrate_mbps = 16;
        }
        if !RECORDING_FPS_OPTIONS.contains(&self.recording_gif_fps) {
            self.recording_gif_fps = 15;
        }
        if !RECORDING_FPS_OPTIONS.contains(&self.recording_webp_fps) {
            self.recording_webp_fps = 15;
        }
        self.annotation_color = normalize_color(&self.annotation_color);
        self.ocr_region = self.ocr_region.trim().to_string();
        if self.ocr_select_model > 1 {
            self.ocr_select_model = 0;
        }
    }

    pub fn annotation_rgba(&self) -> [u8; 4] {
        parse_color(&self.annotation_color).unwrap_or([255, 58, 72, 255])
    }
}

#[derive(Clone, Default)]
pub struct StatusSnapshot {
    pub active: bool,
    pub last_path: Option<String>,
    pub last_size: Option<(usize, usize)>,
    pub ocr_text: Option<String>,
    pub pinned: bool,
    pub error: Option<String>,
}

impl StatusSnapshot {
    pub fn value(&self) -> Value {
        json!({
            "active": self.active,
            "lastPath": self.last_path,
            "lastSize": self.last_size.map(|(width, height)| json!({"width": width, "height": height})),
            "ocrText": self.ocr_text,
            "pinned": self.pinned,
            "error": self.error,
        })
    }
}

/// Physical coordinates on the Windows virtual desktop.
pub type ScreenPoint = Point;
/// A rectangle expressed in virtual-desktop physical pixels.
pub type ScreenRect = RectI;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]

pub struct RectI {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RectI {
    pub fn normalized(a: Point, b: Point) -> Self {
        Self {
            left: a.x.min(b.x),
            top: a.y.min(b.y),
            right: a.x.max(b.x),
            bottom: a.y.max(b.y),
        }
    }

    pub fn width(&self) -> usize {
        (self.right - self.left).max(1) as usize
    }

    pub fn height(&self) -> usize {
        (self.bottom - self.top).max(1) as usize
    }

    pub fn clipped_to(self, bounds: RectI) -> Option<Self> {
        let clipped = Self {
            left: self.left.max(bounds.left),
            top: self.top.max(bounds.top),
            right: self.right.min(bounds.right),
            bottom: self.bottom.min(bounds.bottom),
        };
        (clipped.right - clipped.left > 0 && clipped.bottom - clipped.top > 0).then_some(clipped)
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.left
            && point.x <= self.right
            && point.y >= self.top
            && point.y <= self.bottom
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StrokeStyle {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Clone)]
pub enum Annotation {
    Rect(RectI, [u8; 4], u32, StrokeStyle),
    Ellipse(RectI, [u8; 4], u32, StrokeStyle),
    Arrow(Point, Point, [u8; 4], u32, StrokeStyle),
    Line(Point, Point, [u8; 4], u32, StrokeStyle),
    Highlight(Point, Point, [u8; 4], u32),
    Pen(Vec<Point>, [u8; 4], u32, StrokeStyle),
    Mosaic(Vec<Point>, u32),
    Text(Point, String, u32, [u8; 4]),
    Number(Point, u32, [u8; 4]),
}

pub fn parse_color(value: &str) -> Option<[u8; 4]> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 && value.len() != 8 {
        return None;
    }
    let parse = |part: &str| u8::from_str_radix(part, 16).ok();
    Some([
        parse(&value[0..2])?,
        parse(&value[2..4])?,
        parse(&value[4..6])?,
        if value.len() == 8 {
            parse(&value[6..8])?
        } else {
            255
        },
    ])
}

pub fn normalize_color(value: &str) -> String {
    parse_color(value)
        .map(|color| format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]))
        .unwrap_or_else(|| "#ff3a48".into())
}

#[cfg(test)]
mod tests {
    use super::{Point, RectI};

    #[test]
    fn physical_rect_clips_across_negative_virtual_desktop_coordinates() {
        let selection = RectI {
            left: -2400,
            top: -100,
            right: -1800,
            bottom: 400,
        };
        let desktop = RectI {
            left: -1920,
            top: -200,
            right: 1920,
            bottom: 1080,
        };
        let clipped = selection.clipped_to(desktop).expect("overlap");
        assert_eq!(clipped.left, -1920);
        assert_eq!(clipped.top, -100);
        assert_eq!(clipped.right, -1800);
        assert_eq!(clipped.bottom, 400);
    }

    #[test]
    fn physical_rect_contains_edges_and_rejects_outside_points() {
        let rect = RectI {
            left: -20,
            top: -10,
            right: 80,
            bottom: 40,
        };
        assert!(rect.contains(Point { x: -20, y: -10 }));
        assert!(rect.contains(Point { x: 80, y: 40 }));
        assert!(!rect.contains(Point { x: -21, y: 0 }));
        assert!(!rect.contains(Point { x: 0, y: 41 }));
    }
}
