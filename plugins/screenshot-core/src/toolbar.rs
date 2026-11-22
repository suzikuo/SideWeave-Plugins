//! SnowAir-inspired toolbar, secondary property bar, info capsule, and demo bar models.
//! Pure layout & hit-testing logic decoupled from Win32 painting.

use crate::model::{ScreenRect, Settings};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    Select,
    // Shapes group
    Rectangle,
    Ellipse,
    Triangle,
    Star,
    Bubble,
    Burst,
    // Arrow / Measurement group
    Arrow,
    DoubleArrow,
    CurvedArrow,
    Line,
    Dimension,
    Angle,
    Wave,
    // Pen group
    Pen,
    Highlight,
    FadePen,
    // Effects / Privacy group
    Mosaic,
    Patina,
    Watermark,
    Spotlight,
    // Direct tools
    Text,
    Number,
    Eraser,
}

impl Tool {
    pub fn is_shape_group(self) -> bool {
        matches!(
            self,
            Tool::Rectangle
                | Tool::Ellipse
                | Tool::Triangle
                | Tool::Star
                | Tool::Bubble
                | Tool::Burst
        )
    }

    pub fn is_arrow_group(self) -> bool {
        matches!(
            self,
            Tool::Arrow
                | Tool::DoubleArrow
                | Tool::CurvedArrow
                | Tool::Line
                | Tool::Dimension
                | Tool::Angle
                | Tool::Wave
        )
    }

    pub fn is_pen_group(self) -> bool {
        matches!(self, Tool::Pen | Tool::Highlight | Tool::FadePen)
    }

