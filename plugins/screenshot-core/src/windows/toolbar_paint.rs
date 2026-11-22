//! SnowAir-grade GDI+ anti-aliased floating toolbar, secondary property bar,
//! tooltips, dropdown menus, and uniform DPI-scaled monochrome vector icon renderer
//! (matching Dark, Light & Custom themes and 1:1 synchronized with the Settings preview).

use super::wide;
use crate::model::{ScreenRect, StrokeStyle};
use crate::scroll::MatchAxis;
use crate::theme::ThemePalette;
use crate::toolbar::{
    button_origin as toolbar_button_origin, layout_for_item_count_scaled, variants_for_tool,
    RecordingSetupLayout, SecondaryBarItem, SecondaryBarLayout, Tool, ToolbarAction, ToolbarLayout,
    ToolbarMenu, ToolbarMenuLayout, COLOR_SWATCHES, MOSAIC_SIZES, STROKE_SIZES, TEXT_SIZES,
};

use std::ptr::null_mut;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, DrawTextW, SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_MEDIUM, FW_NORMAL, HDC,
    HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    DashCapRound, FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC,
    GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics,
    GdipDeletePath, GdipDeletePen, GdipDrawArc, GdipDrawBezier, GdipDrawEllipse, GdipDrawLine,
    GdipDrawPath, GdipDrawRectangle, GdipFillEllipse, GdipFillPath, GdipFillRectangle,
    GdipSetPenLineCap197819, GdipSetPenLineJoin, GdipSetPixelOffsetMode, GdipSetSmoothingMode,
    GdiplusStartup, GdiplusStartupInput, GpBrush, GpGraphics, GpPath, GpPen, GpSolidFill,
    LineCapRound, LineJoinRound, PixelOffsetModeHalf, SmoothingModeAntiAlias8x8, UnitPixel,
};

pub(super) const SCROLL_TOOLBAR_ACTIONS: [ToolbarAction; 8] = [
    ToolbarAction::MoveToolbar,
    ToolbarAction::ScrollPauseToggle,
    ToolbarAction::Edit,
    ToolbarAction::Ocr,
    ToolbarAction::Pin,
    ToolbarAction::Save,
    ToolbarAction::Cancel,
    ToolbarAction::Finish,
];

pub(super) fn scroll_toolbar_layout() -> ToolbarLayout {
    layout_for_item_count_scaled(
        SCROLL_TOOLBAR_ACTIONS.len(),
        i32::MAX,
        crate::theme::system_dpi_scale(),
    )
}

pub(super) fn scroll_toolbar_action(index: usize) -> Option<ToolbarAction> {
    SCROLL_TOOLBAR_ACTIONS.get(index).copied()
}

pub(crate) fn toolbar_layout(item_count: usize, screen: ScreenRect) -> ToolbarLayout {
    layout_for_item_count_scaled(item_count, screen.width() as i32, 1.0)
}

pub(crate) fn toolbar_layout_scaled(
    item_count: usize,
    screen: ScreenRect,
    scale: f32,
) -> ToolbarLayout {
    layout_for_item_count_scaled(item_count, screen.width() as i32, scale)
}

pub(crate) fn toolbar_position_for_layout(
    selection: ScreenRect,
    screen: ScreenRect,
    layout: ToolbarLayout,
) -> (i32, i32) {
    let gap = (10.0 * layout.scale()).round() as i32;
    let mut x = selection.left;
    if x + layout.width > screen.right - 8 {
        x = (selection.right - layout.width).max(screen.left + 8);
    }
    x = x.clamp(
        screen.left + 8,
        (screen.right - layout.width - 8).max(screen.left + 8),
    );
    let mut y = selection.bottom + gap;
    if y + layout.height > screen.bottom - 8 {
        y = (selection.top - layout.height - gap).max(screen.top + 8);
    }
    (x, y)
}

pub(super) fn scroll_toolbar_position(selection: ScreenRect, screen: ScreenRect) -> (i32, i32) {
    toolbar_position_for_layout(selection, screen, scroll_toolbar_layout())
}

pub(super) fn ensure_gdiplus() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| unsafe {
        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: 0,
            SuppressExternalCodecs: 0,
        };
        let _ = GdiplusStartup(&mut token, &input, null_mut());
    });
}

pub(super) fn argb(c: [u8; 4], alpha_override: Option<u8>) -> u32 {
    let a = u32::from(alpha_override.unwrap_or(c[3]));
    (a << 24) | (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2])
}

pub(super) unsafe fn create_ui_font(dp_size: f32, scale: f32, medium: bool) -> HFONT {
    let px = (dp_size * scale).round().max(11.0) as i32;
    CreateFontW(
        -px,
        0,
        0,
        0,
        if medium { FW_MEDIUM } else { FW_NORMAL } as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        (DEFAULT_PITCH | FF_DONTCARE) as u32,
        wide("Microsoft YaHei UI").as_ptr(),
    )
}

unsafe fn build_round_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> *mut GpPath {
    let mut path: *mut GpPath = null_mut();
    if GdipCreatePath(FillModeAlternate, &mut path) != 0 || path.is_null() {
        return null_mut();
    }
    let rad = r.min(w * 0.5).min(h * 0.5).max(0.5);
    let d = rad * 2.0;
    GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
    GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
    GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
    GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(path);
    path
}

pub(super) unsafe fn draw_round_rect_gdip(
    g: *mut GpGraphics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    fill_argb: Option<u32>,
    stroke_argb: Option<(u32, f32)>,
) {
    let path = build_round_rect_path(x, y, w, h, r);
    if path.is_null() {
        return;
    }
    if let Some(fill_c) = fill_argb {
        let mut brush: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(fill_c, &mut brush) == 0 && !brush.is_null() {
            GdipFillPath(g, brush as *mut GpBrush, path);
            GdipDeleteBrush(brush as *mut GpBrush);
        }
    }
    if let Some((stroke_c, stroke_w)) = stroke_argb {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(stroke_c, stroke_w, UnitPixel, &mut pen) == 0 && !pen.is_null() {
            GdipDrawPath(g, pen, path);
            GdipDeletePen(pen);
        }
    }
    GdipDeletePath(path);
}

