use crate::model::{Annotation, OcrRegionBox, ScreenPoint, ScreenRect, Settings, StrokeStyle};
use crate::scroll::{MatchAxis, ScrollingCapture};
use crate::toolbar::{
    build_main_actions, SecondaryBarItem, Tool, ToolbarAction, ToolbarMenu,
};

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
    pub(super) radius_dragging: bool,
    pub(super) aspect_locked: bool,
    pub(super) corner_radius: u32,
    pub(super) drop_shadow: bool,
    pub(super) toolbar_actions: Vec<ToolbarAction>,
    pub(super) ui_scale: f32,
    pub(super) toolbar_origin: Option<ScreenPoint>,
    pub(super) toolbar_drag: Option<(ScreenPoint, ScreenPoint)>,
    pub(super) toolbar_hover: Option<usize>,
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
    pub(super) shape_filled: bool,
    pub(super) shape_rounded: bool,
    pub(super) watermark_text: String,
    pub(super) annotations: Vec<Annotation>,
    pub(super) redo_annotations: Vec<Annotation>,
    pub(super) editing_text: Option<usize>,
    pub(super) number_next: u32,
    pub(super) scroll: Option<ScrollingCapture>,
    pub(super) scroll_axis: Option<MatchAxis>,
    pub(super) scroll_paused: bool,
    /// In-place OCR bounding boxes displayed over the selection (relative to selection origin).
    pub(super) ocr_blocks: Vec<OcrRegionBox>,
    pub(super) ocr_selected_block: Option<usize>,
    /// In-place translated RGBA image (matches selection width*height*4) when user clicks Translate!
    pub(super) translated_image: Option<(usize, usize, Vec<u8>)>,
    pub(super) show_translated_overlay: bool,
}

impl CaptureSession {
    pub(super) fn new_with_settings(
        screen_rect: ScreenRect,
        settings: &Settings,
        scrolling: bool,
    ) -> Self {
        Self {
            screen_rect,
            selection: None,
            hover: None,
            drag: None,
            selection_move: None,
            selection_resize: None,
            radius_dragging: false,
            aspect_locked: false,
            corner_radius: settings.default_corner_radius,
            drop_shadow: settings.default_shadow,
            toolbar_actions: build_main_actions(settings),
            ui_scale: crate::theme::ui_scale_for_settings(settings),
            toolbar_origin: None,
            toolbar_drag: None,
            toolbar_hover: None,
            secondary_hover: None,
            menu: None,
            menu_hover: None,
            current_pos: None,
            tool: Tool::Select,
            annotation_color: settings.annotation_rgba(),
            stroke_width: settings.stroke_width,
            brush_size: settings.brush_size,
            text_size: settings.text_size,
            stroke_style: StrokeStyle::Solid,
            shape_filled: false,
            shape_rounded: false,
            watermark_text: settings.watermark_text.clone(),
            annotations: Vec::new(),
            redo_annotations: Vec::new(),
            editing_text: None,
            number_next: 1,
            scroll: scrolling.then(ScrollingCapture::pending),
            scroll_axis: None,
            scroll_paused: false,
            ocr_blocks: Vec::new(),
            ocr_selected_block: None,
            translated_image: None,
            show_translated_overlay: false,
        }
    }

    pub(super) fn new(
        screen_rect: ScreenRect,
        annotation_color: [u8; 4],
        text_size: u32,
        scrolling: bool,
    ) -> Self {
        let mut s = Settings::default();
        s.text_size = text_size;
        let mut session = Self::new_with_settings(screen_rect, &s, scrolling);
        session.annotation_color = annotation_color;
        session
    }

    pub(super) fn toolbar_action(&self, index: usize) -> Option<ToolbarAction> {
        self.toolbar_actions.get(index).copied()
    }

    pub(super) fn toolbar_button_count(&self) -> usize {
        self.toolbar_actions.len()
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
        self.toolbar_hover = None;
        self.secondary_hover = None;
        true
    }

    pub(super) fn erase_at(&mut self, point: ScreenPoint) -> bool {
        if let Some(pos) = self
            .annotations
            .iter()
            .rposition(|ann| ann.hit_test(point, 10))
        {
            let removed = self.annotations.remove(pos);
            self.redo_annotations.push(removed);
            true
        } else {
            false
        }
    }

    pub(super) fn undo(&mut self) -> bool {
        let Some(annotation) = self.annotations.pop() else {
            return false;
        };
        if let Annotation::Number(_, number, _) = &annotation {
            if number.saturating_add(1) == self.number_next {
                self.number_next = *number;
            }
        }
        self.redo_annotations.push(annotation);
        self.editing_text = None;
        true
    }

    pub(super) fn redo(&mut self) -> bool {
        let Some(annotation) = self.redo_annotations.pop() else {
            return false;
        };
        if let Annotation::Number(_, number, _) = &annotation {
            self.number_next = self.number_next.max(number.saturating_add(1));
        }
        self.annotations.push(annotation);
        self.editing_text = None;
        true
    }

    pub(super) fn push_annotation(&mut self, annotation: Annotation) {
        self.redo_annotations.clear();
        self.annotations.push(annotation);
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
        self.redo_annotations.clear();
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
            [239, 68, 68, 255],
            18,
            false,
        )
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

    #[test]
    fn eraser_removes_hit_annotation() {
        let mut capture = session();
        capture.annotations.push(Annotation::Number(
            ScreenPoint { x: 20, y: 20 },
            1,
            capture.annotation_color,
        ));
        assert!(capture.erase_at(ScreenPoint { x: 22, y: 21 }));
        assert!(capture.annotations.is_empty());
    }
}
