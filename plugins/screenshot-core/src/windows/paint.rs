use super::draw::{
    draw_annotations_gdi, draw_banner, draw_info_capsule_gdi, draw_ocr_blocks_overlay,
    draw_preview, draw_rect_border, draw_selection_handles_gdi,
};
use super::input::{
    info_capsule_for, secondary_bar_layout_for, toolbar_layout_for, toolbar_menu_layout_for,
    toolbar_origin,
};
use super::toolbar_paint::{
    argb, create_ui_font, draw_recording_setup_gdi, draw_round_rect_gdip, draw_secondary_bar_gdi,
    draw_secondary_bar_tooltip, draw_soft_shadow_gdip, draw_toolbar_gdi, draw_toolbar_menu_gdi,
    draw_toolbar_tooltip, ensure_gdiplus,
};
use super::{wide, UiState};
use crate::capture::ScreenCapture;
use crate::model::ScreenRect;
use crate::theme::ThemePalette;

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{copy_nonoverlapping, null_mut};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GetDC, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HDC, HGDIOBJ, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    FillModeAlternate, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill,
    GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDrawEllipse,
    GdipFillEllipse, GdipFillPath, GdipAddPathLine, GdipClosePathFigure, GdipSetPixelOffsetMode,
    GdipSetSmoothingMode, GpBrush, GpGraphics, GpPath, GpPen, GpSolidFill, PixelOffsetModeHalf,
    SmoothingModeAntiAlias8x8, UnitPixel,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

/// SnowAir signature green `#34C759` in ARGB
const PLAYER_ACCENT_ARGB: u32 = 0xFF_34C759;
const PLAYER_ACCENT_HOVER_ARGB: u32 = 0xFF_2DB84F;

pub(super) struct GdiSurface {
    pub(super) dc: HDC,
    pub(super) bitmap: HGDIOBJ,
    pub(super) old_bitmap: HGDIOBJ,
}

impl Drop for GdiSurface {
    fn drop(&mut self) {
        unsafe {
            if !self.dc.is_null() {
                if !self.old_bitmap.is_null() {
                    SelectObject(self.dc, self.old_bitmap);
                }
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
                DeleteDC(self.dc);
            }
        }
    }
}

pub(crate) struct OverlayRender {
    pub(super) original: GdiSurface,
    pub(super) dimmed: GdiSurface,
    pub(super) output: GdiSurface,
}

pub(super) unsafe fn create_gdi_surface(
    reference: HDC,
    width: i32,
    height: i32,
) -> Option<GdiSurface> {
    let dc = CreateCompatibleDC(reference);
    if dc.is_null() {
        return None;
    }
    let bitmap = CreateCompatibleBitmap(reference, width, height);
    if bitmap.is_null() {
        DeleteDC(dc);
        return None;
    }
    let bitmap = bitmap as HGDIOBJ;
    let old_bitmap = SelectObject(dc, bitmap);
    Some(GdiSurface {
        dc,
        bitmap,
        old_bitmap,
    })
}

pub(super) unsafe fn create_pixel_surface(
    reference: HDC,
    width: i32,
    height: i32,
    pixels: &[u8],
) -> Option<GdiSurface> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|value| value.checked_mul(4))?;
    if pixels.len() < expected {
        return None;
    }
    let info = bitmap_info(width, -height);
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(reference, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        if !bitmap.is_null() {
            DeleteObject(bitmap as HGDIOBJ);
        }
        return None;
    }
    let bitmap = bitmap as HGDIOBJ;
    let dc = CreateCompatibleDC(reference);
    if dc.is_null() {
        DeleteObject(bitmap);
        return None;
    }
    let old_bitmap = SelectObject(dc, bitmap);
    copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, expected);
    Some(GdiSurface {
        dc,
        bitmap,
        old_bitmap,
    })
}

pub(super) unsafe fn create_overlay_render(
    hwnd: HWND,
    screen: &ScreenCapture,
) -> Option<OverlayRender> {
    let reference = GetDC(hwnd);
    if reference.is_null() {
        return None;
    }
    let original = create_pixel_surface(reference, screen.width, screen.height, &screen.pixels);
    let dimmed = create_pixel_surface(
        reference,
        screen.width,
        screen.height,
        screen.dimmed_pixels(),
    );
    let output = create_gdi_surface(reference, screen.width, screen.height);
    let result = match (original, dimmed, output) {
        (Some(original), Some(dimmed), Some(output)) => Some(OverlayRender {
            original,
            dimmed,
            output,
        }),
        _ => None,
    };
    ReleaseDC(hwnd, reference);
    result
}