    pub fn is_effect_group(self) -> bool {
        matches!(
            self,
            Tool::Mosaic | Tool::Patina | Tool::Watermark | Tool::Spotlight
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "选择 / 移动",
            Tool::Rectangle => "矩形",
            Tool::Ellipse => "椭圆",
            Tool::Triangle => "三角形",
            Tool::Star => "星形",
            Tool::Bubble => "气泡",
            Tool::Burst => "爆裂",
            Tool::Arrow => "箭头",
            Tool::DoubleArrow => "双箭头",
            Tool::CurvedArrow => "趣味箭头",
            Tool::Line => "直线",
            Tool::Dimension => "尺寸标注",
            Tool::Angle => "角度标注",
            Tool::Wave => "波浪线",
            Tool::Pen => "画笔",
            Tool::Highlight => "荧光高亮",
            Tool::FadePen => "渐隐画笔",
            Tool::Mosaic => "马赛克",
            Tool::Patina => "电子包浆",
            Tool::Watermark => "斜纹水印",
            Tool::Spotlight => "聚光高亮",
            Tool::Text => "文字",
            Tool::Number => "序号",
            Tool::Eraser => "橡皮擦",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarAction {
    MoveToolbar,
    Tool(Tool),
    PlaceNumber,
    Pin,
    Scroll,
    Undo,
    Redo,
    Copy,
    Save,
    Cancel,
    Finish,
    Edit,
    Record,
    Ocr,
    Translate,
    Qr,
    ScrollPauseToggle,
    // Demo mode actions
    DemoPassThrough,
    DemoBackground,
    DemoClear,
}

impl ToolbarAction {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "select" => Some(Self::Tool(Tool::Select)),
            "rect" => Some(Self::Tool(Tool::Rectangle)),
            "arrow" => Some(Self::Tool(Tool::Arrow)),
            "pen" => Some(Self::Tool(Tool::Pen)),
            "text" => Some(Self::Tool(Tool::Text)),
            "number" => Some(Self::Tool(Tool::Number)),
            "mosaic" => Some(Self::Tool(Tool::Mosaic)),
            "eraser" => Some(Self::Tool(Tool::Eraser)),
            "undo" => Some(Self::Undo),
            "redo" => Some(Self::Redo),
            "record" => Some(Self::Record),
            "pin" => Some(Self::Pin),
            "ocr" => Some(Self::Ocr),
            "translate" => Some(Self::Translate),
            "scroll" => Some(Self::Scroll),
            "edit" => Some(Self::Edit),
            "qr" => Some(Self::Qr),
            "save" => Some(Self::Save),
            "cancel" => Some(Self::Cancel),
            "finish" => Some(Self::Finish),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MoveToolbar => "拖拽移动工具栏",
            Self::Tool(t) => t.label(),
            Self::PlaceNumber => "序号",
            Self::Pin => "贴图置顶",
            Self::Scroll => "滚动长截图",
            Self::Undo => "撤销 (Ctrl+Z)",
            Self::Redo => "重做 (Ctrl+Y)",
            Self::Copy => "复制图片",
            Self::Save => "另存为 (Ctrl+S)",
            Self::Cancel => "取消 (Esc)",
            Self::Finish => "完成并复制 (Enter)",
            Self::Edit => "编辑窗口",
            Self::Record => "区域录屏",
            Self::Ocr => "OCR 文字识别",
            Self::Translate => "图片翻译",
            Self::Qr => "识别二维码",
            Self::ScrollPauseToggle => "开始 / 暂停 (Space)",
            Self::DemoPassThrough => "鼠标穿透 (`)",
            Self::DemoBackground => "白板 / 背景",
            Self::DemoClear => "清空画布",
        }
    }

    /// True if this action visually represents the active tool (including tool group membership).
    pub fn matches_tool(self, active: Tool) -> bool {
        match self {
            Self::Tool(Tool::Rectangle) => active.is_shape_group(),
            Self::Tool(Tool::Arrow) => active.is_arrow_group(),
            Self::Tool(Tool::Pen) => active.is_pen_group(),
            Self::Tool(Tool::Mosaic) => active.is_effect_group(),
            Self::Tool(t) => t == active,
            _ => false,
        }
    }

    pub fn needs_leading_divider(self) -> bool {
        matches!(self, Self::Undo | Self::Record | Self::DemoClear)
    }
}

/// SnowAir toolbar base logical geometry (44dp slots, 56dp height at 100% DPI;
/// automatically scaled by Windows DPI + user toolbarSize preference!).
pub const TOOLBAR_BUTTON_WIDTH: i32 = 44;
pub const TOOLBAR_BUTTON_HEIGHT: i32 = 44;
pub const TOOLBAR_VERTICAL_PADDING: i32 = 6;
pub const TOOLBAR_HORIZONTAL_PADDING: i32 = 10;

/// Build dynamic main toolbar action list from user's customized settings.
/// Always keeps `MoveToolbar` at start and `Cancel` / `Finish` at end.
pub fn build_main_actions(settings: &Settings) -> Vec<ToolbarAction> {
    let mut actions = Vec::with_capacity(22);
    actions.push(ToolbarAction::MoveToolbar);
    let mut has_finish = false;
    for id in &settings.toolbar_active {
        if let Some(act) = ToolbarAction::from_id(id) {
            if act == ToolbarAction::Cancel {
                continue;
            }
            if act == ToolbarAction::Finish {
                has_finish = true;
                continue;
            }
            if !actions.contains(&act) {
                actions.push(act);
            }
        }
    }
    actions.push(ToolbarAction::Cancel);
    if has_finish || !settings.toolbar_inactive.iter().any(|s| s == "finish") {
        actions.push(ToolbarAction::Finish);
    }
    actions
}

pub const DEMO_TOOLBAR_ACTIONS: [ToolbarAction; 14] = [
    ToolbarAction::MoveToolbar,
    ToolbarAction::DemoPassThrough,
    ToolbarAction::DemoBackground,
    ToolbarAction::Tool(Tool::Rectangle),
    ToolbarAction::Tool(Tool::Ellipse),
    ToolbarAction::Tool(Tool::Arrow),
    ToolbarAction::Tool(Tool::Pen),
    ToolbarAction::Tool(Tool::FadePen),
    ToolbarAction::Tool(Tool::Text),
    ToolbarAction::Tool(Tool::Number),
    ToolbarAction::Tool(Tool::Eraser),
    ToolbarAction::Undo,
    ToolbarAction::DemoClear,
    ToolbarAction::Cancel,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolbarLayout {
    pub item_count: usize,
    pub columns: usize,
    pub rows: usize,
    pub width: i32,
    pub height: i32,
    pub button_w: i32,
    pub button_h: i32,
    pub pad_x: i32,
    pub pad_y: i32,
    pub scale_milli: u32,
}

impl ToolbarLayout {
    pub fn scale(self) -> f32 {
        (self.scale_milli.max(500) as f32) / 1000.0
    }
}

pub fn layout_for_item_count(item_count: usize, screen_width: i32) -> ToolbarLayout {
    layout_for_item_count_scaled(item_count, screen_width, 1.0)
}

pub fn layout_for_item_count_scaled(
    item_count: usize,
    screen_width: i32,
    scale: f32,
) -> ToolbarLayout {
    let item_count = item_count.max(1);
    let scale = scale.clamp(0.75, 2.75);
    let button_w = (TOOLBAR_BUTTON_WIDTH as f32 * scale).round() as i32;
    let button_h = (TOOLBAR_BUTTON_HEIGHT as f32 * scale).round() as i32;
    let pad_x = (TOOLBAR_HORIZONTAL_PADDING as f32 * scale).round() as i32;
    let pad_y = (TOOLBAR_VERTICAL_PADDING as f32 * scale).round() as i32;
    let single_row_width = item_count as i32 * button_w + pad_x * 2 + 16;
    let columns = if screen_width < single_row_width {
        let available_columns =
            ((screen_width - 24 - pad_x * 2) / button_w.max(1)).clamp(1, item_count as i32)
                as usize;
        item_count.div_ceil(2).min(available_columns).max(1)
    } else {
        item_count
    };
    let rows = item_count.div_ceil(columns);
    ToolbarLayout {
        item_count,
        columns,
        rows,
        width: columns as i32 * button_w + pad_x * 2,
        height: rows as i32 * button_h + pad_y * 2,
        button_w,
        button_h,
        pad_x,
        pad_y,
        scale_milli: (scale * 1000.0).round() as u32,
    }
}

pub fn button_origin(
    origin: (i32, i32),
    index: usize,
    layout: ToolbarLayout,
) -> Option<(i32, i32)> {
    if index >= layout.item_count {
        return None;
    }
    let row = index / layout.columns;
    let row_start = row * layout.columns;
    let row_item_count = if row + 1 == layout.rows {
        layout.item_count - row_start
    } else {
        layout.columns
    };
    let right_alignment = layout.columns - row_item_count;
    let column = index % layout.columns + right_alignment;
    Some((
        origin.0 + layout.pad_x + column as i32 * layout.button_w,
        origin.1 + layout.pad_y + row as i32 * layout.button_h,
    ))
}

pub fn hit_test(origin: (i32, i32), point: (i32, i32), layout: ToolbarLayout) -> Option<usize> {
    let (x, y) = origin;
    let (px, py) = point;
    let inner_x = x + layout.pad_x;
    let inner_w = layout.columns as i32 * layout.button_w;
    if px < inner_x
        || px >= inner_x + inner_w
        || py < y + layout.pad_y
        || py >= y + layout.height - layout.pad_y
    {
        return None;
    }
    let row = ((py - y - layout.pad_y) / layout.button_h.max(1)) as usize;
    if row >= layout.rows {
        return None;
    }
    let column = ((px - inner_x) / layout.button_w.max(1)) as usize;
    let row_start = row * layout.columns;
    let row_item_count = if row + 1 == layout.rows {
        layout.item_count - row_start
    } else {
        layout.columns
    };
    let right_alignment = layout.columns - row_item_count;
    if column < right_alignment {
        return None;
    }
    let index = row_start + column - right_alignment;
    (index < layout.item_count).then_some(index)
}

// -------------------------------------------------------------------------
// SnowAir Top-Left Selection Info Capsule (X, Y | Lock | W*H px | Radius Slider | Shadow)
// -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InfoCapsuleLayout {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub scale_milli: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InfoCapsuleHit {
    LockAspect,
    RadiusSlider(u32), // 0..=60
    ShadowToggle,
}

impl InfoCapsuleLayout {
    pub fn scale(self) -> f32 {
        (self.scale_milli.max(500) as f32) / 1000.0
    }