pub(super) unsafe fn draw_soft_shadow_gdip(
    g: *mut GpGraphics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    scale: f32,
    is_dark: bool,
) {
    let layers: [(f32, f32, u8); 4] = if is_dark {
        [
            (4.5 * scale, 4.5 * scale, 14),
            (3.0 * scale, 3.5 * scale, 22),
            (1.8 * scale, 2.5 * scale, 32),
            (0.8 * scale, 1.5 * scale, 44),
        ]
    } else {
        [
            (4.0 * scale, 4.0 * scale, 8),
            (2.5 * scale, 3.0 * scale, 14),
            (1.5 * scale, 2.0 * scale, 20),
            (0.6 * scale, 1.2 * scale, 26),
        ]
    };
    for (spread, dy, alpha) in layers {
        let col = (u32::from(alpha) << 24) | 0x0000_0000;
        draw_round_rect_gdip(
            g,
            x - spread,
            y + dy - spread * 0.35,
            w + spread * 2.0,
            h + spread * 1.4,
            r + spread * 0.6,
            Some(col),
            None,
        );
    }
}

/// SnowAir Floating Main Toolbar (100% GDI+ anti-aliased, DPI-scaled, soft shadow, crisp icons)
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_toolbar_gdi(
    dc: HDC,
    x: i32,
    y: i32,
    layout: ToolbarLayout,
    actions: &[ToolbarAction],
    active_tool: Tool,
    hovered: Option<usize>,
    can_undo: bool,
    can_redo: bool,
    scroll_paused: bool,
    demo_passthrough: bool,
    theme: &ThemePalette,
) {
    ensure_gdiplus();
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(dc, &mut g) != 0 || g.is_null() {
        return;
    }
    GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
    GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);

    let s = layout.scale();
    let fx = x as f32;
    let fy = y as f32;
    let fw = layout.width as f32;
    let fh = layout.height as f32;
    let outer_r = 12.0 * s;

    // 1. Soft multi-layer elevation shadow
    draw_soft_shadow_gdip(g, fx, fy, fw, fh, outer_r, s, theme.is_dark);

    // 2. Smooth SnowAir rounded container + crisp subtle 1px border
    let border_argb = argb(theme.icon, Some(if theme.is_dark { 0x2A } else { 0x26 }));
    draw_round_rect_gdip(
        g,
        fx,
        fy,
        fw,
        fh,
        outer_r,
        Some(argb(theme.bg, Some(0xFA))),
        Some((border_argb, (1.0 * s).max(1.0))),
    );

    let icon_argb = argb(theme.icon, Some(0xFF));
    let drag_argb = argb(theme.icon, Some(0x78));
    let disabled_argb = argb(theme.icon_disabled, Some(0x88));
    let accent_argb = argb(theme.accent, Some(0xFF));
    let danger_argb = argb(theme.danger, Some(0xFF));
    let hover_argb = argb(theme.hover, Some(0xFF));
    let active_bg_argb = argb(theme.accent, Some(if theme.is_dark { 0x26 } else { 0x1E }));
    let divider_argb = argb(theme.icon, Some(0x28));

    let mut div_pen: *mut GpPen = null_mut();
    GdipCreatePen1(divider_argb, (1.0 * s).max(1.0), UnitPixel, &mut div_pen);

    for (pos, &action) in actions.iter().enumerate() {
        let Some((bx, by)) = toolbar_button_origin((x, y), pos, layout) else {
            continue;
        };
        let cx = bx as f32 + layout.button_w as f32 * 0.5;
        let cy = by as f32 + layout.button_h as f32 * 0.5;

        // Subtle SnowAir group divider
        if pos > 0 && (pos == 1 || action.needs_leading_divider()) && !div_pen.is_null() {
            let sep_x = bx as f32 - 0.5;
            let half_h = 10.5 * s;
            GdipDrawLine(g, div_pen, sep_x, cy - half_h, sep_x, cy + half_h);
        }

        let is_active = action.matches_tool(active_tool)
            || (action == ToolbarAction::DemoPassThrough && demo_passthrough)
            || (action == ToolbarAction::ScrollPauseToggle && scroll_paused);
        let is_hovered = hovered == Some(pos);
        let disabled = (action == ToolbarAction::Undo && !can_undo)
            || (action == ToolbarAction::Redo && !can_redo);

        let (fill_opt, draw_col) = if disabled {
            (None, disabled_argb)
        } else {
            match action {
                ToolbarAction::MoveToolbar => (is_hovered.then_some(hover_argb), drag_argb),
                ToolbarAction::Finish => (is_hovered.then_some(hover_argb), accent_argb),
                ToolbarAction::Cancel => (is_hovered.then_some(hover_argb), danger_argb),
                _ if is_active => (Some(active_bg_argb), accent_argb),
                _ if is_hovered => (Some(hover_argb), icon_argb),
                _ => (None, icon_argb),
            }
        };

        if let Some(fill_col) = fill_opt {
            let inset = 3.0 * s;
            draw_round_rect_gdip(
                g,
                bx as f32 + inset,
                by as f32 + inset,
                layout.button_w as f32 - inset * 2.0,
                layout.button_h as f32 - inset * 2.0,
                8.5 * s,
                Some(fill_col),
                None,
            );
        }

        // Sleek SnowAir active pill indicator bar at bottom center
        if is_active
            && !disabled
            && !matches!(
                action,
                ToolbarAction::Finish | ToolbarAction::Cancel | ToolbarAction::MoveToolbar
            )
        {
            let ind_w = 10.0 * s;
            let ind_h = 2.6 * s;
            draw_round_rect_gdip(
                g,
                cx - ind_w * 0.5,
                by as f32 + layout.button_h as f32 - ind_h - 3.0 * s,
                ind_w,
                ind_h,
                ind_h * 0.5,
                Some(accent_argb),
                None,
            );
        }

        // Render active sub-tool variant icon when grouped
        let visual_action = match action {
            ToolbarAction::Tool(Tool::Rectangle) if active_tool.is_shape_group() => {
                ToolbarAction::Tool(active_tool)
            }
            ToolbarAction::Tool(Tool::Arrow) if active_tool.is_arrow_group() => {
                ToolbarAction::Tool(active_tool)
            }
            ToolbarAction::Tool(Tool::Pen) if active_tool.is_pen_group() => {
                ToolbarAction::Tool(active_tool)
            }
            ToolbarAction::Tool(Tool::Mosaic) if active_tool.is_effect_group() => {
                ToolbarAction::Tool(active_tool)
            }
            other => other,
        };
        draw_action_icon_on_graphics(g, visual_action, cx, cy, s, draw_col, scroll_paused);
    }

    if !div_pen.is_null() {
        GdipDeletePen(div_pen);
    }
    GdipDeleteGraphics(g);
}