pub(super) fn paint(hwnd: HWND, state: &mut UiState) {
    unsafe {
        state.paint_pending = false;
        let mut paint_struct: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut paint_struct);
        let theme = state.current_theme();
        let sel_color = ThemePalette::colorref(theme.selection);

        if let (Some(screen), Some(capture)) = (state.screen.as_ref(), state.capture.as_ref()) {
            let active_rect = capture.selection.or_else(|| {
                let start = capture.drag?;
                let current = capture.current_pos?;
                let rect = ScreenRect::normalized(start, current);
                (rect.width() > 2 && rect.height() > 2).then_some(rect)
            });
            let render = state.render.as_ref();
            let render_dc = render.map(|value| value.output.dc).unwrap_or(dc);

            if let Some(render) = render {
                let source = if active_rect.is_some() {
                    render.dimmed.dc
                } else {
                    render.original.dc
                };
                BitBlt(
                    render.output.dc,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    source,
                    0,
                    0,
                    SRCCOPY,
                );
                if let Some(rect) = active_rect {
                    let rx = rect.left - screen.x;
                    let ry = rect.top - screen.y;
                    BitBlt(
                        render.output.dc,
                        rx,
                        ry,
                        rect.width() as i32,
                        rect.height() as i32,
                        render.original.dc,
                        rx,
                        ry,
                        SRCCOPY,
                    );
                }
            } else {
                draw_pixels(
                    render_dc,
                    if active_rect.is_some() {
                        screen.dimmed_pixels()
                    } else {
                        &screen.pixels
                    },
                    screen.width,
                    screen.height,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    0,
                    0,
                    screen.width,
                    screen.height,
                );
                if let Some(rect) = active_rect {
                    let rx = rect.left - screen.x;
                    let ry = rect.top - screen.y;
                    draw_pixels(
                        render_dc,
                        &screen.pixels,
                        screen.width,
                        screen.height,
                        rx,
                        ry,
                        rect.width() as i32,
                        rect.height() as i32,
                        rx,
                        ry,
                        rect.width() as i32,
                        rect.height() as i32,
                    );
                }
            }

            if let Some(rect) = active_rect {
                let rx = rect.left - screen.x;
                let ry = rect.top - screen.y;

                // In-place translated image overlay (SnowAir Image 8)
                if capture.show_translated_overlay {
                    if let Some((tw, th, tpixels)) = capture.translated_image.as_ref() {
                        draw_rgba_preview(
                            render_dc,
                            tpixels,
                            *tw as i32,
                            *th as i32,
                            rx,
                            ry,
                            rect.width() as i32,
                            rect.height() as i32,
                        );
                    }
                } else if !capture.ocr_blocks.is_empty() {
                    // In-place OCR selectable bounding boxes (SnowAir Image 7)
                    draw_ocr_blocks_overlay(
                        render_dc,
                        rect,
                        &capture.ocr_blocks,
                        capture.ocr_selected_block,
                        screen.x,
                        screen.y,
                    );
                }

                // SnowAir crisp green selection border
                draw_rect_border(
                    render_dc,
                    rx,
                    ry,
                    rect.width() as i32,
                    rect.height() as i32,
                    sel_color,
                    2,
                );

                if capture.selection.is_some() && !state.recording_overlay {
                    draw_selection_handles_gdi(
                        render_dc,
                        rx,
                        ry,
                        rect.width() as i32,
                        rect.height() as i32,
                        &theme,
                    );
                    if let Some(capsule) = info_capsule_for(capture) {
                        draw_info_capsule_gdi(
                            render_dc,
                            capsule,
                            rect,
                            capture.aspect_locked,
                            capture.corner_radius,
                            capture.drop_shadow,
                            &theme,
                            screen.x,
                            screen.y,
                        );
                    }
                }

                if capture.selection.is_some() && !state.recording_overlay {
                    draw_annotations_gdi(render_dc, capture, screen, screen.x, screen.y);
                    if let (Some(start), Some(current)) = (capture.drag, capture.current_pos) {
                        draw_preview(render_dc, screen, capture, start, current, screen.x, screen.y);
                    }
                    let (toolbar_x, toolbar_y) = toolbar_origin(capture);
                    let tb_layout = toolbar_layout_for(capture);
                    let can_undo = !capture.annotations.is_empty();
                    let can_redo = !capture.redo_annotations.is_empty();
                    let recording_setup_open =
                        capture.menu == Some(crate::toolbar::ToolbarMenu::RecordingFps);

                    if !recording_setup_open {
                        draw_toolbar_gdi(
                            render_dc,
                            toolbar_x - screen.x,
                            toolbar_y - screen.y,
                            tb_layout,
                            &capture.toolbar_actions,
                            capture.tool,
                            capture.toolbar_hover,
                            can_undo,
                            can_redo,
                            false,
                            false,
                            &theme,
                        );
                    }

                    if !recording_setup_open {
                        if let Some(mut sec_layout) = secondary_bar_layout_for(capture) {
                            sec_layout.left -= screen.x;
                            sec_layout.top -= screen.y;
                            draw_secondary_bar_gdi(
                                render_dc,
                                sec_layout,
                                capture.tool,
                                capture.secondary_hover,
                                capture.annotation_color,
                                capture.stroke_width,
                                capture.brush_size,
                                capture.text_size,
                                capture.stroke_style,
                                capture.shape_filled,
                                capture.shape_rounded,
                                &theme,
                            );
                            if let Some(sec_hover) = capture.secondary_hover {
                                draw_secondary_bar_tooltip(
                                    render_dc,
                                    sec_layout,
                                    screen.width,
                                    screen.height,
                                    sec_hover,
                                    capture.tool,
                                    &theme,
                                );
                            }
                        }
                    }

                    if let Some(menu) = capture
                        .menu
                        .filter(|menu| *menu != crate::toolbar::ToolbarMenu::RecordingFps)
                    {
                        let mut menu_layout = toolbar_menu_layout_for(capture, menu);
                        menu_layout.left -= screen.x;
                        menu_layout.top -= screen.y;
                        draw_toolbar_menu_gdi(
                            render_dc,
                            menu_layout,
                            menu,
                            capture.scroll_axis,
                            capture.menu_hover,
                            &theme,
                        );
                    }

                    if recording_setup_open {
                        let setup_layout = crate::toolbar::recording_setup_layout(
                            capture.selection.unwrap(),
                            capture.screen_rect,
                        );
                        draw_recording_setup_gdi(
                            render_dc,
                            crate::toolbar::RecordingSetupLayout {
                                left: setup_layout.left - screen.x,
                                top: setup_layout.top - screen.y,
                                ..setup_layout
                            },
                            state.recording_show_cursor,
                            &theme,
                        );
                    }

                    if !recording_setup_open {
                        if let Some(index) = capture.toolbar_hover {
                            if let Some(act) = capture.toolbar_action(index) {
                                draw_toolbar_tooltip(
                                    render_dc,
                                    toolbar_x - screen.x,
                                    toolbar_y - screen.y,
                                    tb_layout,
                                    screen.width,
                                    screen.height,
                                    act.label(),
                                    index,
                                    &theme,
                                );
                            }
                        }
                    }
                }
            } else if let Some(hover) = capture.hover {
                draw_rect_border(
                    render_dc,
                    hover.left - screen.x,
                    hover.top - screen.y,
                    hover.width() as i32,
                    hover.height() as i32,
                    sel_color,
                    2,
                );
                draw_banner(render_dc, 18, 18, "点击选窗 · 拖拽框选 · Esc 取消", &theme);
            } else {
                draw_banner(render_dc, 18, 18, "拖拽框选区域 · Esc 取消", &theme);
            }

            if let Some(render) = render {
                BitBlt(
                    dc,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    render.output.dc,
                    0,
                    0,
                    SRCCOPY,
                );
            }
        }
        EndPaint(hwnd, &paint_struct);
    }
}