    pub fn for_selection(selection: ScreenRect, screen: ScreenRect) -> Self {
        Self::for_selection_scaled(selection, screen, 1.0)
    }

    pub fn for_selection_scaled(selection: ScreenRect, screen: ScreenRect, scale: f32) -> Self {
        let s = scale.clamp(0.85, 2.5);
        let width = (432.0 * s).round() as i32;
        let height = (38.0 * s).round() as i32;
        let gap = (10.0 * s).round() as i32;
        let left = selection.left.clamp(
            screen.left + 8,
            (screen.right - width - 8).max(screen.left + 8),
        );
        let top = if selection.top >= screen.top + height + gap + 2 {
            selection.top - height - gap
        } else {
            (selection.top + gap).min(screen.bottom - height - 8)
        };
        Self {
            left,
            top,
            width,
            height,
            scale_milli: (s * 1000.0).round() as u32,
        }
    }

    fn px(self, dp: f32) -> i32 {
        (dp * self.scale()).round() as i32
    }

    pub fn lock_rect(self) -> (i32, i32, i32, i32) {
        (
            self.left + self.px(124.0),
            self.top + self.px(4.0),
            self.left + self.px(152.0),
            self.top + self.height - self.px(4.0),
        )
    }

    pub fn slider_track_rect(self) -> (i32, i32, i32, i32) {
        (
            self.left + self.px(284.0),
            self.top + self.px(6.0),
            self.left + self.px(364.0),
            self.top + self.height - self.px(6.0),
        )
    }

