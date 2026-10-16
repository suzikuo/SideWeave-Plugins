//! Pure toolbar model used by the native screenshot session.
//!
//! Keeping the item order and hit-testing rules here makes the Win32 message
//! loop an adapter instead of the source of truth for screenshot actions.

use crate::model::ScreenRect;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    Select,
    Rectangle,
    Ellipse,
    Arrow,
    Line,
    Pen,
    Highlight,
    Mosaic,
    Text,
    Number,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarAction {
    MoveToolbar,
    Tools,
    Tool(Tool),
    PlaceNumber,
    Pin,
    Scroll,
    Undo,
    Color,
    Size,
    Style,
    Copy,
    Save,
    Cancel,
    Finish,
    Edit,
    Record,
    Ocr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolbarItem {
    pub label: &'static str,
    pub action: ToolbarAction,
}

/// Compact, touch-friendly icon buttons matching the floating reference toolbar.
pub const TOOLBAR_BUTTON_WIDTH: i32 = 46;
pub const TOOLBAR_BUTTON_HEIGHT: i32 = 46;
pub const TOOLBAR_VERTICAL_PADDING: i32 = 5;

pub const TOOL_PALETTE_ACTIONS: [ToolbarAction; 10] = [
    ToolbarAction::Tool(Tool::Select),
    ToolbarAction::Tool(Tool::Rectangle),
    ToolbarAction::Tool(Tool::Ellipse),
    ToolbarAction::Tool(Tool::Arrow),
    ToolbarAction::Tool(Tool::Line),
    ToolbarAction::Tool(Tool::Pen),
    ToolbarAction::Tool(Tool::Highlight),
    ToolbarAction::Tool(Tool::Mosaic),
    ToolbarAction::Tool(Tool::Text),
    ToolbarAction::Tool(Tool::Number),
];

/// Keep the main bar focused on capture and output actions.
pub const TOOLBAR_ITEMS: [ToolbarItem; 11] = [
    ToolbarItem {
        label: "移动工具栏",
        action: ToolbarAction::MoveToolbar,
    },
    ToolbarItem {
        label: "标注工具",
        action: ToolbarAction::Tools,
    },
    ToolbarItem {
        label: "贴图",
        action: ToolbarAction::Pin,
    },
    ToolbarItem {
        label: "长截图",
        action: ToolbarAction::Scroll,
    },
    ToolbarItem {
        label: "撤销",
        action: ToolbarAction::Undo,
    },
    ToolbarItem {
        label: "保存",
        action: ToolbarAction::Save,
    },
    ToolbarItem {
        label: "编辑",
        action: ToolbarAction::Edit,
    },
    ToolbarItem {
        label: "\u{5f55}\u{5236}",
        action: ToolbarAction::Record,
    },
    ToolbarItem {
        label: "识别",
        action: ToolbarAction::Ocr,
    },
    ToolbarItem {
        label: "取消",
        action: ToolbarAction::Cancel,
    },
    ToolbarItem {
        label: "完成",
        action: ToolbarAction::Finish,
    },
];

pub const TOOLBAR_BUTTON_COUNT: usize = TOOLBAR_ITEMS.len();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolbarLayout {
    pub item_count: usize,
    pub columns: usize,
    pub rows: usize,
    pub width: i32,
    pub height: i32,
}

pub fn layout_for_screen_width(screen_width: i32) -> ToolbarLayout {
    layout_for_item_count(TOOLBAR_BUTTON_COUNT, screen_width)
}

pub fn tool_palette_layout_for_screen_width(screen_width: i32) -> ToolbarLayout {
    layout_for_item_count(TOOL_PALETTE_ACTIONS.len(), screen_width)
}

fn layout_for_item_count(item_count: usize, screen_width: i32) -> ToolbarLayout {
    let single_row_width = item_count as i32 * TOOLBAR_BUTTON_WIDTH + 16;
    let columns = if screen_width < single_row_width {
        let available_columns = ((screen_width - 16) / TOOLBAR_BUTTON_WIDTH)
            .clamp(1, item_count as i32) as usize;
        item_count.div_ceil(2).min(available_columns)
    } else {
        item_count
    };
    let rows = item_count.div_ceil(columns);
    ToolbarLayout {
        item_count,
        columns,
        rows,
        width: columns as i32 * TOOLBAR_BUTTON_WIDTH,
        height: rows as i32 * TOOLBAR_BUTTON_HEIGHT + TOOLBAR_VERTICAL_PADDING * 2,
    }
}

pub fn tool_palette_position(
    toolbar_origin: (i32, i32),
    toolbar: ToolbarLayout,
    palette: ToolbarLayout,
    screen: ScreenRect,
    selection: ScreenRect,
) -> (i32, i32) {
    let left = toolbar_origin.0.clamp(
        screen.left + 8,
        (screen.right - palette.width - 8).max(screen.left + 8),
    );
    let toolbar_rect = ScreenRect {
        left: toolbar_origin.0,
        top: toolbar_origin.1,
        right: toolbar_origin.0 + toolbar.width,
        bottom: toolbar_origin.1 + toolbar.height,
    };
    let candidates = [
        (left, toolbar_rect.bottom + 8),
        (left, toolbar_rect.top - palette.height - 8),
        (left, selection.bottom + 8),
        (left, selection.top - palette.height - 8),
    ];
    for (candidate_x, candidate_y) in candidates {
        let x = candidate_x.clamp(
            screen.left + 8,
            (screen.right - palette.width - 8).max(screen.left + 8),
        );
        let y = candidate_y.clamp(
            screen.top + 8,
            (screen.bottom - palette.height - 8).max(screen.top + 8),
        );
        let candidate = ScreenRect {
            left: x,
            top: y,
            right: x + palette.width,
            bottom: y + palette.height,
        };
        if !rects_intersect(candidate, toolbar_rect) && !rects_intersect(candidate, selection) {
            return (x, y);
        }
    }
    (
        left,
        (toolbar_rect.bottom + 8).clamp(
            screen.top + 8,
            (screen.bottom - palette.height - 8).max(screen.top + 8),
        ),
    )
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
        origin.0 + column as i32 * TOOLBAR_BUTTON_WIDTH,
        origin.1 + TOOLBAR_VERTICAL_PADDING + row as i32 * TOOLBAR_BUTTON_HEIGHT,
    ))
}