pub(super) fn paint_editor(hwnd: HWND, state: &mut UiState) {
    unsafe {
        state.paint_pending = false;
        let mut paint_struct: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut paint_struct);
        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);
        let theme = state.current_theme();

        let canvas_dc = state
            .editor
            .as_ref()
            .and_then(|editor| editor.canvas_surface.as_ref())
            .map(|surface| surface.dc);
        let buffer_dc = state
            .editor
            .as_ref()
            .and_then(|editor| editor.back_buffer.as_ref())
            .map(|surface| surface.dc);
        let paint_dc = buffer_dc.unwrap_or(dc);
        let recording_preview = state
            .editor
            .as_ref()
            .is_some_and(|editor| editor.recording_preview);
        let bg_color = if recording_preview || theme.is_dark {
            0x001B_1B1D
        } else {
            0x00F3_F3F3
        };
        let background = CreateSolidBrush(bg_color);
        FillRect(paint_dc, &client, background);
        DeleteObject(background as HGDIOBJ);

        if let (Some(screen), Some(capture), Some(editor)) = (
            state.screen.as_ref(),
            state.capture.as_ref(),
            state.editor.as_ref(),
        ) {
            if editor.recording_preview {
                let video = editor.display_rect;
                draw_pixels(
                    paint_dc,
                    &screen.pixels,
                    screen.width,
                    screen.height,
                    video.left,
                    video.top,
                    video.width() as i32,
                    video.height() as i32,
                    video.left,
                    video.top,
                    video.width() as i32,
                    video.height() as i32,
                );
            } else if let Some(canvas_dc) = canvas_dc {
                BitBlt(
                    paint_dc,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    canvas_dc,
                    0,
                    0,
                    SRCCOPY,
                );
            } else {
                draw_pixels(
                    paint_dc,
                    &screen.pixels,
                    screen.width,
                    screen.height,
                    0,
                    0,
                    screen.width,
                    screen.height,
                    0,
                    0,
                    screen.width,
                    screen.height,
                );
            }
            if !editor.recording_preview {
                draw_rect_border(
                    paint_dc,
                    editor.display_rect.left,
                    editor.display_rect.top,
                    editor.display_rect.width() as i32,
                    editor.display_rect.height() as i32,
                    ThemePalette::colorref(theme.border),
                    1,
                );
            }
            if !editor.recording_preview {
                if capture.show_translated_overlay {
                    if let Some((tw, th, tpixels)) = capture.translated_image.as_ref() {
                        draw_rgba_preview(
                            paint_dc,
                            tpixels,
                            *tw as i32,
                            *th as i32,
                            editor.display_rect.left,
                            editor.display_rect.top,
                            editor.display_rect.width() as i32,
                            editor.display_rect.height() as i32,
                        );
                    }
                }
                draw_annotations_gdi(paint_dc, capture, screen, screen.x, screen.y);
                if let (Some(start), Some(current)) = (capture.drag, capture.current_pos) {
                    draw_preview(paint_dc, screen, capture, start, current, screen.x, screen.y);
                }
            }

            let (toolbar_x, toolbar_y) = toolbar_origin(capture);
            let layout = toolbar_layout_for(capture);
            if editor.recording_preview {
                draw_recording_player(paint_dc, screen.width, screen.height, editor, state, &theme);
            } else {
                let can_undo = !capture.annotations.is_empty();
                let can_redo = !capture.redo_annotations.is_empty();
                draw_toolbar_gdi(
                    paint_dc,
                    toolbar_x,
                    toolbar_y,
                    layout,
                    &capture.toolbar_actions,
                    capture.tool,
                    capture.toolbar_hover,
                    can_undo,
                    can_redo,
                    false,
                    false,
                    &theme,
                );
                if let Some(sec_layout) = secondary_bar_layout_for(capture) {
                    draw_secondary_bar_gdi(
                        paint_dc,
                        sec_layout,
                        capture.tool,
                        capture.secondary_hover,
                        capture.annotation_color,
                        capture.stroke_width,
                        capture.brush_size,
                        capture.text_size,
                        capture.stroke_style,
                        capture.shape_filled,
                        capture.shape_rounded,
                        &theme,
                    );
                    if let Some(sec_hover) = capture.secondary_hover {
                        draw_secondary_bar_tooltip(
                            paint_dc,
                            sec_layout,
                            screen.width,
                            screen.height,
                            sec_hover,
                            capture.tool,
                            &theme,
                        );
                    }
                }
            }
            if let Some(menu) = capture.menu {
                let menu_layout = toolbar_menu_layout_for(capture, menu);
                draw_toolbar_menu_gdi(
                    paint_dc,
                    menu_layout,
                    menu,
                    capture.scroll_axis,
                    capture.menu_hover,
                    &theme,
                );
            }
            if !editor.recording_preview {
                if let Some(index) = capture.toolbar_hover {
                    if let Some(act) = capture.toolbar_action(index) {
                        draw_toolbar_tooltip(
                            paint_dc,
                            toolbar_x,
                            toolbar_y,
                            layout,
                            screen.width,
                            screen.height,
                            act.label(),
                            index,
                            &theme,
                        );
                    }
                }
            }
        }

        if let Some(buffer_dc) = buffer_dc {
            let dirty = paint_struct.rcPaint;
            let width = (dirty.right - dirty.left).max(0);
            let height = (dirty.bottom - dirty.top).max(0);
            if width > 0 && height > 0 {
                BitBlt(
                    dc,
                    dirty.left,
                    dirty.top,
                    width,
                    height,
                    buffer_dc,
                    dirty.left,
                    dirty.top,
                    SRCCOPY,
                );
            }
        }

        EndPaint(hwnd, &paint_struct);
    }
}