/// SnowAir Secondary Property Bar (Variants + Filled/Dashed/Rounded + 4 Sizes + 7 Square Swatches)
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_secondary_bar_gdi(
    dc: HDC,
    layout: SecondaryBarLayout,
    active_tool: Tool,
    hovered: Option<SecondaryBarItem>,
    annotation_color: [u8; 4],
    stroke_width: u32,
    brush_size: u32,
    text_size: u32,
    stroke_style: StrokeStyle,
    shape_filled: bool,
    shape_rounded: bool,
    theme: &ThemePalette,
) {
    ensure_gdiplus();
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(dc, &mut g) != 0 || g.is_null() {
        return;
    }
    GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
    GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);

    let s = layout.scale();
    let outer_r = 11.0 * s;

    draw_soft_shadow_gdip(
        g,
        layout.left as f32,
        layout.top as f32,
        layout.width as f32,
        layout.height as f32,
        outer_r,
        s,
        theme.is_dark,
    );

    let border_argb = argb(theme.icon, Some(if theme.is_dark { 0x2A } else { 0x26 }));
    draw_round_rect_gdip(
        g,
        layout.left as f32,
        layout.top as f32,
        layout.width as f32,
        layout.height as f32,
        outer_r,
        Some(argb(theme.bg, Some(0xFA))),
        Some((border_argb, (1.0 * s).max(1.0))),
    );

    let icon_argb = argb(theme.icon, Some(0xFF));
    let accent_argb = argb(theme.accent, Some(0xFF));
    let hover_argb = argb(theme.hover, Some(0xFF));
    let active_bg_argb = argb(theme.accent, Some(if theme.is_dark { 0x26 } else { 0x1E }));
    let divider_argb = argb(theme.icon, Some(0x28));

    // 1. Sub-tool variants
    let variants = variants_for_tool(active_tool);
    for (i, &variant) in variants.iter().enumerate() {
        let Some((l, t, r, b)) = layout.variant_rect(i) else {
            continue;
        };
        let is_sel = variant == active_tool;
        let is_hov = hovered == Some(SecondaryBarItem::Variant(variant));
        if is_sel || is_hov {
            draw_round_rect_gdip(
                g,
                l as f32 + 2.0 * s,
                t as f32 + 1.0 * s,
                (r - l) as f32 - 4.0 * s,
                (b - t) as f32 - 2.0 * s,
                8.0 * s,
                Some(if is_sel { active_bg_argb } else { hover_argb }),
                None,
            );
        }
        draw_action_icon_on_graphics(
            g,
            ToolbarAction::Tool(variant),
            (l + r) as f32 * 0.5,
            (t + b) as f32 * 0.5,
            s * 0.92,
            if is_sel { accent_argb } else { icon_argb },
            false,
        );
    }

    // 2. Style toggles (Filled / Dashed / Rounded)
    if layout.has_styles {
        let items: Vec<(SecondaryBarItem, bool)> = if active_tool.is_shape_group() {
            vec![
                (SecondaryBarItem::ToggleFilled, shape_filled),
                (
                    SecondaryBarItem::ToggleDashed,
                    stroke_style == StrokeStyle::Dashed,
                ),
                (SecondaryBarItem::ToggleRounded, shape_rounded),
            ]
        } else {
            vec![(
                SecondaryBarItem::ToggleDashed,
                stroke_style == StrokeStyle::Dashed,
            )]
        };
        for (idx, (item, is_on)) in items.into_iter().enumerate() {
            let Some((l, t, r, b)) = layout.style_rect(idx) else {
                continue;
            };
            let is_hov = hovered == Some(item);
            if is_on || is_hov {
                draw_round_rect_gdip(
                    g,
                    l as f32 + 2.0 * s,
                    t as f32 + 1.0 * s,
                    (r - l) as f32 - 4.0 * s,
                    (b - t) as f32 - 2.0 * s,
                    7.5 * s,
                    Some(if is_on { active_bg_argb } else { hover_argb }),
                    None,
                );
            }
            let cx = (l + r) as f32 * 0.5;
            let cy = (t + b) as f32 * 0.5;
            let col = if is_on { accent_argb } else { icon_argb };
            let mut pen: *mut GpPen = null_mut();
            GdipCreatePen1(col, 1.8 * s, UnitPixel, &mut pen);
            if !pen.is_null() {
                GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapRound);
            }
            match item {
                SecondaryBarItem::ToggleFilled => {
                    draw_round_rect_gdip(
                        g,
                        cx - 7.0 * s,
                        cy - 7.0 * s,
                        14.0 * s,
                        14.0 * s,
                        3.0 * s,
                        Some(col),
                        None,
                    );
                }
                SecondaryBarItem::ToggleDashed => {
                    if !pen.is_null() {
                        GdipDrawLine(g, pen, cx - 7.5 * s, cy, cx - 3.5 * s, cy);
                        GdipDrawLine(g, pen, cx - 1.5 * s, cy, cx + 1.5 * s, cy);
                        GdipDrawLine(g, pen, cx + 3.5 * s, cy, cx + 7.5 * s, cy);
                    }
                }
                SecondaryBarItem::ToggleRounded => {
                    draw_round_rect_gdip(
                        g,
                        cx - 7.0 * s,
                        cy - 7.0 * s,
                        14.0 * s,
                        14.0 * s,
                        5.5 * s,
                        None,
                        Some((col, 1.8 * s)),
                    );
                }
                _ => {}
            }
            if !pen.is_null() {
                GdipDeletePen(pen);
            }
        }
    }

    // 3. Size dots (4 stepped circles)
    if layout.has_size {
        let selected_idx = if active_tool == Tool::Text {
            TEXT_SIZES
                .iter()
                .position(|&v| v == text_size)
                .unwrap_or(1)
        } else if matches!(active_tool, Tool::Mosaic | Tool::Patina) {
            MOSAIC_SIZES
                .iter()
                .position(|&v| v == brush_size)
                .unwrap_or(1)
        } else {
            STROKE_SIZES
                .iter()
                .position(|&v| v == stroke_width)
                .unwrap_or(1)
        };
        let radii = [2.6f32, 4.2, 5.8, 7.6];
        for (i, r_dp) in radii.into_iter().enumerate() {
            let Some((l, t, r, b)) = layout.size_rect(i) else {
                continue;
            };
            let is_sel = i == selected_idx;
            let is_hov = hovered == Some(SecondaryBarItem::Size(i));
            if is_sel || is_hov {
                draw_round_rect_gdip(
                    g,
                    l as f32 + 2.0 * s,
                    t as f32 + 2.0 * s,
                    (r - l) as f32 - 4.0 * s,
                    (b - t) as f32 - 4.0 * s,
                    7.5 * s,
                    Some(if is_sel { active_bg_argb } else { hover_argb }),
                    None,
                );
            }
            let cx = (l + r) as f32 * 0.5;
            let cy = (t + b) as f32 * 0.5;
            let rad = r_dp * s;
            let mut br: *mut GpSolidFill = null_mut();
            if GdipCreateSolidFill(if is_sel { accent_argb } else { icon_argb }, &mut br) == 0
                && !br.is_null()
            {
                GdipFillEllipse(
                    g,
                    br as *mut GpBrush,
                    cx - rad,
                    cy - rad,
                    rad * 2.0,
                    rad * 2.0,
                );
                GdipDeleteBrush(br as *mut GpBrush);
            }
        }
    }

    // 4. SnowAir 7 rounded-square color swatches (matches SnowAir screenshot!)
    if layout.has_color {
        for (i, &sw) in COLOR_SWATCHES.iter().enumerate() {
            let Some((l, t, r, b)) = layout.color_rect(i) else {
                continue;
            };
            let is_sel =
                sw[0] == annotation_color[0] && sw[1] == annotation_color[1] && sw[2] == annotation_color[2];
            let is_hov = hovered == Some(SecondaryBarItem::Color(i));
            let w = (r - l) as f32;
            let h = (b - t) as f32;
            if is_sel || is_hov {
                let ring = 2.5 * s;
                draw_round_rect_gdip(
                    g,
                    l as f32 - ring,
                    t as f32 - ring,
                    w + ring * 2.0,
                    h + ring * 2.0,
                    5.5 * s,
                    None,
                    Some((if is_sel { accent_argb } else { icon_argb }, 1.8 * s)),
                );
            }
            let sw_border = argb(theme.icon, Some(0x36));
            draw_round_rect_gdip(
                g,
                l as f32,
                t as f32,
                w,
                h,
                4.0 * s,
                Some(argb(sw, Some(0xFF))),
                Some((sw_border, (1.0 * s).max(1.0))),
            );
        }
    }

    // Section dividers
    let mut div_pen: *mut GpPen = null_mut();
    if GdipCreatePen1(divider_argb, (1.0 * s).max(1.0), UnitPixel, &mut div_pen) == 0
        && !div_pen.is_null()
    {
        for dx in layout.divider_xs() {
            let cy = layout.top as f32 + layout.height as f32 * 0.5;
            GdipDrawLine(g, div_pen, dx as f32, cy - 10.0 * s, dx as f32, cy + 10.0 * s);
        }
        GdipDeletePen(div_pen);
    }

    GdipDeleteGraphics(g);
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_toolbar_tooltip(
    dc: HDC,
    x: i32,
    y: i32,
    layout: ToolbarLayout,
    screen_w: i32,
    screen_h: i32,
    label: &str,
    index: usize,
    theme: &ThemePalette,
) {
    let Some((bx, by)) = toolbar_button_origin((x, y), index, layout) else {
        return;
    };
    draw_floating_tooltip(
        dc,
        bx + layout.button_w / 2,
        by,
        by + layout.button_h,
        screen_w,
        screen_h,
        label,
        layout.scale(),
        theme,
    );
}