pub fn toolbar_item(index: usize) -> Option<&'static ToolbarItem> {
    TOOLBAR_ITEMS.get(index)
}

pub fn toolbar_action(index: usize) -> Option<ToolbarAction> {
    toolbar_item(index).map(|item| item.action)
}

pub fn tool_palette_action(index: usize) -> Option<ToolbarAction> {
    TOOL_PALETTE_ACTIONS.get(index).copied()
}

pub fn tool_palette_label(index: usize) -> Option<&'static str> {
    match tool_palette_action(index)? {
        ToolbarAction::Tool(Tool::Select) => Some("选择"),
        ToolbarAction::Tool(Tool::Rectangle) => Some("矩形"),
        ToolbarAction::Tool(Tool::Ellipse) => Some("椭圆"),
        ToolbarAction::Tool(Tool::Arrow) => Some("箭头"),
        ToolbarAction::Tool(Tool::Line) => Some("直线"),
        ToolbarAction::Tool(Tool::Pen) => Some("画笔"),
        ToolbarAction::Tool(Tool::Highlight) => Some("高亮"),
        ToolbarAction::Tool(Tool::Mosaic) => Some("马赛克"),
        ToolbarAction::Tool(Tool::Text) => Some("文字"),
        ToolbarAction::Tool(Tool::Number) => Some("序号"),
        _ => None,
    }
}

/// Stable drawing identity for each action, independent of its toolbar position.
pub fn toolbar_icon_index(action: ToolbarAction) -> Option<usize> {
    match action {
        ToolbarAction::MoveToolbar => Some(0),
        ToolbarAction::Tools => Some(19),
        ToolbarAction::Tool(Tool::Select) => Some(20),
        ToolbarAction::Tool(Tool::Rectangle) => Some(1),
        ToolbarAction::Tool(Tool::Ellipse) => Some(2),
        ToolbarAction::Tool(Tool::Arrow) => Some(3),
        ToolbarAction::Tool(Tool::Pen) => Some(4),
        ToolbarAction::Tool(Tool::Mosaic) => Some(5),
        ToolbarAction::Tool(Tool::Text) => Some(6),
        ToolbarAction::Tool(Tool::Number) => Some(7),
        ToolbarAction::Tool(Tool::Line) => Some(17),
        ToolbarAction::Tool(Tool::Highlight) => Some(18),
        ToolbarAction::Pin => Some(8),
        ToolbarAction::Scroll => Some(9),
        ToolbarAction::Undo => Some(10),
        ToolbarAction::Save => Some(11),
        ToolbarAction::Cancel => Some(12),
        ToolbarAction::Finish => Some(13),
        ToolbarAction::Edit => Some(14),
        ToolbarAction::Record => Some(15),
        ToolbarAction::Ocr => Some(16),
        ToolbarAction::Copy => Some(100),
        ToolbarAction::PlaceNumber | ToolbarAction::Color
        | ToolbarAction::Size
        | ToolbarAction::Style => None,
    }
}