unsafe fn draw_recording_player(
    dc: HDC,
    width: i32,
    height: i32,
    editor: &super::editor::EditorCanvas,
    state: &UiState,
    theme: &ThemePalette,
) {
    use super::editor::{compute_player_metrics, PlayerHit};

    let status = state.recording.snapshot();
    let is_exporting = status.phase == "exporting";
    let show_open_folder = !is_exporting && status.last_path.is_some();
    let m = compute_player_metrics(width, height, status.fps.max(1), show_open_folder);
    let s = m.scale;
    let hover = editor.player_hover;

    // SnowAir Studio dual-theme surfaces
    let panel_bg_colorref: u32 = if theme.is_dark { 0x0021_1D1D } else { 0x00FF_FFFF };
    let divider_colorref: u32 = if theme.is_dark { 0x0034_2E2E } else { 0x00E8_E3E0 };
    let text_primary: u32 = if theme.is_dark { 0x00FA_F7F5 } else { 0x0024_1F1C };
    let text_muted: u32 = if theme.is_dark { 0x00A6_9E98 } else { 0x007A_706A };
    let accent_text: u32 = if theme.is_dark { 0x006A_DA4C } else { 0x003E_9916 };
    let danger_text: u32 = if theme.is_dark { 0x006B_6BFF } else { 0x003B_3BD9 };

    let footer_top = height - m.footer_height;

    // 1. Solid header & footer panel fills + 1px crisp hairlines
    let panel_br = CreateSolidBrush(panel_bg_colorref);
    let header_rc = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: m.header_height,
    };
    let footer_rc = RECT {
        left: 0,
        top: footer_top,
        right: width,
        bottom: height,
    };
    FillRect(dc, &header_rc, panel_br);
    FillRect(dc, &footer_rc, panel_br);
    DeleteObject(panel_br as HGDIOBJ);

    let div_br = CreateSolidBrush(divider_colorref);
    let hdr_div = RECT {
        left: 0,
        top: m.header_height - 1,
        right: width,
        bottom: m.header_height,
    };
    let ftr_div = RECT {
        left: 0,
        top: footer_top,
        right: width,
        bottom: footer_top + 1,
    };
    FillRect(dc, &hdr_div, div_br);
    FillRect(dc, &ftr_div, div_br);
    DeleteObject(div_br as HGDIOBJ);

    // 2. GDI+ anti-aliased layer: viewport frame, center play badge, header segmented pills,
    //    timeline scrubber, play/pause button, action pills & progress bar.
    ensure_gdiplus();
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(dc, &mut g) == 0 && !g.is_null() {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);

        // 2a. Video preview cinema border & subtle paused play badge
        let vr = editor.display_rect;
        let vw = vr.width() as f32;
        let vh = vr.height() as f32;
        let frame_border = if theme.is_dark {
            0x30_FFFFFF
        } else {
            0x28_000000
        };
        draw_round_rect_gdip(
            g,
            vr.left as f32 - 0.5,
            vr.top as f32 - 0.5,
            vw + 1.0,
            vh + 1.0,
            2.0 * s,
            None,
            Some((frame_border, (1.0 * s).max(1.0))),
        );

        if !editor.playing && !is_exporting && vw > 96.0 * s && vh > 96.0 * s {
            let cx = vr.left as f32 + vw * 0.5;
            let cy = vr.top as f32 + vh * 0.5;
            let video_hovered = hover == Some(PlayerHit::Video);
            let badge_r = if video_hovered { 29.0 * s } else { 26.0 * s };
            let badge_bg = if video_hovered { 0xC8_141418 } else { 0x9C_141418 };
            let mut br: *mut GpSolidFill = null_mut();
            if GdipCreateSolidFill(badge_bg, &mut br) == 0 && !br.is_null() {
                GdipFillEllipse(
                    g,
                    br as *mut GpBrush,
                    cx - badge_r,
                    cy - badge_r,
                    badge_r * 2.0,
                    badge_r * 2.0,
                );
                GdipDeleteBrush(br as *mut GpBrush);
            }
            let mut pen: *mut GpPen = null_mut();
            if GdipCreatePen1(
                if video_hovered { 0x90_FFFFFF } else { 0x55_FFFFFF },
                (1.5 * s).max(1.0),
                UnitPixel,
                &mut pen,
            ) == 0
                && !pen.is_null()
            {
                GdipDrawEllipse(
                    g,
                    pen,
                    cx - badge_r,
                    cy - badge_r,
                    badge_r * 2.0,
                    badge_r * 2.0,
                );
                GdipDeletePen(pen);
            }
            draw_gdip_play_triangle(g, cx + 2.0 * s, cy, 10.5 * s, 0xFF_FFFFFF);
        }

        // 2b. Header status pulse dot
        let dot_cx = m.side_pad as f32 + 6.0 * s;
        let dot_cy = m.header_height as f32 * 0.5;
        let (dot_core, dot_halo) = if is_exporting {
            (0xFF_F59E0B, 0x38_F59E0B)
        } else if editor.playing {
            (PLAYER_ACCENT_ARGB, 0x45_34C759)
        } else {
            (PLAYER_ACCENT_ARGB, 0x26_34C759)
        };
        let mut halo_br: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(dot_halo, &mut halo_br) == 0 && !halo_br.is_null() {
            let hr = 9.0 * s;
            GdipFillEllipse(g, halo_br as *mut GpBrush, dot_cx - hr, dot_cy - hr, hr * 2.0, hr * 2.0);
            GdipDeleteBrush(halo_br as *mut GpBrush);
        }
        let mut core_br: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(dot_core, &mut core_br) == 0 && !core_br.is_null() {
            let cr = 4.6 * s;
            GdipFillEllipse(g, core_br as *mut GpBrush, dot_cx - cr, dot_cy - cr, cr * 2.0, cr * 2.0);
            GdipDeleteBrush(core_br as *mut GpBrush);
        }

        // 2c. Header inline segmented presets (Resolution + FPS)
        let tray_bg = if theme.is_dark { 0xFF_141418 } else { 0xFF_EFEFF4 };
        let tray_border = if theme.is_dark { 0xFF_2D2D34 } else { 0xFF_DCDFE6 };
        let seg_active_bg = if theme.is_dark { 0xFF_2B2E35 } else { 0xFF_FFFFFF };
        let seg_active_border = argb(theme.accent, Some(if theme.is_dark { 0x99 } else { 0x80 }));
        let seg_hover_bg = if theme.is_dark { 0xFF_222228 } else { 0xFF_E4E7EE };

        // Quality group tray
        let qg = m.quality_group_rect;
        draw_round_rect_gdip(
            g,
            qg.0 as f32,
            qg.1 as f32,
            (qg.2 - qg.0) as f32,
            (qg.3 - qg.1) as f32,
            9.0 * s,
            Some(tray_bg),
            Some((tray_border, (1.0 * s).max(1.0))),
        );
        for (idx, &rect) in m.quality_rects.iter().enumerate() {
            let active = editor.export_quality_preset == idx as u8;
            let hovered = !is_exporting && hover == Some(PlayerHit::Quality(idx as u8));
            if active {
                if !theme.is_dark {
                    draw_soft_shadow_gdip(
                        g,
                        rect.0 as f32,
                        rect.1 as f32,
                        (rect.2 - rect.0) as f32,
                        (rect.3 - rect.1) as f32,
                        7.0 * s,
                        s * 0.45,
                        false,
                    );
                }
                draw_round_rect_gdip(
                    g,
                    rect.0 as f32,
                    rect.1 as f32,
                    (rect.2 - rect.0) as f32,
                    (rect.3 - rect.1) as f32,
                    7.0 * s,
                    Some(seg_active_bg),
                    Some((seg_active_border, (1.0 * s).max(1.0))),
                );
            } else if hovered {
                draw_round_rect_gdip(
                    g,
                    rect.0 as f32,
                    rect.1 as f32,
                    (rect.2 - rect.0) as f32,
                    (rect.3 - rect.1) as f32,
                    7.0 * s,
                    Some(seg_hover_bg),
                    None,
                );
            }
        }

        // FPS group tray
        let fg = m.fps_group_rect;
        draw_round_rect_gdip(
            g,
            fg.0 as f32,
            fg.1 as f32,
            (fg.2 - fg.0) as f32,
            (fg.3 - fg.1) as f32,
            9.0 * s,
            Some(tray_bg),
            Some((tray_border, (1.0 * s).max(1.0))),
        );
        for (idx, (&rect, &fps_val)) in m.fps_rects.iter().zip(m.fps_values.iter()).enumerate() {
            let active = editor.export_fps_preset == fps_val;
            let hovered = !is_exporting && hover == Some(PlayerHit::Fps(idx));
            if active {
                if !theme.is_dark {
                    draw_soft_shadow_gdip(
                        g,
                        rect.0 as f32,
                        rect.1 as f32,
                        (rect.2 - rect.0) as f32,
                        (rect.3 - rect.1) as f32,
                        7.0 * s,
                        s * 0.45,
                        false,
                    );
                }
                draw_round_rect_gdip(
                    g,
                    rect.0 as f32,
                    rect.1 as f32,
                    (rect.2 - rect.0) as f32,
                    (rect.3 - rect.1) as f32,
                    7.0 * s,
                    Some(seg_active_bg),
                    Some((seg_active_border, (1.0 * s).max(1.0))),
                );
            } else if hovered {
                draw_round_rect_gdip(
                    g,
                    rect.0 as f32,
                    rect.1 as f32,
                    (rect.2 - rect.0) as f32,
                    (rect.3 - rect.1) as f32,
                    7.0 * s,
                    Some(seg_hover_bg),
                    None,
                );
            }
        }

        // 2d. Interactive Scrubber Timeline Strip
        let timeline_frame_count = status.timeline_frame_count();
        let progress = if timeline_frame_count <= 1 {
            0.0
        } else {
            editor.frame_index as f64 / (timeline_frame_count.saturating_sub(1)) as f64
        }
        .clamp(0.0, 1.0);
        let tl_left = m.timeline_left as f32;
        let tl_right = m.timeline_right.max(m.timeline_left + 1) as f32;
        let tl_y = m.timeline_y as f32;
        let tl_h = (6.0 * s).max(4.0);
        let knob_x = tl_left + (tl_right - tl_left) * progress as f32;
        let tl_track_bg = if theme.is_dark { 0xFF_2D2D34 } else { 0xFF_D2D6DE };

        draw_round_rect_gdip(
            g,
            tl_left,
            tl_y - tl_h * 0.5,
            tl_right - tl_left,
            tl_h,
            tl_h * 0.5,
            Some(tl_track_bg),
            None,
        );
        let played_w = (knob_x - tl_left).max(tl_h);
        draw_round_rect_gdip(
            g,
            tl_left,
            tl_y - tl_h * 0.5,
            played_w.min(tl_right - tl_left),
            tl_h,
            tl_h * 0.5,
            Some(PLAYER_ACCENT_ARGB),
            None,
        );

        let tl_active = editor.seeking || hover == Some(PlayerHit::Timeline);
        if tl_active {
            let mut glow: *mut GpSolidFill = null_mut();
            if GdipCreateSolidFill(0x40_34C759, &mut glow) == 0 && !glow.is_null() {
                let gr = 12.5 * s;
                GdipFillEllipse(g, glow as *mut GpBrush, knob_x - gr, tl_y - gr, gr * 2.0, gr * 2.0);
                GdipDeleteBrush(glow as *mut GpBrush);
            }
        }
        let knob_r = if tl_active { 7.5 * s } else { 6.5 * s };
        let mut knob_fill: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(0xFF_FFFFFF, &mut knob_fill) == 0 && !knob_fill.is_null() {
            GdipFillEllipse(
                g,
                knob_fill as *mut GpBrush,
                knob_x - knob_r,
                tl_y - knob_r,
                knob_r * 2.0,
                knob_r * 2.0,
            );
            GdipDeleteBrush(knob_fill as *mut GpBrush);
        }
        let mut knob_ring: *mut GpPen = null_mut();
        if GdipCreatePen1(PLAYER_ACCENT_ARGB, (2.4 * s).max(2.0), UnitPixel, &mut knob_ring) == 0
            && !knob_ring.is_null()
        {
            GdipDrawEllipse(
                g,
                knob_ring,
                knob_x - knob_r,
                tl_y - knob_r,
                knob_r * 2.0,
                knob_r * 2.0,
            );
            GdipDeletePen(knob_ring);
        }

        // 2e. Footer controls
        if is_exporting {
            // Smooth GDI+ progress bar on left
            let bar_left = m.side_pad as f32;
            let bar_right = (m.cancel_export_rect.0 - (20.0 * s).round() as i32).max(m.side_pad + 40) as f32;
            let bar_h = (8.0 * s).max(6.0);
            let bar_top = footer_top as f32 + (46.0 * s);
            draw_round_rect_gdip(
                g,
                bar_left,
                bar_top,
                bar_right - bar_left,
                bar_h,
                bar_h * 0.5,
                Some(tl_track_bg),
                None,
            );
            let fill_w = (bar_right - bar_left) * (f32::from(status.progress_percent.min(100)) / 100.0);
            if fill_w > 1.0 {
                draw_round_rect_gdip(
                    g,
                    bar_left,
                    bar_top,
                    fill_w.max(bar_h).min(bar_right - bar_left),
                    bar_h,
                    bar_h * 0.5,
                    Some(PLAYER_ACCENT_ARGB),
                    None,
                );
            }

            // Cancel Export pill on right
            let cancel_hovered = hover == Some(PlayerHit::CancelExport);
            let c_bg = if cancel_hovered {
                if theme.is_dark { 0x45_EF4444 } else { 0x26_EF4444 }
            } else if theme.is_dark {
                0x24_EF4444
            } else {
                0x14_EF4444
            };
            let c_border = if cancel_hovered { 0xCC_EF4444 } else { 0x80_EF4444 };
            draw_gdip_pill(g, m.cancel_export_rect, 9.0 * s, c_bg, c_border, s);
        } else {
            // Play / Pause button
            let play_hovered = hover == Some(PlayerHit::Play);
            let sec_bg = if theme.is_dark { 0xFF_2A2A31 } else { 0xFF_F3F4F7 };
            let sec_hover_bg = if theme.is_dark { 0xFF_35353E } else { 0xFF_E7E9EF };
            let sec_border = if theme.is_dark { 0xFF_3D3D47 } else { 0xFF_D7DBE3 };

            draw_gdip_pill(
                g,
                m.play_rect,
                10.0 * s,
                if play_hovered { sec_hover_bg } else { sec_bg },
                if play_hovered { argb(theme.accent, Some(0x99)) } else { sec_border },
                s,
            );
            let pcx = (m.play_rect.0 + m.play_rect.2) as f32 * 0.5;
            let pcy = (m.play_rect.1 + m.play_rect.3) as f32 * 0.5;
            if editor.playing {
                let bw = 3.6 * s;
                let bh = 13.0 * s;
                let gap = 2.6 * s;
                draw_round_rect_gdip(
                    g,
                    pcx - gap - bw,
                    pcy - bh * 0.5,
                    bw,
                    bh,
                    1.5 * s,
                    Some(PLAYER_ACCENT_ARGB),
                    None,
                );
                draw_round_rect_gdip(
                    g,
                    pcx + gap,
                    pcy - bh * 0.5,
                    bw,
                    bh,
                    1.5 * s,
                    Some(PLAYER_ACCENT_ARGB),
                    None,
                );
            } else {
                draw_gdip_play_triangle(g, pcx + 1.5 * s, pcy, 7.2 * s, PLAYER_ACCENT_ARGB);
            }

            // Optional "打开位置" subtle pill
            if let Some(folder_rect) = m.open_folder_rect {
                let f_hov = hover == Some(PlayerHit::OpenFolder);
                let f_bg = if f_hov {
                    argb(theme.accent, Some(if theme.is_dark { 0x3A } else { 0x28 }))
                } else {
                    argb(theme.accent, Some(if theme.is_dark { 0x20 } else { 0x16 }))
                };
                draw_gdip_pill(
                    g,
                    folder_rect,
                    7.0 * s,
                    f_bg,
                    argb(theme.accent, Some(0x70)),
                    s,
                );
            }

            // Direct 1-click export pills: [ 复制帧 ] [ 导出 WebP ] [ 导出 GIF ] [ 导出 MP4 ]
            let copy_hov = hover == Some(PlayerHit::CopyFrame);
            draw_gdip_pill(
                g,
                m.copy_rect,
                9.0 * s,
                if copy_hov { sec_hover_bg } else { sec_bg },
                sec_border,
                s,
            );

            let webp_hov = hover == Some(PlayerHit::ExportWebp);
            draw_gdip_pill(
                g,
                m.webp_rect,
                9.0 * s,
                if webp_hov { sec_hover_bg } else { sec_bg },
                if webp_hov { argb(theme.accent, Some(0x90)) } else { sec_border },
                s,
            );

            let gif_hov = hover == Some(PlayerHit::ExportGif);
            draw_gdip_pill(
                g,
                m.gif_rect,
                9.0 * s,
                if gif_hov { sec_hover_bg } else { sec_bg },
                if gif_hov { argb(theme.accent, Some(0x90)) } else { sec_border },
                s,
            );

            let mp4_hov = hover == Some(PlayerHit::ExportMp4);
            draw_gdip_pill(
                g,
                m.mp4_rect,
                9.0 * s,
                if mp4_hov { PLAYER_ACCENT_HOVER_ARGB } else { PLAYER_ACCENT_ARGB },
                if mp4_hov { PLAYER_ACCENT_HOVER_ARGB } else { PLAYER_ACCENT_ARGB },
                s,
            );
        }

        GdipDeleteGraphics(g);
    }

    // 3. Crisp ClearType Microsoft YaHei UI text layer
    let font_title = create_ui_font(15.0, s, true);
    let font_sub = create_ui_font(11.5, s, false);
    let font_btn = create_ui_font(13.0, s, true);
    let font_seg = create_ui_font(12.0, s, true);
    let font_seg_muted = create_ui_font(12.0, s, false);

    // Header title & metadata
    let text_left = m.side_pad + (22.0 * s).round() as i32;
    let text_right = (m.quality_group_rect.0 - (14.0 * s).round() as i32).max(text_left + 80);
    draw_studio_text(
        dc,
        font_title,
        text_left,
        (9.0 * s).round() as i32,
        text_right,
        (33.0 * s).round() as i32,
        "录制预览与导出",
        text_primary,
        0,
    );
    let metadata = format!(
        "{}×{}  ·  {}  ·  {} FPS  ·  {} 帧",
        editor.source_width,
        editor.source_height,
        format_player_time(status.elapsed_ms),
        status.fps.max(1),
        status.frame_count
    );
    draw_studio_text(
        dc,
        font_sub,
        text_left,
        (34.0 * s).round() as i32,
        text_right,
        (54.0 * s).round() as i32,
        &metadata,
        text_muted,
        0,
    );

    // Header segmented labels
    let q_labels = ["540p", "720p 荐", "1080p"];
    for (idx, (&rect, &label)) in m.quality_rects.iter().zip(q_labels.iter()).enumerate() {
        let active = editor.export_quality_preset == idx as u8;
        draw_studio_text(
            dc,
            if active { font_seg } else { font_seg_muted },
            rect.0,
            rect.1,
            rect.2,
            rect.3,
            label,
            if active { accent_text } else { text_muted },
            1,
        );
    }
    for (&rect, &fps_val) in m.fps_rects.iter().zip(m.fps_values.iter()) {
        let active = editor.export_fps_preset == fps_val;
        let label = format!("{fps_val}fps");
        draw_studio_text(
            dc,
            if active { font_seg } else { font_seg_muted },
            rect.0,
            rect.1,
            rect.2,
            rect.3,
            &label,
            if active { accent_text } else { text_muted },
            1,
        );
    }

    // Footer text
    if is_exporting {
        let bar_left = m.side_pad;
        let bar_right = (m.cancel_export_rect.0 - (20.0 * s).round() as i32).max(bar_left + 40);
        let pct_w = (54.0 * s).round() as i32;
        let msg = status.message.as_deref().unwrap_or("正在导出…");
        draw_studio_text(
            dc,
            font_btn,
            bar_left,
            footer_top + (15.0 * s).round() as i32,
            bar_right - pct_w,
            footer_top + (38.0 * s).round() as i32,
            msg,
            text_primary,
            0,
        );
        let pct = format!("{}%", status.progress_percent);
        draw_studio_text(
            dc,
            font_btn,
            bar_right - pct_w,
            footer_top + (15.0 * s).round() as i32,
            bar_right,
            footer_top + (38.0 * s).round() as i32,
            &pct,
            accent_text,
            2,
        );
        draw_studio_text(
            dc,
            font_btn,
            m.cancel_export_rect.0,
            m.cancel_export_rect.1,
            m.cancel_export_rect.2,
            m.cancel_export_rect.3,
            "取消导出",
            danger_text,
            1,
        );
    } else {
        let timeline_frame_count = status.timeline_frame_count();
        let fps = status.fps.max(1) as u64;
        let current_ms = editor.frame_index as u64 * 1000 / fps;
        let duration_ms = status
            .elapsed_ms
            .max(timeline_frame_count.saturating_sub(1) as u64 * 1000 / fps);
        let timestamp = format!(
            "{} / {}",
            format_player_time(current_ms),
            format_player_time(duration_ms)
        );
        let msg = status.message.as_deref();
        let top_y = footer_top + (15.0 * s).round() as i32;
        let mid_y = footer_top + (38.0 * s).round() as i32;
        let bot_y = footer_top + (59.0 * s).round() as i32;

        if let Some(m_str) = msg.filter(|v| v.starts_with("导出完成：")) {
            let file_name = std::path::Path::new(m_str.trim_start_matches("导出完成："))
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(m_str);
            draw_studio_text(
                dc,
                font_btn,
                m.status_left,
                top_y,
                m.status_right,
                mid_y,
                "✓ 导出完成",
                accent_text,
                0,
            );
            draw_studio_text(
                dc,
                font_sub,
                m.status_left,
                mid_y,
                m.status_right,
                bot_y,
                file_name,
                text_muted,
                0,
            );
        } else if let Some(m_str) = msg.filter(|v| v.starts_with("导出失败：")) {
            draw_studio_text(
                dc,
                font_btn,
                m.status_left,
                top_y,
                m.status_right,
                mid_y,
                "导出失败",
                danger_text,
                0,
            );
            draw_studio_text(
                dc,
                font_sub,
                m.status_left,
                mid_y,
                m.status_right,
                bot_y,
                m_str.trim_start_matches("导出失败："),
                danger_text,
                0,
            );
        } else {
            draw_studio_text(
                dc,
                font_btn,
                m.status_left,
                top_y,
                m.status_right,
                mid_y,
                &timestamp,
                text_primary,
                0,
            );
            let sub_hint = if msg == Some("已取消导出") {
                "已取消导出 · 可重新调整参数导出"
            } else {
                "空格播放/暂停 · 动图参数仅作用于 GIF/WebP"
            };
            draw_studio_text(
                dc,
                font_sub,
                m.status_left,
                mid_y,
                m.status_right,
                bot_y,
                sub_hint,
                text_muted,
                0,
            );
        }

        if let Some(folder_rect) = m.open_folder_rect {
            draw_studio_text(
                dc,
                font_seg,
                folder_rect.0,
                folder_rect.1,
                folder_rect.2,
                folder_rect.3,
                "打开位置",
                accent_text,
                1,
            );
        }

        draw_studio_text(
            dc,
            font_btn,
            m.copy_rect.0,
            m.copy_rect.1,
            m.copy_rect.2,
            m.copy_rect.3,
            "复制帧",
            text_primary,
            1,
        );
        draw_studio_text(
            dc,
            font_btn,
            m.webp_rect.0,
            m.webp_rect.1,
            m.webp_rect.2,
            m.webp_rect.3,
            "导出 WebP",
            text_primary,
            1,
        );
        draw_studio_text(
            dc,
            font_btn,
            m.gif_rect.0,
            m.gif_rect.1,
            m.gif_rect.2,
            m.gif_rect.3,
            "导出 GIF",
            text_primary,
            1,
        );
        draw_studio_text(
            dc,
            font_btn,
            m.mp4_rect.0,
            m.mp4_rect.1,
            m.mp4_rect.2,
            m.mp4_rect.3,
            "导出 MP4",
            0x00FF_FFFF,
            1,
        );
    }

    DeleteObject(font_title as HGDIOBJ);
    DeleteObject(font_sub as HGDIOBJ);
    DeleteObject(font_btn as HGDIOBJ);
    DeleteObject(font_seg as HGDIOBJ);
    DeleteObject(font_seg_muted as HGDIOBJ);
}

