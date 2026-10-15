use crate::model::{Annotation, ScreenPoint, ScreenRect, StrokeStyle};
use crate::scroll::{MatchAxis, ScrollingCapture};
use crate::toolbar::{SecondaryBarItem, Tool, ToolbarMenu};

#[derive(Clone, Copy)]
pub(super) struct ResizeEdges {
    pub(super) left: bool,
    pub(super) top: bool,
    pub(super) right: bool,
    pub(super) bottom: bool,
}

pub(super) struct CaptureSession {
    pub(super) screen_rect: ScreenRect,
    pub(super) selection: Option<ScreenRect>,
    pub(super) hover: Option<ScreenRect>,
    pub(super) drag: Option<ScreenPoint>,
    pub(super) selection_move: Option<(ScreenPoint, ScreenRect)>,
    pub(super) selection_resize: Option<(ScreenPoint, ScreenRect, ResizeEdges)>,
    pub(super) toolbar_origin: Option<ScreenPoint>,
    pub(super) toolbar_drag: Option<(ScreenPoint, ScreenPoint)>,
    pub(super) toolbar_hover: Option<usize>,
    pub(super) tool_palette_open: bool,
    pub(super) tool_palette_hover: Option<usize>,
    pub(super) secondary_hover: Option<SecondaryBarItem>,
    pub(super) menu: Option<ToolbarMenu>,
    pub(super) menu_hover: Option<usize>,
    pub(super) current_pos: Option<ScreenPoint>,
    pub(super) tool: Tool,
    pub(super) annotation_color: [u8; 4],
    pub(super) stroke_width: u32,
    pub(super) brush_size: u32,
    pub(super) text_size: u32,
    pub(super) stroke_style: StrokeStyle,
    pub(super) annotations: Vec<Annotation>,
    pub(super) editing_text: Option<usize>,
    pub(super) number_next: u32,
    pub(super) scroll: Option<ScrollingCapture>,
    pub(super) scroll_axis: Option<MatchAxis>,
}

impl CaptureSession {
    pub(super) fn new(
        screen_rect: ScreenRect,
        annotation_color: [u8; 4],
        text_size: u32,
        scrolling: bool,
    ) -> Self {
        Self {
            screen_rect,
            selection: None,
            hover: None,
            drag: None,
            selection_move: None,
            selection_resize: None,
            toolbar_origin: None,
            toolbar_drag: None,
            toolbar_hover: None,
            tool_palette_open: false,
            tool_palette_hover: None,
            secondary_hover: None,
            menu: None,
            menu_hover: None,
            current_pos: None,
            tool: Tool::Select,
            annotation_color,
            stroke_width: 4,
            brush_size: 24,
            text_size,
            stroke_style: StrokeStyle::Solid,
            annotations: Vec::new(),
            editing_text: None,
            number_next: 1,
            scroll: scrolling.then(ScrollingCapture::pending),
            scroll_axis: None,
        }
    }

    #[cfg(test)]
    pub(super) fn reset_selection(&mut self) {
        self.selection = None;
        self.hover = None;
        self.drag = None;
        self.selection_move = None;
        self.selection_resize = None;
        self.toolbar_origin = None;
        self.toolbar_drag = None;
        self.toolbar_hover = None;
        self.tool_palette_open = false;
        self.tool_palette_hover = None;
        self.secondary_hover = None;
        self.current_pos = None;
        self.annotations.clear();
        self.editing_text = None;
        self.number_next = 1;
        self.tool = Tool::Select;
        self.scroll_axis = None;
    }

    pub(super) fn open_menu(&mut self, menu: ToolbarMenu) {
        self.menu = Some(menu);
        self.menu_hover = None;
    }

    pub(super) fn close_menu(&mut self) {
        self.menu = None;
        self.menu_hover = None;
    }
    pub(super) fn select_tool(&mut self, tool: Tool) -> bool {
        if self.selection.is_none() {
            return false;
        }
        self.tool = tool;
        self.editing_text = None;
        self.tool_palette_open = false;
        self.tool_palette_hover = None;
        self.toolbar_hover = None;
        self.secondary_hover = None;
        true
    }

    pub(super) fn undo(&mut self) -> bool {
        let Some(annotation) = self.annotations.pop() else {
            return false;
        };
        if let Annotation::Number(_, number, _) = annotation {
            if number.saturating_add(1) == self.number_next {
                self.number_next = number;
            }
        }
        self.editing_text = None;
        true
    }

    pub(super) fn finish_text_editing(&mut self) -> bool {
        let changed = self.editing_text.is_some();
        self.editing_text = None;
        changed
    }

    pub(super) fn append_text_char(&mut self, value: u16) -> bool {
        let Some(index) = self.editing_text else {
            return false;
        };
        if value == 8 {
            if let Some(Annotation::Text(_, text, _, _)) = self.annotations.get_mut(index) {
                text.pop();
            }
        } else if value == 13 {
            self.editing_text = None;
        } else if let Some(character) = char::from_u32(value as u32).filter(|c| !c.is_control()) {
            if let Some(Annotation::Text(_, text, _, _)) = self.annotations.get_mut(index) {
                text.push(character);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> CaptureSession {
        CaptureSession::new(
            ScreenRect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100,
            },
            [255, 58, 72, 255],
            18,
            false,
        )
    }

    #[test]
    fn reset_clears_annotations_and_restores_selection_tool() {
        let mut capture = session();
        capture.selection = Some(ScreenRect {
            left: 1,
            top: 1,
            right: 10,
            bottom: 10,
        });
        capture.tool = Tool::Rectangle;
        capture.annotations.push(Annotation::Number(
            ScreenPoint { x: 2, y: 2 },
            1,
            capture.annotation_color,
        ));
        capture.reset_selection();
        assert!(capture.selection.is_none());
        assert!(capture.annotations.is_empty());
        assert_eq!(capture.tool, Tool::Select);
        assert_eq!(capture.number_next, 1);
    }

    #[test]
    fn undo_rewinds_number_sequence() {
        let mut capture = session();
        capture.annotations.push(Annotation::Number(
            ScreenPoint { x: 2, y: 2 },
            1,
            capture.annotation_color,
        ));
        capture.number_next = 2;
        assert!(capture.undo());
        assert_eq!(capture.number_next, 1);
        assert!(!capture.undo());
    }
}