pub fn toolbar_label(index: usize) -> Option<&'static str> {
    toolbar_item(index).map(|item| item.label)
}

pub fn hit_test(origin: (i32, i32), point: (i32, i32), layout: ToolbarLayout) -> Option<usize> {
    let (x, y) = origin;
    let (point_x, point_y) = point;
    if point_x < x
        || point_x >= x + layout.width
        || point_y < y + TOOLBAR_VERTICAL_PADDING
        || point_y >= y + layout.height - TOOLBAR_VERTICAL_PADDING
    {
        return None;
    }
    let row = ((point_y - y - TOOLBAR_VERTICAL_PADDING) / TOOLBAR_BUTTON_HEIGHT) as usize;
    if row >= layout.rows {
        return None;
    }
    let column = ((point_x - x) / TOOLBAR_BUTTON_WIDTH) as usize;
    let row_start = row * layout.columns;
    let item_count = layout.item_count;
    let row_item_count = if row + 1 == layout.rows {
        item_count - row_start
    } else {
        layout.columns
    };
    let right_alignment = layout.columns - row_item_count;
    if column < right_alignment {
        return None;
    }
    let index = row_start + column - right_alignment;
    (index < item_count).then_some(index)
}

// -------------------------------------------------------------------------
// 图 2：二级属性栏（Secondary Property Bar）模型
// -------------------------------------------------------------------------

/// 飞书常用色板（红、橙、绿、蓝、紫、黑、白 7 色）
pub const COLOR_SWATCHES: [[u8; 4]; 7] = [
    [245, 74, 69, 255],   // 飞书红 #F54A45
    [250, 140, 22, 255],  // 暖阳橙 #FA8C16
    [0, 180, 42, 255],    // 翠羽绿 #00B42A
    [22, 111, 247, 255],  // 飞书蓝 #166FF7
    [114, 46, 209, 255],  // 典雅紫 #722ED1
    [31, 35, 41, 255],    // 极客黑 #1F2329
    [255, 255, 255, 255], // 纯白 #FFFFFF
];

/// 4 档笔触尺寸 (细、中、粗、特粗)
pub const STROKE_SIZES: [u32; 4] = [2, 4, 6, 10];
pub const TEXT_SIZES: [u32; 4] = [14, 18, 24, 32];
pub const MOSAIC_SIZES: [u32; 4] = [12, 24, 40, 64];

pub fn tool_has_secondary_bar(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Rectangle
            | Tool::Ellipse
            | Tool::Arrow
            | Tool::Line
            | Tool::Pen
            | Tool::Highlight
            | Tool::Mosaic
            | Tool::Text
            | Tool::Number
    )
}