unsafe fn draw_gdip_pill(
    g: *mut GpGraphics,
    rect: (i32, i32, i32, i32),
    radius: f32,
    fill_argb: u32,
    border_argb: u32,
    scale: f32,
) {
    draw_round_rect_gdip(
        g,
        rect.0 as f32,
        rect.1 as f32,
        (rect.2 - rect.0) as f32,
        (rect.3 - rect.1) as f32,
        radius,
        Some(fill_argb),
        Some((border_argb, (1.0 * scale).max(1.0))),
    );
}

unsafe fn draw_gdip_play_triangle(
    g: *mut GpGraphics,
    cx: f32,
    cy: f32,
    radius: f32,
    color_argb: u32,
) {
    let mut path: *mut GpPath = null_mut();
    if GdipCreatePath(FillModeAlternate, &mut path) != 0 || path.is_null() {
        return;
    }
    let x1 = cx - radius * 0.72;
    let y1 = cy - radius * 0.92;
    let x2 = cx - radius * 0.72;
    let y2 = cy + radius * 0.92;
    let x3 = cx + radius * 0.96;
    let y3 = cy;
    GdipAddPathLine(path, x1, y1, x2, y2);
    GdipAddPathLine(path, x2, y2, x3, y3);
    GdipClosePathFigure(path);
    let mut br: *mut GpSolidFill = null_mut();
    if GdipCreateSolidFill(color_argb, &mut br) == 0 && !br.is_null() {
        GdipFillPath(g, br as *mut GpBrush, path);
        GdipDeleteBrush(br as *mut GpBrush);
    }
    GdipDeletePath(path);
}