    pub fn shadow_rect(self) -> (i32, i32, i32, i32) {
        (
            self.left + self.px(394.0),
            self.top + self.px(4.0),
            self.left + self.px(426.0),
            self.top + self.height - self.px(4.0),
        )
    }

    pub fn hit_test(self, px: i32, py: i32) -> Option<InfoCapsuleHit> {
        if px < self.left
            || px >= self.left + self.width
            || py < self.top
            || py >= self.top + self.height
        {
            return None;
        }
        let in_r = |r: (i32, i32, i32, i32)| px >= r.0 && px < r.2 && py >= r.1 && py < r.3;
        if in_r(self.lock_rect()) {
            return Some(InfoCapsuleHit::LockAspect);
        }
        let s = self.slider_track_rect();
        let pad = self.px(8.0);
        if px >= s.0 - pad && px <= s.2 + pad && py >= self.top && py < self.top + self.height {
            let ratio = ((px - s.0) as f32 / (s.2 - s.0).max(1) as f32).clamp(0.0, 1.0);
            return Some(InfoCapsuleHit::RadiusSlider((ratio * 60.0).round() as u32));
        }
        if in_r(self.shadow_rect()) {
            return Some(InfoCapsuleHit::ShadowToggle);
        }
        None
    }
}

// -------------------------------------------------------------------------
// SnowAir Secondary Property Bar (Variants + Styles + 7 Colors + 4 Sizes)
// -------------------------------------------------------------------------

/// SnowAir 7-color palette (Red, Orange, SnowAir Green, Blue, Purple, Black, White)
pub const COLOR_SWATCHES: [[u8; 4]; 7] = [
    [239, 68, 68, 255],   // Red #EF4444
    [249, 115, 22, 255],  // Orange #F97316
    [52, 199, 89, 255],   // SnowAir Green #34C759
    [59, 130, 246, 255],  // Blue #3B82F6
    [168, 85, 247, 255],  // Purple #A855F7
    [24, 24, 27, 255],    // Dark #18181B
    [255, 255, 255, 255], // White #FFFFFF
];

pub const STROKE_SIZES: [u32; 4] = [2, 4, 7, 12];
pub const TEXT_SIZES: [u32; 4] = [14, 18, 24, 34];
pub const MOSAIC_SIZES: [u32; 4] = [12, 22, 36, 56];

pub const SHAPE_VARIANTS: [Tool; 6] = [
    Tool::Rectangle,
    Tool::Ellipse,
    Tool::Triangle,
    Tool::Star,
    Tool::Bubble,
    Tool::Burst,
];

pub const ARROW_VARIANTS: [Tool; 7] = [
    Tool::Line,
    Tool::Arrow,
    Tool::DoubleArrow,
    Tool::CurvedArrow,
    Tool::Dimension,
    Tool::Angle,
    Tool::Wave,
];

pub const PEN_VARIANTS: [Tool; 3] = [Tool::Pen, Tool::Highlight, Tool::FadePen];

pub const EFFECT_VARIANTS: [Tool; 4] = [
    Tool::Mosaic,
    Tool::Patina,
    Tool::Watermark,
    Tool::Spotlight,
];

pub fn variants_for_tool(tool: Tool) -> &'static [Tool] {
    if tool.is_shape_group() {
        &SHAPE_VARIANTS
    } else if tool.is_arrow_group() {
        &ARROW_VARIANTS
    } else if tool.is_pen_group() {
        &PEN_VARIANTS
    } else if tool.is_effect_group() {
        &EFFECT_VARIANTS
    } else {
        &[]
    }
}