pub(super) unsafe fn draw_secondary_bar_tooltip(
    dc: HDC,
    layout: SecondaryBarLayout,
    screen_w: i32,
    screen_h: i32,
    item: SecondaryBarItem,
    tool: Tool,
    theme: &ThemePalette,
) {
    let label = item.tooltip(tool);
    if label.is_empty() {
        return;
    }
    let anchor_x = match item {
        SecondaryBarItem::Variant(v) => variants_for_tool(tool)
            .iter()
            .position(|&x| x == v)
            .and_then(|i| layout.variant_rect(i))
            .map(|(l, _, r, _)| (l + r) / 2),
        SecondaryBarItem::ToggleFilled => layout.style_rect(0).map(|(l, _, r, _)| (l + r) / 2),
        SecondaryBarItem::ToggleDashed => layout
            .style_rect(if tool.is_shape_group() { 1 } else { 0 })
            .map(|(l, _, r, _)| (l + r) / 2),
        SecondaryBarItem::ToggleRounded => layout.style_rect(2).map(|(l, _, r, _)| (l + r) / 2),
        SecondaryBarItem::Size(i) => layout.size_rect(i).map(|(l, _, r, _)| (l + r) / 2),
        SecondaryBarItem::Color(i) => layout.color_rect(i).map(|(l, _, r, _)| (l + r) / 2),
    }
    .unwrap_or(layout.left + layout.width / 2);

    draw_floating_tooltip(
        dc,
        anchor_x,
        layout.top,
        layout.top + layout.height,
        screen_w,
        screen_h,
        label,
        layout.scale(),
        theme,
    );
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_floating_tooltip(
    dc: HDC,
    center_x: i32,
    anchor_top: i32,
    anchor_bottom: i32,
    screen_w: i32,
    screen_h: i32,
    label: &str,
    scale: f32,
    theme: &ThemePalette,
) {
    let w = ((label.chars().count() as f32 * 13.0 + 24.0) * scale)
        .round()
        .clamp(68.0 * scale, 220.0 * scale) as i32;
    let h = (28.0 * scale).round() as i32;
    let gap = (8.0 * scale).round() as i32;
    let left = (center_x - w / 2).clamp(6, (screen_w - w - 6).max(6));
    let top = if anchor_bottom + gap + h <= screen_h - 6 {
        anchor_bottom + gap
    } else {
        (anchor_top - h - gap).max(6)
    };

    ensure_gdiplus();
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(dc, &mut g) == 0 && !g.is_null() {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
        let border_argb = argb(theme.icon, Some(0x2A));
        draw_round_rect_gdip(
            g,
            left as f32,
            top as f32,
            w as f32,
            h as f32,
            7.0 * scale,
            Some(argb(theme.info_bg, Some(0xF5))),
            Some((border_argb, (1.0 * scale).max(1.0))),
        );
        GdipDeleteGraphics(g);
    }

    let font = create_ui_font(12.5, scale, false);
    let old_f = if !font.is_null() {
        SelectObject(dc, font as HGDIOBJ)
    } else {
        null_mut()
    };
    SetBkMode(dc, 1);
    SetTextColor(dc, ThemePalette::colorref(theme.info_text));
    let text = wide(label);
    let mut rc = RECT {
        left: left + 4,
        top,
        right: left + w - 4,
        bottom: top + h,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut rc, 0x1 | 0x4 | 0x20);
    if !old_f.is_null() {
        SelectObject(dc, old_f);
    }
    if !font.is_null() {
        DeleteObject(font as HGDIOBJ);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_toolbar_menu_gdi(
    dc: HDC,
    layout: ToolbarMenuLayout,
    menu: ToolbarMenu,
    scroll_axis: Option<MatchAxis>,
    hovered: Option<usize>,
    theme: &ThemePalette,
) {
    let scale = crate::theme::system_dpi_scale();
    ensure_gdiplus();
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(dc, &mut g) == 0 && !g.is_null() {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
        draw_soft_shadow_gdip(
            g,
            layout.left as f32,
            layout.top as f32,
            layout.width as f32,
            layout.height() as f32,
            11.0,
            scale,
            theme.is_dark,
        );
        let border_argb = argb(theme.icon, Some(0x2A));
        draw_round_rect_gdip(
            g,
            layout.left as f32,
            layout.top as f32,
            layout.width as f32,
            layout.height() as f32,
            11.0,
            Some(argb(theme.bg, Some(0xFA))),
            Some((border_argb, 1.0)),
        );
        GdipDeleteGraphics(g);
    }

    let text_c = ThemePalette::colorref(theme.info_text);
    let muted_c = ThemePalette::colorref(theme.info_muted);
    let accent_c = ThemePalette::colorref(theme.accent);

    let font = create_ui_font(13.5, 1.0, false);
    let old_f = if !font.is_null() {
        SelectObject(dc, font as HGDIOBJ)
    } else {
        null_mut()
    };
    SetBkMode(dc, 1);

    if layout.header_height > 0 {
        let title = match menu {
            ToolbarMenu::RecordingExport => "选择导出格式",
            _ => "选项",
        };
        SetTextColor(dc, muted_c);
        let t = wide(title);
        let mut hrc = RECT {
            left: layout.left + 14,
            top: layout.top + 8,
            right: layout.left + layout.width - 14,
            bottom: layout.top + layout.header_height,
        };
        DrawTextW(dc, t.as_ptr(), -1, &mut hrc, 0x4 | 0x20);
    }

    let items: &[&str] = match menu {
        ToolbarMenu::ScrollMode => &["纵向滚动长截图", "横向滚动长截图"],
        ToolbarMenu::Recognition => &["OCR 文字识别", "图片一键翻译", "识别二维码"],
        ToolbarMenu::RecordingExport => &["导出为 MP4 视频", "导出为 GIF 动图", "导出为 WebP 动图"],
        ToolbarMenu::DemoBg => &[
            "透明桌面（实时穿透）",
            "纯白白板背景",
            "深黑演播室背景",
            "护眼黑板绿背景",
            "加载图片 / PPT 幻灯片",
        ],
        ToolbarMenu::RecordingFps => &["开始录制", "取消"],
    };

    for (i, label) in items.iter().enumerate() {
        let Some((l, t, r, b)) = layout.item_rect(i) else {
            continue;
        };
        let is_hov = hovered == Some(i);
        let is_selected = match menu {
            ToolbarMenu::ScrollMode => {
                (i == 0 && scroll_axis == Some(MatchAxis::Vertical))
                    || (i == 1 && scroll_axis == Some(MatchAxis::Horizontal))
            }
            _ => false,
        };
        if is_hov || is_selected {
            let mut g2: *mut GpGraphics = null_mut();
            if GdipCreateFromHDC(dc, &mut g2) == 0 && !g2.is_null() {
                GdipSetSmoothingMode(g2, SmoothingModeAntiAlias8x8);
                draw_round_rect_gdip(
                    g2,
                    (l + 3) as f32,
                    (t + 2) as f32,
                    (r - l - 6) as f32,
                    (b - t - 4) as f32,
                    7.0,
                    Some(argb(theme.hover, Some(0xFF))),
                    None,
                );
                GdipDeleteGraphics(g2);
            }
        }
        SetTextColor(dc, if is_selected { accent_c } else { text_c });
        let txt = wide(label);
        let mut rc = RECT {
            left: l + 14,
            top: t,
            right: r - 14,
            bottom: b,
        };
        DrawTextW(dc, txt.as_ptr(), -1, &mut rc, 0x4 | 0x20);
    }

    if !old_f.is_null() {
        SelectObject(dc, old_f);
    }
    if !font.is_null() {
        DeleteObject(font as HGDIOBJ);
    }
}

pub(super) unsafe fn draw_recording_setup_gdi(
    dc: HDC,
    layout: RecordingSetupLayout,
    show_cursor: bool,
    theme: &ThemePalette,
) {
    let scale = crate::theme::system_dpi_scale();
    ensure_gdiplus();
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(dc, &mut g) == 0 && !g.is_null() {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
        draw_soft_shadow_gdip(
            g,
            layout.left as f32,
            layout.top as f32,
            layout.width as f32,
            layout.height as f32,
            14.0,
            scale,
            theme.is_dark,
        );
        draw_round_rect_gdip(
            g,
            layout.left as f32,
            layout.top as f32,
            layout.width as f32,
            layout.height as f32,
            14.0,
            Some(argb(theme.bg, Some(0xFA))),
            Some((argb(theme.icon, Some(0x28)), 1.0)),
        );
        let start = layout.start_rect();
        draw_round_rect_gdip(
            g,
            start.0 as f32,
            start.1 as f32,
            (start.2 - start.0) as f32,
            (start.3 - start.1) as f32,
            10.0,
            Some(argb(theme.accent, Some(0xFF))),
            None,
        );
        let mut sep_pen: *mut GpPen = null_mut();
        if GdipCreatePen1(argb(theme.icon, Some(0x22)), 1.0, UnitPixel, &mut sep_pen) == 0
            && !sep_pen.is_null()
        {
            GdipDrawLine(
                g,
                sep_pen,
                (layout.left + 16) as f32,
                (layout.top + 82) as f32,
                (layout.left + layout.width - 16) as f32,
                (layout.top + 82) as f32,
            );
            GdipDeletePen(sep_pen);
        }
        let controls_on = [true, true, true, show_cursor];
        for (index, on) in controls_on.into_iter().enumerate() {
            let rect = layout.control_rect(index).unwrap();
            let cx = (rect.0 + rect.2) as f32 * 0.5;
            let mut br: *mut GpSolidFill = null_mut();
            let col = if on {
                argb(theme.accent, Some(0xFF))
            } else {
                argb(theme.info_muted, Some(0x99))
            };
            if GdipCreateSolidFill(col, &mut br) == 0 && !br.is_null() {
                GdipFillEllipse(g, br as *mut GpBrush, cx - 5.5, (layout.top + 153) as f32, 11.0, 11.0);
                GdipDeleteBrush(br as *mut GpBrush);
            }
        }
        GdipDeleteGraphics(g);
    }

    let text_c = ThemePalette::colorref(theme.info_text);
    let muted_c = ThemePalette::colorref(theme.info_muted);
    let start = layout.start_rect();
    draw_setup_text(dc, start, "开始录制（支持录制期实时标注与鼠标穿透）", 0x00FF_FFFF, true, true);
    draw_setup_text(
        dc,
        (layout.left + 24, layout.top + 88, layout.left + 132, layout.top + 126),
        "导出格式",
        text_c,
        false,
        true,
    );
    draw_setup_text(
        dc,
        layout.output_note_rect(),
        "结束后支持 MP4 / GIF / WebP 动图导出",
        muted_c,
        false,
        false,
    );
    let controls = [
        ("实时标注", "录制期可画板标注"),
        ("鼠标穿透", "快捷键 ` 切换"),
        ("动图导出", "GIF / WebP / MP4"),
        ("鼠标光标", if show_cursor { "显示鼠标" } else { "隐藏鼠标" }),
    ];
    for (index, (label, detail)) in controls.into_iter().enumerate() {
        let rect = layout.control_rect(index).unwrap();
        draw_setup_text(dc, (rect.0, layout.top + 174, rect.2, layout.top + 196), label, text_c, true, true);
        draw_setup_text(dc, (rect.0, layout.top + 196, rect.2, layout.top + 216), detail, muted_c, true, false);
    }
    draw_setup_text(dc, layout.close_rect(), "×", muted_c, true, true);
}

unsafe fn draw_setup_text(
    dc: HDC,
    rect: (i32, i32, i32, i32),
    label: &str,
    color: u32,
    centered: bool,
    medium: bool,
) {
    let font = create_ui_font(13.5, 1.0, medium);
    let old_font = if !font.is_null() {
        SelectObject(dc, font as HGDIOBJ)
    } else {
        null_mut()
    };
    SetBkMode(dc, 1);
    SetTextColor(dc, color);
    let text = wide(label);
    let mut target = RECT {
        left: rect.0 + 2,
        top: rect.1,
        right: rect.2 - 2,
        bottom: rect.3,
    };
    DrawTextW(
        dc,
        text.as_ptr(),
        -1,
        &mut target,
        if centered { 1 | 4 | 0x20 } else { 4 | 0x20 },
    );
    if !old_font.is_null() {
        SelectObject(dc, old_font);
    }
    if !font.is_null() {
        DeleteObject(font as HGDIOBJ);
    }
}

// -------------------------------------------------------------------------
// SnowAir-Grade Uniform DPI-Scaled Vector Icon Renderer
// Coordinate space: logical [-9.0..+9.0] multiplied by `s`, rounded caps/joins
// Synchronized 1:1 with `ui/src/components/ToolbarIcon.vue`!
// -------------------------------------------------------------------------

unsafe fn draw_action_icon_on_graphics(
    g: *mut GpGraphics,
    action: ToolbarAction,
    cx: f32,
    cy: f32,
    s: f32,
    argb_color: u32,
    scroll_paused: bool,
) {
    let mut pen: *mut GpPen = null_mut();
    let mut fill: *mut GpSolidFill = null_mut();
    let stroke_w = (1.85 * s).max(1.5);
    GdipCreatePen1(argb_color, stroke_w, UnitPixel, &mut pen);
    GdipCreateSolidFill(argb_color, &mut fill);
    if !pen.is_null() {
        GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapRound);
        GdipSetPenLineJoin(pen, LineJoinRound);
    }

    let line = |x1: f32, y1: f32, x2: f32, y2: f32| {
        GdipDrawLine(g, pen, cx + x1 * s, cy + y1 * s, cx + x2 * s, cy + y2 * s);
    };
    let dot = |dx: f32, dy: f32, r: f32| {
        let rr = r * s;
        GdipFillEllipse(
            g,
            fill as *mut GpBrush,
            cx + dx * s - rr,
            cy + dy * s - rr,
            rr * 2.0,
            rr * 2.0,
        );
    };
    let round_box = |rx: f32, ry: f32, rw: f32, rh: f32, rad: f32| {
        draw_round_rect_gdip(
            g,
            cx + rx * s,
            cy + ry * s,
            rw * s,
            rh * s,
            rad * s,
            None,
            Some((argb_color, stroke_w)),
        );
    };

    match action {
        ToolbarAction::MoveToolbar => {
            for row in [-5.5f32, 0.0, 5.5] {
                dot(-2.6, row, 1.45);
                dot(2.6, row, 1.45);
            }
        }
        ToolbarAction::Tool(Tool::Select) | ToolbarAction::DemoPassThrough => {
            // Sleek SnowAir tilted cursor pointer (closed crisp arrow)
            line(-5.5, -7.2, 6.2, 0.2);
            line(6.2, 0.2, 0.6, 1.8);
            line(0.6, 1.8, -1.6, 7.2);
            line(-1.6, 7.2, -5.5, -7.2);
        }
        ToolbarAction::Tool(Tool::Rectangle) => {
            round_box(-7.8, -6.6, 15.6, 13.2, 2.8);
        }
        ToolbarAction::Tool(Tool::Ellipse) => {
            GdipDrawEllipse(
                g,
                pen,
                cx - 8.0 * s,
                cy - 6.8 * s,
                16.0 * s,
                13.6 * s,
            );
        }
        ToolbarAction::Tool(Tool::Triangle) => {
            line(0.0, -7.5, 8.0, 6.8);
            line(8.0, 6.8, -8.0, 6.8);
            line(-8.0, 6.8, 0.0, -7.5);
        }
        ToolbarAction::Tool(Tool::Star) => {
            let mut prev = (0.0f32, 0.0f32);
            let mut first = (0.0f32, 0.0f32);
            for i in 0..10 {
                let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                let r = if i % 2 == 0 { 8.4 } else { 3.7 };
                let p = (r * a.cos(), r * a.sin());
                if i == 0 {
                    first = p;
                } else {
                    line(prev.0, prev.1, p.0, p.1);
                }
                prev = p;
            }
            line(prev.0, prev.1, first.0, first.1);
        }
        ToolbarAction::Tool(Tool::Bubble) => {
            line(-7.5, -6.5, 7.5, -6.5);
            line(7.5, -6.5, 7.5, 3.2);
            line(7.5, 3.2, 0.2, 3.2);
            line(0.2, 3.2, -4.2, 7.8);
            line(-4.2, 7.8, -3.6, 3.2);
            line(-3.6, 3.2, -7.5, 3.2);
            line(-7.5, 3.2, -7.5, -6.5);
        }
        ToolbarAction::Tool(Tool::Burst) => {
            let mut prev = (0.0f32, 0.0f32);
            let mut first = (0.0f32, 0.0f32);
            for i in 0..12 {
                let a = i as f32 * std::f32::consts::TAU / 12.0;
                let r = if i % 2 == 0 { 8.4 } else { 5.0 };
                let p = (r * a.cos(), r * a.sin());
                if i == 0 {
                    first = p;
                } else {
                    line(prev.0, prev.1, p.0, p.1);
                }
                prev = p;
            }
            line(prev.0, prev.1, first.0, first.1);
        }
        ToolbarAction::Tool(Tool::Arrow) => {
            line(-6.6, 6.6, 6.6, -6.6);
            line(0.8, -6.6, 6.6, -6.6);
            line(6.6, -6.6, 6.6, -0.8);
        }
        ToolbarAction::Tool(Tool::DoubleArrow) => {
            line(-7.5, 0.0, 7.5, 0.0);
            line(-7.5, 0.0, -3.8, -3.8);
            line(-7.5, 0.0, -3.8, 3.8);
            line(7.5, 0.0, 3.8, -3.8);
            line(7.5, 0.0, 3.8, 3.8);
        }
        ToolbarAction::Tool(Tool::CurvedArrow) => {
            GdipDrawBezier(
                g,
                pen,
                cx - 7.0 * s,
                cy + 6.0 * s,
                cx - 6.5 * s,
                cy - 5.8 * s,
                cx + 2.2 * s,
                cy - 7.0 * s,
                cx + 7.0 * s,
                cy - 3.6 * s,
            );
            line(7.0, -3.6, 2.6, -5.8);
            line(7.0, -3.6, 4.8, 0.8);
        }
        ToolbarAction::Tool(Tool::Line) => {
            line(-6.8, 6.8, 6.8, -6.8);
        }
        ToolbarAction::Tool(Tool::Dimension) => {
            line(-7.5, 0.0, 7.5, 0.0);
            line(-7.5, -4.8, -7.5, 4.8);
            line(7.5, -4.8, 7.5, 4.8);
        }
        ToolbarAction::Tool(Tool::Angle) => {
            line(-6.5, 6.5, 7.5, 6.5);
            line(-6.5, 6.5, 3.2, -6.8);
            GdipDrawArc(g, pen, cx - 12.5 * s, cy + 0.5 * s, 12.0 * s, 12.0 * s, -52.0, 52.0);
        }
        ToolbarAction::Tool(Tool::Wave) => {
            GdipDrawBezier(
                g,
                pen,
                cx - 8.0 * s,
                cy + 1.0 * s,
                cx - 3.8 * s,
                cy - 6.5 * s,
                cx + 3.8 * s,
                cy + 7.5 * s,
                cx + 8.0 * s,
                cy - 1.0 * s,
            );
        }
        ToolbarAction::Tool(Tool::Pen) => {
            line(-6.8, 7.0, -4.5, 1.8);
            line(-4.5, 1.8, 4.2, -7.0);
            line(4.2, -7.0, 7.2, -4.0);
            line(7.2, -4.0, -1.6, 4.8);
            line(-1.6, 4.8, -6.8, 7.0);
            line(1.8, -4.6, 4.8, -1.6);
        }
        ToolbarAction::Tool(Tool::Highlight) => {
            line(-6.8, 5.8, 2.8, -6.4);
            line(-1.4, 7.0, 7.0, -3.2);
            line(-6.8, 5.8, -1.4, 7.0);
            line(2.8, -6.4, 7.0, -3.2);
        }
        ToolbarAction::Tool(Tool::FadePen) => {
            line(1.6, -8.0, -5.4, 1.0);
            line(-5.4, 1.0, 0.5, 1.0);
            line(0.5, 1.0, -1.6, 8.0);
            line(-1.6, 8.0, 5.4, -1.0);
            line(5.4, -1.0, -0.5, -1.0);
            line(-0.5, -1.0, 1.6, -8.0);
        }
        ToolbarAction::Tool(Tool::Text) => {
            round_box(-7.5, -7.5, 15.0, 15.0, 3.0);
            line(-3.8, -3.2, 3.8, -3.2);
            line(0.0, -3.2, 0.0, 4.4);
        }
        ToolbarAction::Tool(Tool::Number) | ToolbarAction::PlaceNumber => {
            GdipDrawEllipse(g, pen, cx - 8.0 * s, cy - 8.0 * s, 16.0 * s, 16.0 * s);
            line(0.2, -4.0, 0.2, 4.2);
            line(-2.2, -1.8, 0.2, -4.0);
            line(-2.4, 4.2, 2.6, 4.2);
        }
        ToolbarAction::Tool(Tool::Mosaic) => {
            round_box(-7.5, -7.5, 15.0, 15.0, 2.8);
            for gy in 0..3 {
                for gx in 0..3 {
                    if (gx + gy) % 2 == 0 {
                        GdipFillRectangle(
                            g,
                            fill as *mut GpBrush,
                            cx + (-5.6 + gx as f32 * 3.8) * s,
                            cy + (-5.6 + gy as f32 * 3.8) * s,
                            3.3 * s,
                            3.3 * s,
                        );
                    }
                }
            }
        }
        ToolbarAction::Tool(Tool::Patina) => {
            GdipDrawEllipse(g, pen, cx - 8.0 * s, cy - 8.0 * s, 16.0 * s, 16.0 * s);
            line(0.0, 1.2, 4.4, -3.8);
            dot(0.0, 1.6, 1.6);
        }
        ToolbarAction::Tool(Tool::Watermark) => {
            line(-6.4, 0.0, 0.0, -6.4);
            line(-5.4, 5.8, 5.8, -5.4);
            line(0.4, 6.4, 6.4, 0.4);
        }
        ToolbarAction::Tool(Tool::Spotlight) => {
            GdipDrawEllipse(g, pen, cx - 8.0 * s, cy - 8.0 * s, 16.0 * s, 16.0 * s);
            line(0.0, -8.0, 0.0, 8.0);
            line(0.0, -3.8, 6.2, -3.8);
            line(0.0, 0.0, 7.6, 0.0);
            line(0.0, 3.8, 6.2, 3.8);
        }
        ToolbarAction::Tool(Tool::Eraser) => {
            line(-7.0, 2.0, 1.0, -6.4);
            line(1.0, -6.4, 7.4, 0.0);
            line(7.4, 0.0, 1.2, 6.8);
            line(1.2, 6.8, -3.8, 6.8);
            line(-3.8, 6.8, -7.0, 2.0);
            line(-2.6, -2.0, 3.6, 4.2);
            line(1.2, 6.8, 7.2, 6.8);
        }
        ToolbarAction::Undo => {
            line(-6.8, -2.6, -2.6, -6.8);
            line(-6.8, -2.6, -2.6, 1.6);
            line(-6.8, -2.6, 2.4, -2.6);
            GdipDrawArc(g, pen, cx - 2.2 * s, cy - 2.6 * s, 9.6 * s, 9.6 * s, -90.0, 180.0);
            line(-2.4, 7.0, 2.6, 7.0);
        }
        ToolbarAction::Redo => {
            line(6.8, -2.6, 2.6, -6.8);
            line(6.8, -2.6, 2.6, 1.6);
            line(6.8, -2.6, -2.4, -2.6);
            GdipDrawArc(g, pen, cx - 7.4 * s, cy - 2.6 * s, 9.6 * s, 9.6 * s, 90.0, 180.0);
            line(-2.6, 7.0, 2.4, 7.0);
        }
        ToolbarAction::Record => {
            // SnowAir viewfinder brackets + solid center dot
            line(-8.0, -3.6, -8.0, -8.0);
            line(-8.0, -8.0, -3.6, -8.0);
            line(3.6, -8.0, 8.0, -8.0);
            line(8.0, -8.0, 8.0, -3.6);
            line(-8.0, 3.6, -8.0, 8.0);
            line(-8.0, 8.0, -3.6, 8.0);
            line(3.6, 8.0, 8.0, 8.0);
            line(8.0, 8.0, 8.0, 3.6);
            dot(0.0, 0.0, 3.1);
        }
        ToolbarAction::Pin => {
            line(-0.8, -7.2, 7.2, 0.8);
            line(0.8, -5.6, -3.4, -0.4);
            line(5.6, -0.8, 0.4, 3.4);
            line(-5.2, -1.2, 1.2, 5.2);
            line(-2.4, 2.4, -7.4, 7.4);
        }
        ToolbarAction::Ocr => {
            line(-8.0, -3.6, -8.0, -8.0);
            line(-8.0, -8.0, -3.6, -8.0);
            line(3.6, -8.0, 8.0, -8.0);
            line(8.0, -8.0, 8.0, -3.6);
            line(-8.0, 3.6, -8.0, 8.0);
            line(-8.0, 8.0, -3.6, 8.0);
            line(3.6, 8.0, 8.0, 8.0);
            line(8.0, 8.0, 8.0, 3.6);
            line(-4.2, -2.6, 4.2, -2.6);
            line(-4.2, 0.6, 4.2, 0.6);
            line(-4.2, 3.8, 1.6, 3.8);
        }
        ToolbarAction::Translate => {
            line(-8.0, -4.8, -0.6, -4.8);
            line(-4.3, -7.6, -4.3, -4.8);
            line(-2.2, -4.8, -7.0, 2.6);
            line(-6.4, -3.0, -1.6, 1.6);
            line(1.2, 7.0, 4.8, -1.6);
            line(4.8, -1.6, 8.2, 7.0);
            line(2.4, 4.2, 7.0, 4.2);
        }
        ToolbarAction::Qr => {
            GdipDrawRectangle(g, pen, cx - 7.5 * s, cy - 7.5 * s, 5.8 * s, 5.8 * s);
            GdipDrawRectangle(g, pen, cx + 1.7 * s, cy - 7.5 * s, 5.8 * s, 5.8 * s);
            GdipDrawRectangle(g, pen, cx - 7.5 * s, cy + 1.7 * s, 5.8 * s, 5.8 * s);
            dot(3.2, 3.2, 1.5);
            dot(6.4, 6.4, 1.5);
        }
        ToolbarAction::Scroll => {
            round_box(-6.4, -8.0, 12.8, 16.0, 2.6);
            line(0.0, -4.4, 0.0, 4.4);
            line(-2.8, -1.6, 0.0, -4.4);
            line(2.8, -1.6, 0.0, -4.4);
            line(-2.8, 1.6, 0.0, 4.4);
            line(2.8, 1.6, 0.0, 4.4);
        }
        ToolbarAction::ScrollPauseToggle => {
            round_box(-7.5, -7.0, 15.0, 14.0, 2.8);
            if scroll_paused {
                line(-2.2, -3.4, 3.8, 0.0);
                line(3.8, 0.0, -2.2, 3.4);
                line(-2.2, 3.4, -2.2, -3.4);
            } else {
                line(-2.4, -3.4, -2.4, 3.4);
                line(2.4, -3.4, 2.4, 3.4);
            }
        }
        ToolbarAction::Edit => {
            round_box(-7.5, -7.0, 15.0, 14.0, 2.8);
            line(-1.0, -7.0, -1.0, 7.0);
            line(-1.0, 0.0, 7.5, 0.0);
        }
        ToolbarAction::Copy => {
            round_box(-6.0, -3.6, 9.6, 10.6, 2.0);
            line(-2.6, -7.0, 6.4, -7.0);
            line(6.4, -7.0, 6.4, 3.6);
        }
        ToolbarAction::Save => {
            line(0.0, -7.6, 0.0, 2.6);
            line(-3.8, -1.2, 0.0, 2.6);
            line(3.8, -1.2, 0.0, 2.6);
            line(-7.2, 2.6, -7.2, 7.0);
            line(-7.2, 7.0, 7.2, 7.0);
            line(7.2, 7.0, 7.2, 2.6);
        }
        ToolbarAction::Cancel => {
            line(-6.0, -6.0, 6.0, 6.0);
            line(6.0, -6.0, -6.0, 6.0);
        }
        ToolbarAction::Finish => {
            line(-7.0, 0.4, -1.6, 6.0);
            line(-1.6, 6.0, 7.6, -5.0);
        }
        ToolbarAction::DemoBackground => {
            round_box(-8.0, -6.4, 16.0, 11.4, 2.2);
            line(-4.4, 7.6, 4.4, 7.6);
            line(0.0, 5.0, 0.0, 7.6);
        }
        ToolbarAction::DemoClear => {
            GdipDrawArc(g, pen, cx - 7.0 * s, cy - 7.0 * s, 14.0 * s, 14.0 * s, -45.0, 290.0);
            line(3.8, -6.6, 7.0, -2.8);
            line(7.0, -2.8, 2.6, -1.6);
        }
    }

    if !fill.is_null() {
        GdipDeleteBrush(fill as *mut GpBrush);
    }
    if !pen.is_null() {
        GdipDeletePen(pen);
    }
}
