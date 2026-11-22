use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::path::PathBuf;

pub const DEFAULT_HOTKEY: &str = "ctrl+shift+a";
pub const DEFAULT_RECORD_HOTKEY: &str = "alt+shift+r";
pub const DEFAULT_DEMO_HOTKEY: &str = "alt+shift+d";
pub const DEFAULT_SCROLL_HOTKEY: &str = "alt+shift+s";
pub const DEFAULT_QUICK_PIN_HOTKEY: &str = "alt+shift+p";
pub const RECORDING_FPS_OPTIONS: [u32; 8] = [5, 8, 10, 12, 15, 20, 24, 30];

pub const DEFAULT_TOOLBAR_ACTIVE: [&str; 17] = [
    "select",
    "rect",
    "arrow",
    "pen",
    "text",
    "number",
    "mosaic",
    "eraser",
    "undo",
    "redo",
    "record",
    "pin",
    "ocr",
    "translate",
    "scroll",
    "save",
    "finish",
];

pub const DEFAULT_TOOLBAR_INACTIVE: [&str; 2] = ["edit", "qr"];

pub fn default_toolbar_active() -> Vec<String> {
    DEFAULT_TOOLBAR_ACTIVE.iter().map(|s| (*s).to_string()).collect()
}

pub fn default_toolbar_inactive() -> Vec<String> {
    DEFAULT_TOOLBAR_INACTIVE
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

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
    // Hotkeys
    pub hotkey: String,
    pub record_hotkey: String,
    pub demo_hotkey: String,
    pub scroll_hotkey: String,
    pub quick_pin_hotkey: String,
    pub double_tap_f1: bool,

    // Appearance & SnowAir Dual Theme
    pub theme_mode: String,            // "system" | "dark" | "light"
    pub toolbar_theme: String,         // "system" | "dark" | "light" | "custom"
    pub toolbar_size: String,          // "small" | "medium" | "large"
    pub toolbar_custom_bg: String,
    pub toolbar_custom_hover: String,
    pub toolbar_custom_icon: String,
    pub toolbar_custom_accent: String,
    pub toolbar_custom_selection: String,
    pub toolbar_active: Vec<String>,
    pub toolbar_inactive: Vec<String>,

    // Capture & Output
    pub save_directory: String,
    pub file_name_template: String,
    pub save_format: String,           // "png" | "jpg" | "webp"
    pub auto_save: bool,
    pub auto_copy: bool,
    pub finish_action: String,         // "copy_and_save" | "copy_only" | "save_only"
    pub detect_window: bool,
    pub default_corner_radius: u32,
    pub default_shadow: bool,
    pub scroll_step: i32,
    pub hdr_capture_mode: HdrCaptureMode,
    pub watermark_text: String,

    // Initial Annotation Properties
    pub annotation_color: String,
    pub stroke_width: u32,
    pub brush_size: u32,
    pub text_size: u32,

    // Pin Settings
    pub pin_cursor_zoom: bool,
    pub pin_auto_ocr: bool,
    pub pin_double_click: String,      // "thumbnail" | "close" | "copy"

    // Demo / Presentation Settings
    pub demo_default_tool: String,     // "fade_pen" | "pen" | "none"
    pub demo_fade_ms: u32,
    pub demo_toolbar_pos: String,      // "top" | "bottom"

    // OCR & Translation
    pub ocr_engine: String,            // "system" | "tencent"
    pub ocr_region: String,
    pub ocr_select_model: u8,
    pub translate_engine: String,      // "youdao" | "microsoft" | "google" | "baidu"
    pub translate_target_lang: String, // "zh-CN" | "en" | "ja" | "ko" | "zh-TW"

    // Recording
    pub recording_fps: u32,
    pub recording_max_height: u32,
    pub recording_bitrate_mbps: u32,
    pub recording_gif_colors: u16,
    pub recording_webp_quality: u8,
    pub recording_show_cursor: bool,
    pub recording_default_passthrough: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.into(),
            record_hotkey: DEFAULT_RECORD_HOTKEY.into(),
            demo_hotkey: DEFAULT_DEMO_HOTKEY.into(),
            scroll_hotkey: DEFAULT_SCROLL_HOTKEY.into(),
            quick_pin_hotkey: DEFAULT_QUICK_PIN_HOTKEY.into(),
            double_tap_f1: true,

            theme_mode: "system".into(),
            toolbar_theme: "system".into(),
            toolbar_size: "medium".into(),
            toolbar_custom_bg: "#1e1e1f".into(),
            toolbar_custom_hover: "#333336".into(),
            toolbar_custom_icon: "#f2f2f5".into(),
            toolbar_custom_accent: "#34c759".into(),
            toolbar_custom_selection: "#34c759".into(),
            toolbar_active: default_toolbar_active(),
            toolbar_inactive: default_toolbar_inactive(),

            save_directory: String::new(),
            file_name_template: "SideWeave_{{YYYY}}{{MM}}{{DD}}_{{HH}}{{mm}}{{ss}}".into(),
            save_format: "png".into(),
            auto_save: false,
            auto_copy: true,
            finish_action: "copy_and_save".into(),
            detect_window: true,
            default_corner_radius: 0,
            default_shadow: false,
            scroll_step: 640,
            hdr_capture_mode: HdrCaptureMode::Auto,
            watermark_text: String::new(),

            annotation_color: "#ef4444".into(),
            stroke_width: 4,
            brush_size: 24,
            text_size: 18,

            pin_cursor_zoom: true,
            pin_auto_ocr: false,
            pin_double_click: "thumbnail".into(),

            demo_default_tool: "fade_pen".into(),
            demo_fade_ms: 2400,
            demo_toolbar_pos: "top".into(),

            ocr_engine: "system".into(),
            ocr_region: String::new(),
            ocr_select_model: 0,
            translate_engine: "youdao".into(),
            translate_target_lang: "zh-CN".into(),

            recording_fps: 15,
            recording_max_height: 1080,
            recording_bitrate_mbps: 16,
            recording_gif_colors: 256,
            recording_webp_quality: 90,
            recording_show_cursor: true,
            recording_default_passthrough: true,
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

    pub fn resolved_save_directory(&self) -> PathBuf {
        if self.save_directory.trim().is_empty() {
            env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|p| p.join("Pictures").join("SideWeave").join("Screenshots"))
                .unwrap_or_else(|| PathBuf::from("."))
        } else {
            PathBuf::from(self.save_directory.trim())
        }
    }

    pub fn load() -> Self {
        let Ok(contents) = std::fs::read_to_string(Self::path()) else {
            return Self::default();
        };
        let Ok(raw_settings) = serde_json::from_str::<Value>(&contents) else {
            return Self::default();
        };
        let has_unified_recording_settings = raw_settings.get("recordingFps").is_some()
            || raw_settings.get("recordingMaxHeight").is_some()
            || raw_settings.get("recordingBitrateMbps").is_some();
        let mut value: Self = serde_json::from_value(raw_settings.clone()).unwrap_or_default();
        if !has_unified_recording_settings {
            if let Some(fps) = raw_settings
                .get("recordingMp4Fps")
                .and_then(Value::as_u64)
                .and_then(|fps| u32::try_from(fps).ok())
            {
                value.recording_fps = fps;
            }
            if let Some(max_height) = raw_settings
                .get("recordingMp4MaxHeight")
                .and_then(Value::as_u64)
                .and_then(|height| u32::try_from(height).ok())
            {
                value.recording_max_height = max_height;
            }
            if let Some(bitrate) = raw_settings
                .get("recordingMp4BitrateMbps")
                .and_then(Value::as_u64)
                .and_then(|bitrate| u32::try_from(bitrate).ok())
            {
                value.recording_bitrate_mbps = bitrate;
            }
        }
        value.normalize();
        value
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
        self.stroke_width = self.stroke_width.clamp(1, 24);
        self.brush_size = self.brush_size.clamp(8, 80);
        self.default_corner_radius = self.default_corner_radius.min(60);
        self.demo_fade_ms = self.demo_fade_ms.clamp(600, 8000);
        if !RECORDING_FPS_OPTIONS.contains(&self.recording_fps) {
            self.recording_fps = 15;
        }
        if ![720, 1080].contains(&self.recording_max_height) {
            self.recording_max_height = 1080;
        }
        if ![4, 6, 8, 10, 12, 16, 20, 24].contains(&self.recording_bitrate_mbps) {
            self.recording_bitrate_mbps = 16;
        }
        if ![64, 128, 256].contains(&self.recording_gif_colors) {
            self.recording_gif_colors = 128;
        }
        if ![50, 65, 80, 90, 95].contains(&self.recording_webp_quality) {
            self.recording_webp_quality = 90;
        }
        if !matches!(self.theme_mode.as_str(), "system" | "dark" | "light") {
            self.theme_mode = "system".into();
        }
        if !matches!(self.toolbar_theme.as_str(), "system" | "dark" | "light" | "custom") {
            self.toolbar_theme = "system".into();
        }
        if !matches!(self.toolbar_size.as_str(), "small" | "medium" | "large") {
            self.toolbar_size = "medium".into();
        }
        if !matches!(self.save_format.as_str(), "png" | "jpg" | "webp") {
            self.save_format = "png".into();
        }
        if !matches!(
            self.finish_action.as_str(),
            "copy_and_save" | "copy_only" | "save_only"
        ) {
            self.finish_action = "copy_and_save".into();
        }
        if !matches!(self.pin_double_click.as_str(), "thumbnail" | "close" | "copy") {
            self.pin_double_click = "thumbnail".into();
        }
        if !matches!(self.demo_default_tool.as_str(), "fade_pen" | "pen" | "none") {
            self.demo_default_tool = "fade_pen".into();
        }
        if !matches!(self.demo_toolbar_pos.as_str(), "top" | "bottom") {
            self.demo_toolbar_pos = "top".into();
        }
        if !matches!(self.ocr_engine.as_str(), "system" | "tencent") {
            self.ocr_engine = "system".into();
        }
        if !matches!(
            self.translate_engine.as_str(),
            "youdao" | "microsoft" | "google" | "baidu"
        ) {
            self.translate_engine = "youdao".into();
        }
        if self.translate_target_lang.trim().is_empty() {
            self.translate_target_lang = "zh-CN".into();
        }
        if self.file_name_template.trim().is_empty() {
            self.file_name_template = "SideWeave_{{YYYY}}{{MM}}{{DD}}_{{HH}}{{mm}}{{ss}}".into();
        }
        self.annotation_color = normalize_color(&self.annotation_color);
        self.toolbar_custom_bg = normalize_color(&self.toolbar_custom_bg);
        self.toolbar_custom_hover = normalize_color(&self.toolbar_custom_hover);
        self.toolbar_custom_icon = normalize_color(&self.toolbar_custom_icon);
        self.toolbar_custom_accent = normalize_color(&self.toolbar_custom_accent);
        self.toolbar_custom_selection = normalize_color(&self.toolbar_custom_selection);
        self.ocr_region = self.ocr_region.trim().to_string();
        if self.ocr_select_model > 1 {
            self.ocr_select_model = 0;
        }
        if self.toolbar_active.is_empty() {
            self.toolbar_active = default_toolbar_active();
        }
    }

    pub fn annotation_rgba(&self) -> [u8; 4] {
        parse_color(&self.annotation_color).unwrap_or([239, 68, 68, 255])
    }
}