unsafe fn draw_studio_text(
    dc: HDC,
    font: windows_sys::Win32::Graphics::Gdi::HFONT,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    value: &str,
    color: u32,
    align: i32, // 0 = left, 1 = center, 2 = right
) {
    if right <= left || bottom <= top {
        return;
    }
    let old_font = SelectObject(dc, font as HGDIOBJ);
    SetBkMode(dc, 1);
    SetTextColor(dc, color);
    let text = wide(value);
    let mut rect = RECT {
        left,
        top,
        right,
        bottom,
    };
    // DT_VCENTER (0x04) | DT_SINGLELINE (0x20) | DT_END_ELLIPSIS (0x8000) | DT_NOPREFIX (0x0800)
    let align_flag = match align {
        1 => 0x0000_0001, // DT_CENTER
        2 => 0x0000_0002, // DT_RIGHT
        _ => 0x0000_0000, // DT_LEFT
    };
    DrawTextW(
        dc,
        text.as_ptr(),
        -1,
        &mut rect,
        0x0000_0004 | 0x0000_0020 | 0x0000_0800 | 0x0000_8000 | align_flag,
    );
    SelectObject(dc, old_font);
}

fn format_player_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

pub(crate) fn rgba_preview_bgra(
    pixels: &[u8],
    source_width: i32,
    source_height: i32,
    target_width: i32,
    target_height: i32,
) -> Option<Vec<u8>> {
    let source_width = usize::try_from(source_width).ok()?;
    let source_height = usize::try_from(source_height).ok()?;
    let target_width = usize::try_from(target_width).ok()?;
    let target_height = usize::try_from(target_height).ok()?;
    if source_width == 0 || source_height == 0 || target_width == 0 || target_height == 0 {
        return None;
    }
    let source_len = source_width.checked_mul(source_height)?.checked_mul(4)?;
    if pixels.len() != source_len {
        return None;
    }
    let target_len = target_width.checked_mul(target_height)?.checked_mul(4)?;
    let mut bgra = vec![0u8; target_len];
    for y in 0..target_height {
        let source_y = y.saturating_mul(source_height) / target_height;
        for x in 0..target_width {
            let source_x = x.saturating_mul(source_width) / target_width;
            let source = (source_y * source_width + source_x) * 4;
            let target = (y * target_width + x) * 4;
            bgra[target] = pixels[source + 2];
            bgra[target + 1] = pixels[source + 1];
            bgra[target + 2] = pixels[source];
            bgra[target + 3] = pixels[source + 3];
        }
    }
    Some(bgra)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_rgba_preview(
    dc: HDC,
    pixels: &[u8],
    source_width: i32,
    source_height: i32,
    destination_x: i32,
    destination_y: i32,
    destination_width: i32,
    destination_height: i32,
) {
    let Some(bgra) = rgba_preview_bgra(
        pixels,
        source_width,
        source_height,
        destination_width,
        destination_height,
    ) else {
        return;
    };
    draw_pixels(
        dc,
        &bgra,
        destination_width,
        destination_height,
        0,
        0,
        destination_width,
        destination_height,
        destination_x,
        destination_y,
        destination_width,
        destination_height,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_pixels(
    dc: HDC,
    pixels: &[u8],
    source_width: i32,
    source_height: i32,
    source_x: i32,
    source_y: i32,
    source_width_rect: i32,
    source_height_rect: i32,
    destination_x: i32,
    destination_y: i32,
    destination_width: i32,
    destination_height: i32,
) {
    if dc.is_null()
        || pixels.is_empty()
        || source_width <= 0
        || source_height <= 0
        || source_width_rect <= 0
        || source_height_rect <= 0
        || destination_width <= 0
        || destination_height <= 0
    {
        return;
    }
    let info = bitmap_info(source_width, -source_height);
    unsafe {
        StretchDIBits(
            dc,
            destination_x,
            destination_y,
            destination_width,
            destination_height,
            source_x,
            source_y,
            source_width_rect,
            source_height_rect,
            pixels.as_ptr() as *const c_void,
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

pub(super) fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { zeroed() }
        },
        bmiColors: [unsafe { zeroed() }],
    }
}