pub fn tool_has_color_picker(tool: Tool) -> bool {
    tool != Tool::Mosaic
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecondaryBarLayout {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub has_size: bool,
    pub has_color: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecondaryBarItem {
    Size(usize),
    Color(usize),
}

impl SecondaryBarLayout {
    pub fn size_rect(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        if index >= 4 {
            return None;
        }
        let item_w = 28;
        let left = self.left + 8 + index as i32 * item_w;
        Some((left, self.top + 4, left + item_w, self.top + self.height - 4))
    }

    pub fn separator_x(&self) -> Option<i32> {
        if self.has_color {
            Some(self.left + 8 + 4 * 28 + 6)
        } else {
            None
        }
    }

    pub fn color_rect(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        if !self.has_color || index >= COLOR_SWATCHES.len() {
            return None;
        }
        let start_x = self.left + 8 + 4 * 28 + 14;
        let item_w = 22;
        let gap = 6;
        let left = start_x + index as i32 * (item_w + gap);
        let top = self.top + (self.height - item_w) / 2;
        Some((left, top, left + item_w, top + item_w))
    }
}

pub fn secondary_bar_layout(
    toolbar_origin: (i32, i32),
    tool: Tool,
    screen_rect: ScreenRect,
    selection: ScreenRect,
) -> Option<SecondaryBarLayout> {
    if !tool_has_secondary_bar(tool) {
        return None;
    }
    let has_color = tool_has_color_picker(tool);
    // 8 + 4*28 + 14 + 7*22 + 6*6 + 8 = 8 + 112 + 14 + 154 + 36 + 8 = 332
    let width = (if has_color { 332 } else { 128 })
        .min((screen_rect.width() as i32 - 16).max(128));
    let height = 40;
    let (tx, ty) = toolbar_origin;
    let left = tx.clamp(
        screen_rect.left + 8,
        (screen_rect.right - width - 8).max(screen_rect.left + 8),
    );
    let toolbar_layout = layout_for_screen_width(screen_rect.width() as i32);
    let toolbar = ScreenRect {
        left: tx,
        top: ty,
        right: tx + toolbar_layout.width,
        bottom: ty + toolbar_layout.height,
    };
    let toolbar = if toolbar.right > screen_rect.right {
        ScreenRect {
            right: (screen_rect.right - 8).max(toolbar.left),
            ..toolbar
        }
    } else {
        toolbar
    };
    let selection_center = (selection.top + selection.bottom) / 2;
    let candidates = [
        (left, toolbar.bottom + 8),
        (left, toolbar.top - height - 8),
        (left, selection.bottom + 8),
        (left, selection.top - height - 8),
        (selection.left - width - 8, selection_center - height / 2),
        (selection.right + 8, selection_center - height / 2),
    ];
    let mut top = None;
    for (candidate_x, candidate_y) in candidates {
        let x = candidate_x.clamp(
            screen_rect.left + 8,
            (screen_rect.right - width - 8).max(screen_rect.left + 8),
        );
        let y = candidate_y.clamp(
            screen_rect.top + 8,
            (screen_rect.bottom - height - 8).max(screen_rect.top + 8),
        );
        let candidate = ScreenRect {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        };
        if !rects_intersect(candidate, selection) && !rects_intersect(candidate, toolbar) {
            top = Some((x, y));
            break;
        }
    }
    let (left, top) = top.unwrap_or((left, (toolbar.bottom + 8).clamp(
        screen_rect.top + 8,
        (screen_rect.bottom - height - 8).max(screen_rect.top + 8),
    )));
    Some(SecondaryBarLayout {
        left,
        top,
        width,
        height,
        has_size: true,
        has_color,
    })
}

fn rects_intersect(a: ScreenRect, b: ScreenRect) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
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
    for i in 0..4 {
        if let Some((l, t, r, b)) = layout.size_rect(i) {
            if px >= l && px < r && py >= t && py < b {
                return Some(SecondaryBarItem::Size(i));
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

// -------------------------------------------------------------------------
// 保留旧 ToolbarMenu 供滚动长截模式（ScrollMode）使用
// -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarMenu {
    Color,
    Size,
    Style,
    ScrollMode,
    Recognition,
    RecordingFps,
    RecordingExport,
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
    Format(AnimationFormatKey),
    ToggleCursor,
    Ignore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationFormatKey {
    Mp4,
    Gif,
    Webp,
}

impl RecordingSetupLayout {
    pub fn close_rect(self) -> (i32, i32, i32, i32) {
        (self.left + self.width - 34, self.top + 3, self.left + self.width - 10, self.top + 18)
    }

    pub fn start_rect(self) -> (i32, i32, i32, i32) {
        (self.left + 24, self.top + 20, self.left + self.width - 24, self.top + 68)
    }

    pub fn format_rect(self, index: usize) -> Option<(i32, i32, i32, i32)> {
        let (left, right) = match index {
            0 => (self.left + 138, self.left + 224),
            1 => (self.left + 252, self.left + 326),
            2 => (self.left + 336, self.left + 426),
            _ => return None,
        };
        Some((left, self.top + 88, right, self.top + 126))
    }

    pub fn fps_rect(self, index: usize) -> Option<(i32, i32, i32, i32)> {
        (index < 2).then_some((
            self.left + 20 + index as i32 * 200,
            self.top + 160,
            self.left + 210 + index as i32 * 200,
            self.top + 198,
        ))
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
    RecordingSetupLayout { left, top, width, height }
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
    for index in 0..3 {
        if layout.format_rect(index).is_some_and(inside) {
            return Some(RecordingSetupAction::Format(match index {
                0 => AnimationFormatKey::Mp4,
                1 => AnimationFormatKey::Gif,
                _ => AnimationFormatKey::Webp,
            }));
        }
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

pub fn menu_for_action(action: ToolbarAction) -> Option<ToolbarMenu> {
    match action {
        ToolbarAction::Color => Some(ToolbarMenu::Color),
        ToolbarAction::Size => Some(ToolbarMenu::Size),
        ToolbarAction::Style => Some(ToolbarMenu::Style),
        ToolbarAction::Record => Some(ToolbarMenu::RecordingFps),
        ToolbarAction::Ocr => Some(ToolbarMenu::Recognition),
        _ => None,
    }
}

pub fn menu_item_count(menu: ToolbarMenu, tool: Tool) -> usize {
    match menu {
        ToolbarMenu::Color => COLOR_SWATCHES.len(),
        ToolbarMenu::Size => {
            if tool == Tool::Text {
                TEXT_SIZES.len()
            } else if tool == Tool::Mosaic {
                MOSAIC_SIZES.len()
            } else {
                STROKE_SIZES.len()
            }
        }
        ToolbarMenu::Style => 3,
        ToolbarMenu::ScrollMode => 2,
        ToolbarMenu::Recognition => 2,
        ToolbarMenu::RecordingFps => 2,
        ToolbarMenu::RecordingExport => 3,
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

pub fn menu_layout(
    toolbar_origin: (i32, i32),
    _toolbar_layout: ToolbarLayout,
    menu: ToolbarMenu,
    tool: Tool,
) -> ToolbarMenuLayout {
    let count = menu_item_count(menu, tool);
    let (width, item_height, header_height) = match menu {
        ToolbarMenu::RecordingFps => (276, 42, 52),
        ToolbarMenu::RecordingExport => (216, 36, 52),
        ToolbarMenu::Recognition => (216, 38, 0),
        ToolbarMenu::ScrollMode => (204, 32, 0),
        ToolbarMenu::Color => (224, 32, 0),
        ToolbarMenu::Size | ToolbarMenu::Style => (176, 32, 0),
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
    fn toolbar_order_has_explicit_capture_actions() {
        assert_eq!(TOOLBAR_ITEMS.len(), 11);
        assert_eq!(toolbar_action(0), Some(ToolbarAction::MoveToolbar));
        assert_eq!(toolbar_action(1), Some(ToolbarAction::Tools));
        assert_eq!(toolbar_action(2), Some(ToolbarAction::Pin));
        assert_eq!(toolbar_action(3), Some(ToolbarAction::Scroll));
        assert_eq!(toolbar_action(4), Some(ToolbarAction::Undo));
        assert_eq!(toolbar_action(5), Some(ToolbarAction::Save));
        assert_eq!(toolbar_action(6), Some(ToolbarAction::Edit));
        assert_eq!(toolbar_action(7), Some(ToolbarAction::Record));
        assert_eq!(toolbar_action(8), Some(ToolbarAction::Ocr));
        assert_eq!(toolbar_action(9), Some(ToolbarAction::Cancel));
        assert_eq!(toolbar_action(10), Some(ToolbarAction::Finish));
    }

    #[test]
    fn hit_test_rejects_edges_outside_toolbar() {
        let wide = layout_for_screen_width(1920);
        assert_eq!(hit_test((100, 200), (100, 208), wide), Some(0));
        assert_eq!(
            hit_test((100, 200), (100 + TOOLBAR_BUTTON_WIDTH * 10, 208), wide),
            Some(10)
        );
        assert_eq!(
            hit_test((100, 200), (100 + TOOLBAR_BUTTON_WIDTH * 11, 208), wide),
            None
        );
        assert_eq!(hit_test((100, 200), (100, 200 + wide.height), wide), None);
    }

    #[test]
    fn secondary_bar_model_works() {
        let screen = ScreenRect { left: 0, top: 0, right: 1920, bottom: 1080 };
        let selection = ScreenRect { left: 100, top: 100, right: 500, bottom: 350 };
        let bar = secondary_bar_layout((100, 400), Tool::Pen, screen, selection).unwrap();
        assert!(bar.has_size);
        assert!(bar.has_color);
        assert_eq!(bar.top, 400 - 40 - 8);

        // hit test size 0
        let (s0_l, s0_t, _, _) = bar.size_rect(0).unwrap();
        assert_eq!(secondary_bar_hit_test(bar, (s0_l + 2, s0_t + 2)), Some(SecondaryBarItem::Size(0)));

        // hit test color 0
        let (c0_l, c0_t, _, _) = bar.color_rect(0).unwrap();
        assert_eq!(secondary_bar_hit_test(bar, (c0_l + 2, c0_t + 2)), Some(SecondaryBarItem::Color(0)));
    }
}