#[derive(Clone, Default)]
pub struct StatusSnapshot {
    pub active: bool,
    pub demo_active: bool,
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
            "demoActive": self.demo_active,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

    pub fn expanded(self, pad: i32) -> Self {
        Self {
            left: self.left - pad,
            top: self.top - pad,
            right: self.right + pad,
            bottom: self.bottom + pad,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeStyle {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowVariant {
    Single,
    Double,
    Curved,
}

#[derive(Clone, Debug)]
pub struct OcrRegionBox {
    /// Coordinates relative to the captured selection image.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub text: String,
    pub translated: Option<String>,
}

#[derive(Clone)]
pub enum Annotation {
    Rect {
        rect: RectI,
        color: [u8; 4],
        width: u32,
        style: StrokeStyle,
        filled: bool,
        rounded: bool,
    },
    Ellipse {
        rect: RectI,
        color: [u8; 4],
        width: u32,
        style: StrokeStyle,
        filled: bool,
    },
    Triangle {
        rect: RectI,
        color: [u8; 4],
        width: u32,
        style: StrokeStyle,
        filled: bool,
    },
    Star {
        rect: RectI,
        color: [u8; 4],
        width: u32,
        style: StrokeStyle,
        filled: bool,
    },
    Bubble {
        rect: RectI,
        color: [u8; 4],
        width: u32,
        style: StrokeStyle,
    },
    Burst {
        rect: RectI,
        color: [u8; 4],
        width: u32,
        style: StrokeStyle,
    },
    Arrow {
        start: Point,
        end: Point,
        color: [u8; 4],
        width: u32,
        style: StrokeStyle,
        variant: ArrowVariant,
    },
    Line(Point, Point, [u8; 4], u32, StrokeStyle),
    Dimension {
        start: Point,
        end: Point,
        color: [u8; 4],
        width: u32,
    },
    Angle {
        start: Point,
        end: Point,
        color: [u8; 4],
        width: u32,
    },
    Wave {
        start: Point,
        end: Point,
        color: [u8; 4],
        width: u32,
    },
    Highlight(Point, Point, [u8; 4], u32),
    /// Elliptical focus area; pixels outside the union of all spotlight shapes are dimmed.
    Spotlight(RectI),
    Pen(Vec<Point>, [u8; 4], u32, StrokeStyle),
    FadePen {
        points: Vec<Point>,
        color: [u8; 4],
        width: u32,
        created_ms: u64,
    },
    Mosaic(Vec<Point>, u32),
    Patina {
        rect: RectI,
        intensity: u8,
    },
    Watermark {
        rect: RectI,
        text: String,
        color: [u8; 4],
    },
    Text(Point, String, u32, [u8; 4]),
    Number(Point, u32, [u8; 4]),
}

impl Annotation {
    /// Coarse bounding box for eraser hit-testing.
    pub fn hit_test(&self, point: Point, tolerance: i32) -> bool {
        let tol = tolerance.max(8);
        match self {
            Annotation::Rect { rect, .. }
            | Annotation::Ellipse { rect, .. }
            | Annotation::Triangle { rect, .. }
            | Annotation::Star { rect, .. }
            | Annotation::Bubble { rect, .. }
            | Annotation::Burst { rect, .. }
            | Annotation::Spotlight(rect)
            | Annotation::Patina { rect, .. }
            | Annotation::Watermark { rect, .. } => rect.expanded(tol).contains(point),
            Annotation::Arrow { start, end, .. }
            | Annotation::Line(start, end, ..)
            | Annotation::Dimension { start, end, .. }
            | Annotation::Angle { start, end, .. }
            | Annotation::Wave { start, end, .. }
            | Annotation::Highlight(start, end, ..) => {
                RectI::normalized(*start, *end).expanded(tol).contains(point)
            }
            Annotation::Pen(points, _, _, _)
            | Annotation::FadePen { points, .. }
            | Annotation::Mosaic(points, _) => points.iter().any(|p| {
                (p.x - point.x).abs() <= tol + 6 && (p.y - point.y).abs() <= tol + 6
            }),
            Annotation::Text(pos, text, size, _) => {
                let w = (text.chars().count().max(2) as i32) * (*size as i32);
                let h = (*size as i32 * 2).max(24);
                RectI {
                    left: pos.x - tol,
                    top: pos.y - tol,
                    right: pos.x + w + tol,
                    bottom: pos.y + h + tol,
                }
                .contains(point)
            }
            Annotation::Number(pos, _, _) => {
                let dx = point.x - pos.x;
                let dy = point.y - pos.y;
                dx * dx + dy * dy <= (16 + tol) * (16 + tol)
            }
        }
    }
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
        .unwrap_or_else(|| "#ef4444".into())
}

#[cfg(test)]
mod tests {
    use super::{Point, RectI, Settings};

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

    #[test]
    fn default_settings_normalize_cleanly() {
        let mut s = Settings::default();
        s.normalize();
        assert!(!s.toolbar_active.is_empty());
        assert_eq!(s.ocr_engine, "system");
        assert_eq!(s.translate_engine, "youdao");
    }
}