pub fn tool_has_secondary_bar(tool: Tool) -> bool {
    !matches!(tool, Tool::Select | Tool::Eraser)
}

pub fn tool_has_style_toggles(tool: Tool) -> bool {
    tool.is_shape_group() || tool.is_arrow_group()
}

pub fn tool_has_color_picker(tool: Tool) -> bool {
    !matches!(tool, Tool::Mosaic | Tool::Patina | Tool::Spotlight)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecondaryBarItem {
    Variant(Tool),
    ToggleFilled,
    ToggleDashed,
    ToggleRounded,
    Size(usize),
    Color(usize),
}

impl SecondaryBarItem {
    pub fn tooltip(self, tool: Tool) -> &'static str {
        match self {
            Self::Variant(t) => t.label(),
            Self::ToggleFilled => "实心填充",
            Self::ToggleDashed => "虚线样式",
            Self::ToggleRounded => "圆角矩形",
            Self::Size(i) => {
                if tool == Tool::Text {
                    ["14px", "18px", "24px", "34px"].get(i).copied().unwrap_or("")
                } else {
                    ["细", "中", "粗", "特粗"].get(i).copied().unwrap_or("")
                }
            }
            Self::Color(i) => ["红色", "橙色", "轻雪绿", "蓝色", "紫色", "炭黑", "纯白"]
                .get(i)
                .copied()
                .unwrap_or(""),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecondaryBarLayout {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub tool: Tool,
    pub variant_count: usize,
    pub has_styles: bool,
    pub has_size: bool,
    pub has_color: bool,
    pub scale_milli: u32,
}

impl SecondaryBarLayout {
    pub fn scale(self) -> f32 {
        (self.scale_milli.max(500) as f32) / 1000.0
    }

    fn px(&self, dp: f32) -> i32 {
        (dp * self.scale()).round() as i32
    }

    fn pad_x(&self) -> i32 {
        self.px(12.0)
    }

    fn item_w(&self) -> i32 {
        self.px(38.0)
    }

    fn style_w(&self) -> i32 {
        self.px(34.0)
    }

    fn size_w(&self) -> i32 {
        self.px(30.0)
    }

    fn gap(&self) -> i32 {
        self.px(14.0)
    }

    fn styles_start_x(&self) -> i32 {
        let mut x = self.left + self.pad_x();
        if self.variant_count > 0 {
            x += self.variant_count as i32 * self.item_w() + self.gap();
        }
        x
    }

    fn style_count(&self) -> usize {
        if !self.has_styles {
            0
        } else if self.tool.is_shape_group() {
            3 // Filled, Dashed, Rounded
        } else {
            1 // Dashed
        }
    }

    fn size_start_x(&self) -> i32 {
        let mut x = self.styles_start_x();
        let sc = self.style_count();
        if sc > 0 {
            x += sc as i32 * self.style_w() + self.gap();
        }
        x
    }

    fn color_start_x(&self) -> i32 {
        let mut x = self.size_start_x();
        if self.has_size {
            x += 4 * self.size_w() + self.gap();
        }
        x
    }

    pub fn variant_rect(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        if index >= self.variant_count {
            return None;
        }
        let l = self.left + self.pad_x() + index as i32 * self.item_w();
        let vy = self.px(5.0);
        Some((l, self.top + vy, l + self.item_w(), self.top + self.height - vy))
    }

    pub fn style_rect(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        if index >= self.style_count() {
            return None;
        }
        let l = self.styles_start_x() + index as i32 * self.style_w();
        let vy = self.px(5.0);
        Some((l, self.top + vy, l + self.style_w(), self.top + self.height - vy))
    }

    pub fn size_rect(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        if !self.has_size || index >= 4 {
            return None;
        }
        let l = self.size_start_x() + index as i32 * self.size_w();
        let vy = self.px(5.0);
        Some((l, self.top + vy, l + self.size_w(), self.top + self.height - vy))
    }

    pub fn color_rect(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        if !self.has_color || index >= COLOR_SWATCHES.len() {
            return None;
        }
        let item_w = self.px(22.0);
        let gap = self.px(7.0);
        let l = self.color_start_x() + index as i32 * (item_w + gap);
        let t = self.top + (self.height - item_w) / 2;
        Some((l, t, l + item_w, t + item_w))
    }

    pub fn divider_xs(&self) -> Vec<i32> {
        let mut xs = Vec::new();
        let half_gap = self.gap() / 2;
        if self.variant_count > 0 && (self.style_count() > 0 || self.has_size || self.has_color) {
            xs.push(self.styles_start_x() - half_gap);
        }
        if self.style_count() > 0 && (self.has_size || self.has_color) {
            xs.push(self.size_start_x() - half_gap);
        }
        if self.has_size && self.has_color {
            xs.push(self.color_start_x() - half_gap);
        }
        xs
    }
}

pub fn secondary_bar_layout(
    toolbar_origin: (i32, i32),
    toolbar: ToolbarLayout,
    tool: Tool,
    screen_rect: ScreenRect,
    selection: ScreenRect,
) -> Option<SecondaryBarLayout> {
    if !tool_has_secondary_bar(tool) {
        return None;
    }
    let scale = toolbar.scale();
    let variants = variants_for_tool(tool);
    let has_styles = tool_has_style_toggles(tool);
    let has_color = tool_has_color_picker(tool);
    let has_size = !matches!(tool, Tool::Spotlight | Tool::Watermark);
    let height = (48.0 * scale).round() as i32;
    let mut temp = SecondaryBarLayout {
        left: 0,
        top: 0,
        width: 0,
        height,
        tool,
        variant_count: variants.len(),
        has_styles,
        has_size,
        has_color,
        scale_milli: toolbar.scale_milli,
    };
    let swatch_w = temp.px(22.0);
    let swatch_gap = temp.px(7.0);
    let end_x = if has_color {
        temp.color_start_x()
            + (COLOR_SWATCHES.len() as i32 * (swatch_w + swatch_gap) - swatch_gap)
            + temp.pad_x()
    } else if has_size {
        temp.size_start_x() + 4 * temp.size_w() + temp.pad_x()
    } else if temp.style_count() > 0 {
        temp.styles_start_x() + temp.style_count() as i32 * temp.style_w() + temp.pad_x()
    } else {
        temp.pad_x() * 2 + variants.len() as i32 * temp.item_w()
    };
    let width = end_x.max(120).min((screen_rect.width() as i32 - 16).max(120));
    let bar_gap = (8.0 * scale).round() as i32;
    let (tx, ty) = toolbar_origin;
    let tb = ScreenRect {
        left: tx,
        top: ty,
        right: tx + toolbar.width,
        bottom: ty + toolbar.height,
    };
    let left_default = tx.clamp(
        screen_rect.left + 8,
        (screen_rect.right - width - 8).max(screen_rect.left + 8),
    );
    let candidates = [
        (left_default, tb.bottom + bar_gap),
        (left_default, tb.top - height - bar_gap),
        (left_default, selection.bottom + bar_gap),
        (left_default, selection.top - height - bar_gap),
    ];
    let mut chosen = None;
    for (cx, cy) in candidates {
        let x = cx.clamp(
            screen_rect.left + 8,
            (screen_rect.right - width - 8).max(screen_rect.left + 8),
        );
        let y = cy.clamp(
            screen_rect.top + 8,
            (screen_rect.bottom - height - 8).max(screen_rect.top + 8),
        );
        let rc = ScreenRect {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        };
        if !rects_intersect(rc, tb) && !rects_intersect(rc, selection) {
            chosen = Some((x, y));
            break;
        }
    }
    let (left, top) = chosen.unwrap_or((
        left_default,
        (tb.bottom + bar_gap).clamp(
            screen_rect.top + 8,
            (screen_rect.bottom - height - 8).max(screen_rect.top + 8),
        ),
    ));
    temp.left = left;
    temp.top = top;
    temp.width = width;
    Some(temp)
}

pub fn secondary_bar_hit_test(
    layout: SecondaryBarLayout,
    point: (i32, i32),
) -> Option<SecondaryBarItem> {
    let (px, py) = point;
    if px < layout.left
        || px >= layout.left + layout.width
        || py < layout.top
        || py >= layout.top + layout.height
    {
        return None;
    }
    let variants = variants_for_tool(layout.tool);
    for (i, &v) in variants.iter().enumerate() {
        if let Some((l, t, r, b)) = layout.variant_rect(i) {
            if px >= l && px < r && py >= t && py < b {
                return Some(SecondaryBarItem::Variant(v));
            }
        }
    }
    if layout.has_styles {
        if layout.tool.is_shape_group() {
            let items = [
                SecondaryBarItem::ToggleFilled,
                SecondaryBarItem::ToggleDashed,
                SecondaryBarItem::ToggleRounded,
            ];
            for (i, item) in items.into_iter().enumerate() {
                if let Some((l, t, r, b)) = layout.style_rect(i) {
                    if px >= l && px < r && py >= t && py < b {
                        return Some(item);
                    }
                }
            }
        } else if let Some((l, t, r, b)) = layout.style_rect(0) {
            if px >= l && px < r && py >= t && py < b {
                return Some(SecondaryBarItem::ToggleDashed);
            }
        }
    }
    if layout.has_size {
        for i in 0..4 {
            if let Some((l, t, r, b)) = layout.size_rect(i) {
                if px >= l && px < r && py >= t && py < b {
                    return Some(SecondaryBarItem::Size(i));
                }
            }
        }
    }
    if layout.has_color {
        for i in 0..COLOR_SWATCHES.len() {
            if let Some((l, t, r, b)) = layout.color_rect(i) {
                if px >= l && px < r && py >= t && py < b {
                    return Some(SecondaryBarItem::Color(i));
                }
            }
        }
    }
    None
}

fn rects_intersect(a: ScreenRect, b: ScreenRect) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

// -------------------------------------------------------------------------
// Menus & Recording Setup Card
// -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarMenu {
    ScrollMode,
    Recognition,
    RecordingFps,
    RecordingExport,
    DemoBg,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordingSetupLayout {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingSetupAction {
    Close,
    Start,
    ToggleCursor,
    Ignore,
}

impl RecordingSetupLayout {
    pub fn close_rect(self) -> (i32, i32, i32, i32) {
        (self.left + self.width - 34, self.top + 3, self.left + self.width - 10, self.top + 18)
    }

    pub fn start_rect(self) -> (i32, i32, i32, i32) {
        (self.left + 24, self.top + 20, self.left + self.width - 24, self.top + 68)
    }

    pub fn output_note_rect(self) -> (i32, i32, i32, i32) {
        (
            self.left + 138,
            self.top + 88,
            self.left + self.width - 16,
            self.top + 126,
        )
    }

    pub fn control_rect(self, index: usize) -> Option<(i32, i32, i32, i32)> {
        (index < 4).then_some((
            self.left + 12 + index as i32 * 104,
            self.top + 140,
            self.left + 116 + index as i32 * 104,
            self.top + 226,
        ))
    }
}

pub fn recording_setup_layout(selection: ScreenRect, screen: ScreenRect) -> RecordingSetupLayout {
    let width = 440;
    let height = 232;
    let margin = 12;
    let left = (selection.left + (selection.width() as i32 - width) / 2).clamp(
        screen.left + margin,
        (screen.right - width - margin).max(screen.left + margin),
    );
    let below = selection.bottom + margin;
    let above = selection.top - height - margin;
    let top = if below + height <= screen.bottom - margin {
        below
    } else if above >= screen.top + margin {
        above
    } else {
        (screen.bottom - height - margin).max(screen.top + margin)
    };
    RecordingSetupLayout {
        left,
        top,
        width,
        height,
    }
}

pub fn recording_setup_hit_test(
    layout: RecordingSetupLayout,
    point: (i32, i32),
) -> Option<RecordingSetupAction> {
    let inside = |rect: (i32, i32, i32, i32)| {
        point.0 >= rect.0 && point.0 < rect.2 && point.1 >= rect.1 && point.1 < rect.3
    };
    if inside(layout.close_rect()) {
        return Some(RecordingSetupAction::Close);
    }
    if inside(layout.start_rect()) {
        return Some(RecordingSetupAction::Start);
    }
    if layout.control_rect(3).is_some_and(inside) {
        return Some(RecordingSetupAction::ToggleCursor);
    }
    Some(RecordingSetupAction::Ignore).filter(|_| {
        point.0 >= layout.left
            && point.0 < layout.left + layout.width
            && point.1 >= layout.top
            && point.1 < layout.top + layout.height
    })
}

pub fn menu_item_count(menu: ToolbarMenu) -> usize {
    match menu {
        ToolbarMenu::ScrollMode => 2,
        ToolbarMenu::Recognition => 3, // OCR, Image Translate, QR
        ToolbarMenu::RecordingFps => 2,
        ToolbarMenu::RecordingExport => 3, // MP4, GIF, WebP
        ToolbarMenu::DemoBg => 5,          // Transparent, White, Dark, Blackboard Green, Slide Image
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolbarMenuLayout {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub item_height: i32,
    pub header_height: i32,
    pub count: usize,
}

impl ToolbarMenuLayout {
    pub fn height(self) -> i32 {
        self.item_height * self.count as i32 + self.header_height + 8
    }

    pub fn item_rect(self, index: usize) -> Option<(i32, i32, i32, i32)> {
        (index < self.count).then_some((
            self.left + 4,
            self.top + 4 + self.header_height + index as i32 * self.item_height,
            self.left + self.width - 4,
            self.top + 4 + self.header_height + (index + 1) as i32 * self.item_height,
        ))
    }
}

pub fn menu_layout(toolbar_origin: (i32, i32), menu: ToolbarMenu) -> ToolbarMenuLayout {
    let count = menu_item_count(menu);
    let (width, item_height, header_height) = match menu {
        ToolbarMenu::RecordingFps => (276, 42, 52),
        ToolbarMenu::RecordingExport => (220, 36, 50),
        ToolbarMenu::Recognition => (216, 36, 0),
        ToolbarMenu::ScrollMode => (204, 34, 0),
        ToolbarMenu::DemoBg => (220, 34, 0),
    };
    let height = item_height * count as i32 + header_height + 8;
    ToolbarMenuLayout {
        left: toolbar_origin.0,
        top: toolbar_origin.1 - height,
        width,
        item_height,
        header_height,
        count,
    }
}

pub fn menu_hit_test(layout: ToolbarMenuLayout, point: (i32, i32)) -> Option<usize> {
    (point.0 >= layout.left
        && point.0 < layout.left + layout.width
        && point.1 >= layout.top
        && point.1 < layout.top + layout.height())
    .then(|| ((point.1 - layout.top - 4 - layout.header_height) / layout.item_height) as usize)
    .filter(|index| *index < layout.count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_toolbar_builds_from_settings_with_drag_cancel_finish() {
        let settings = Settings::default();
        let actions = build_main_actions(&settings);
        assert_eq!(actions.first(), Some(&ToolbarAction::MoveToolbar));
        assert!(actions.contains(&ToolbarAction::Cancel));
        assert!(actions.contains(&ToolbarAction::Finish));
        assert!(actions.contains(&ToolbarAction::Translate));
    }

    #[test]
    fn info_capsule_radius_slider_hits_range() {
        let cap = InfoCapsuleLayout::for_selection(
            ScreenRect {
                left: 100,
                top: 100,
                right: 600,
                bottom: 400,
            },
            ScreenRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
        );
        let track = cap.slider_track_rect();
        assert_eq!(
            cap.hit_test(track.0, cap.top + 16),
            Some(InfoCapsuleHit::RadiusSlider(0))
        );
        assert_eq!(
            cap.hit_test(track.2, cap.top + 16),
            Some(InfoCapsuleHit::RadiusSlider(60))
        );
    }

    #[test]
    fn secondary_bar_supports_shape_variants() {
        let screen = ScreenRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let sel = ScreenRect {
            left: 100,
            top: 100,
            right: 500,
            bottom: 350,
        };
        let tb = layout_for_item_count(16, 1920);
        let bar = secondary_bar_layout((100, 400), tb, Tool::Rectangle, screen, sel).unwrap();
        assert_eq!(bar.variant_count, 6);
        assert!(bar.has_styles);
        assert!(bar.has_color);
        let (l, t, _, _) = bar.variant_rect(1).unwrap();
        assert_eq!(
            secondary_bar_hit_test(bar, (l + 4, t + 4)),
            Some(SecondaryBarItem::Variant(Tool::Ellipse))
        );
    }
}
