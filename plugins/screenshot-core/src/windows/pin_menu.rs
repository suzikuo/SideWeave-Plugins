//! SnowAir-grade GDI+ anti-aliased custom context menu for desktop Pinned Image windows.
//! Supports Dark/Light dual theme, checkmarks, inline pill rows (Flip / Opacity / Scale),
//! hover highlights, DPI auto-scaling, and instant click-outside / Esc dismissal.

use super::toolbar_paint::{argb, create_ui_font, draw_round_rect_gdip, ensure_gdiplus};
use super::{wide, UiState};

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateRoundRectRgn,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, InvalidateRect,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, HGDIOBJ, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    GdipCreateFromHDC, GdipCreatePen1, GdipDeleteGraphics, GdipDeletePen, GdipDrawLine,
    GdipSetPixelOffsetMode, GdipSetSmoothingMode, GpGraphics, GpPen, PixelOffsetModeHalf,
    SmoothingModeAntiAlias8x8, UnitPixel,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetSystemMetrics,
    LoadCursorW, SetCursor, SetForegroundWindow, ShowWindow, IDC_HAND, SW_SHOW, WM_ACTIVATE,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONUP, WM_SETCURSOR, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PinMenuAction {
    Copy,
    Save,
    Ocr,
    Rotate90,
    FlipH,
    FlipV,
    ToggleThumbnail,
    ToggleTopmost,
    ToggleBorder,
    Opacity(u8),
    Scale(u32), // 50, 100, 150
    Close,
}

#[derive(Clone)]
struct MenuHitItem {
    action: PinMenuAction,
    rect: (i32, i32, i32, i32),
    label: String,
    checked: bool,
    danger: bool,
    is_pill: bool,
    active_pill: bool,
}

struct MenuLayout {
    scale: f32,
    width: i32,
    height: i32,
    items: Vec<MenuHitItem>,
    dividers_y: Vec<i32>,
    row_labels: Vec<((i32, i32, i32, i32), &'static str)>,
}

pub(crate) struct PinMenuState {
    pub(crate) hwnd: HWND,
    pub(crate) target_pin_hwnd: HWND,
    pub(crate) hover_idx: Option<usize>,
    pub(crate) tracking: bool,
}

fn compute_menu_layout(
    is_thumb: bool,
    topmost: bool,
    show_border: bool,
    opacity: u8,
) -> MenuLayout {
    let s = crate::theme::system_dpi_scale();
    let px = |dp: f32| (dp * s).round() as i32;

    let width = px(228.0);
    let pad_x = px(6.0);
    let mut y = px(7.0);
    let row_h = px(30.0);
    let pill_row_h = px(28.0);
    let gap = px(2.0);
    let div_pad = px(5.0);

    let mut items = Vec::new();
    let mut dividers_y = Vec::new();
    let mut row_labels = Vec::new();

    let push_row = |items: &mut Vec<MenuHitItem>,
                    y: &mut i32,
                    action: PinMenuAction,
                    label: &str,
                    checked: bool,
                    danger: bool| {
        items.push(MenuHitItem {
            action,
            rect: (pad_x, *y, width - pad_x, *y + row_h),
            label: label.to_string(),
            checked,
            danger,
            is_pill: false,
            active_pill: false,
        });
        *y += row_h + gap;
    };

    push_row(&mut items, &mut y, PinMenuAction::Copy, "复制图片", false, false);
    push_row(&mut items, &mut y, PinMenuAction::Save, "另存为…", false, false);
    push_row(&mut items, &mut y, PinMenuAction::Ocr, "文字识别 (OCR)", false, false);

    dividers_y.push(y + div_pad / 2);
    y += div_pad + 1;

    push_row(&mut items, &mut y, PinMenuAction::Rotate90, "顺时针旋转 90°", false, false);
    // Inline Flip H / Flip V pills
    {
        let pill_gap = px(6.0);
        let half_w = (width - pad_x * 2 - pill_gap) / 2;
        items.push(MenuHitItem {
            action: PinMenuAction::FlipH,
            rect: (pad_x, y, pad_x + half_w, y + pill_row_h),
            label: "水平翻转".into(),
            checked: false,
            danger: false,
            is_pill: true,
            active_pill: false,
        });
        items.push(MenuHitItem {
            action: PinMenuAction::FlipV,
            rect: (pad_x + half_w + pill_gap, y, width - pad_x, y + pill_row_h),
            label: "垂直翻转".into(),
            checked: false,
            danger: false,
            is_pill: true,
            active_pill: false,
        });
        y += pill_row_h + gap + px(2.0);
    }

    dividers_y.push(y + div_pad / 2);
    y += div_pad + 1;

    push_row(&mut items, &mut y, PinMenuAction::ToggleThumbnail, "缩略图模式", is_thumb, false);
    push_row(&mut items, &mut y, PinMenuAction::ToggleTopmost, "窗口置顶", topmost, false);
    push_row(&mut items, &mut y, PinMenuAction::ToggleBorder, "显示边框", show_border, false);

    dividers_y.push(y + div_pad / 2);
    y += div_pad + 1;

    // Inline Opacity pills: 25% / 50% / 75% / 100%
    {
        let lbl_h = px(18.0);
        row_labels.push(((pad_x + px(8.0), y, width - pad_x, y + lbl_h), "不透明度"));
        y += lbl_h + px(2.0);
        let presets: [(u8, &str); 4] = [(64, "25%"), (128, "50%"), (191, "75%"), (255, "100%")];
        let p_gap = px(4.0);
        let p_w = (width - pad_x * 2 - p_gap * 3) / 4;
        for (i, (val, lbl)) in presets.iter().enumerate() {
            let l = pad_x + i as i32 * (p_w + p_gap);
            let r = if i == 3 { width - pad_x } else { l + p_w };
            let active = (i32::from(opacity) - i32::from(*val)).abs() <= 16;
            items.push(MenuHitItem {
                action: PinMenuAction::Opacity(*val),
                rect: (l, y, r, y + px(26.0)),
                label: (*lbl).into(),
                checked: false,
                danger: false,
                is_pill: true,
                active_pill: active,
            });
        }
        y += px(26.0) + px(6.0);
    }

    // Inline Scale pills: 50% / 100% / 150%
    {
        let lbl_h = px(18.0);
        row_labels.push(((pad_x + px(8.0), y, width - pad_x, y + lbl_h), "缩放比例"));
        y += lbl_h + px(2.0);
        let presets: [(u32, &str); 3] = [(50, "50%"), (100, "100%"), (150, "150%")];
        let p_gap = px(5.0);
        let p_w = (width - pad_x * 2 - p_gap * 2) / 3;
        for (i, (val, lbl)) in presets.iter().enumerate() {
            let l = pad_x + i as i32 * (p_w + p_gap);
            let r = if i == 2 { width - pad_x } else { l + p_w };
            items.push(MenuHitItem {
                action: PinMenuAction::Scale(*val),
                rect: (l, y, r, y + px(26.0)),
                label: (*lbl).into(),
                checked: false,
                danger: false,
                is_pill: true,
                active_pill: false,
            });
        }
        y += px(26.0) + px(4.0);
    }

    dividers_y.push(y + div_pad / 2);
    y += div_pad + 1;

    push_row(&mut items, &mut y, PinMenuAction::Close, "关闭贴图 (Esc)", false, true);

    let height = y + px(5.0);
    MenuLayout {
        scale: s,
        width,
        height,
        items,
        dividers_y,
        row_labels,
    }
}

pub(super) fn destroy_pin_menu(state: &mut UiState) {
    if let Some(menu) = state.pin_menu.take() {
        if !menu.hwnd.is_null() {
            unsafe { DestroyWindow(menu.hwnd) };
        }
    }
}

pub(super) fn open_pin_menu(state: &mut UiState, pin_index: usize) {
    destroy_pin_menu(state);
    let Some(pin) = state.pinned_images.get(pin_index) else {
        return;
    };
    let target_pin_hwnd = pin.hwnd;
    let layout = compute_menu_layout(pin.thumbnail_mode, pin.topmost, pin.show_border, pin.opacity);

    unsafe {
        let mut pt: POINT = zeroed();
        GetCursorPos(&mut pt);
        let vx = GetSystemMetrics(76);
        let vy = GetSystemMetrics(77);
        let vw = GetSystemMetrics(78).max(640);
        let vh = GetSystemMetrics(79).max(480);
        let x = pt.x.clamp(vx + 6, (vx + vw - layout.width - 6).max(vx + 6));
        let y = pt.y.clamp(vy + 6, (vy + vh - layout.height - 6).max(vy + 6));

        let class_name = wide("AgileTilesScreenshotCoreV2");
        let instance = GetModuleHandleW(null());
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            wide("PinMenu").as_ptr(),
            WS_POPUP,
            x,
            y,
            layout.width,
            layout.height,
            target_pin_hwnd,
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        if hwnd.is_null() {
            return;
        }
        let rad = (12.0 * layout.scale).round() as i32;
        let rgn = CreateRoundRectRgn(0, 0, layout.width + 1, layout.height + 1, rad * 2, rad * 2);
        if !rgn.is_null() {
            SetWindowRgn(hwnd, rgn, 1);
        }
        state.pin_menu = Some(PinMenuState {
            hwnd,
            target_pin_hwnd,
            hover_idx: None,
            tracking: false,
        });
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        InvalidateRect(hwnd, null(), 0);
    }
}

pub(super) unsafe fn pin_menu_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state: &mut UiState,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_pin_menu(hwnd, state);
            0
        }
        WM_SETCURSOR => {
            let cursor = LoadCursorW(null_mut(), IDC_HAND);
            if !cursor.is_null() {
                SetCursor(cursor);
            }
            1
        }
        WM_MOUSEMOVE => {
            if let Some(menu) = state.pin_menu.as_mut() {
                if !menu.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    if TrackMouseEvent(&mut tme) != 0 {
                        menu.tracking = true;
                    }
                }
            }
            let pt = (
                (lparam & 0xffff) as i16 as i32,
                ((lparam >> 16) & 0xffff) as i16 as i32,
            );
            let next_hover = current_layout(state).and_then(|layout| {
                layout
                    .items
                    .iter()
                    .position(|it| inside(pt, it.rect))
            });
            if let Some(menu) = state.pin_menu.as_mut() {
                if menu.hover_idx != next_hover {
                    menu.hover_idx = next_hover;
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_MOUSELEAVE => {
            if let Some(menu) = state.pin_menu.as_mut() {
                menu.tracking = false;
                if menu.hover_idx.take().is_some() {
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let pt = (
                (lparam & 0xffff) as i16 as i32,
                ((lparam >> 16) & 0xffff) as i16 as i32,
            );
            let target_hwnd = state.pin_menu.as_ref().map(|m| m.target_pin_hwnd);
            let picked = current_layout(state).and_then(|layout| {
                layout
                    .items
                    .iter()
                    .find(|it| inside(pt, it.rect))
                    .map(|it| it.action)
            });
            destroy_pin_menu(state);
            if let (Some(pin_hwnd), Some(action)) = (target_hwnd, picked) {
                super::output::execute_pin_menu_action(state, pin_hwnd, action);
            }
            0
        }
        WM_KEYDOWN if wparam as u32 == 0x1b => {
            destroy_pin_menu(state);
            0
        }
        WM_ACTIVATE => {
            if (wparam & 0xffff) == 0 {
                destroy_pin_menu(state);
            }
            0
        }
        WM_KILLFOCUS => {
            destroy_pin_menu(state);
            0
        }
        WM_DESTROY => {
            if state
                .pin_menu
                .as_ref()
                .is_some_and(|m| m.hwnd == hwnd)
            {
                state.pin_menu = None;
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn current_layout(state: &UiState) -> Option<MenuLayout> {
    let target = state.pin_menu.as_ref()?.target_pin_hwnd;
    let pin = state.pinned_images.iter().find(|p| p.hwnd == target)?;
    Some(compute_menu_layout(
        pin.thumbnail_mode,
        pin.topmost,
        pin.show_border,
        pin.opacity,
    ))
}

fn inside(pt: (i32, i32), r: (i32, i32, i32, i32)) -> bool {
    pt.0 >= r.0 && pt.0 < r.2 && pt.1 >= r.1 && pt.1 < r.3
}

unsafe fn paint_pin_menu(hwnd: HWND, state: &UiState) {
    let mut ps: PAINTSTRUCT = zeroed();
    let target_dc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let w = (rc.right - rc.left).max(1);
    let h = (rc.bottom - rc.top).max(1);

    let mem_dc = CreateCompatibleDC(target_dc);
    let bmp = CreateCompatibleBitmap(target_dc, w, h);
    let use_buf = !mem_dc.is_null() && !bmp.is_null();
    let old_bmp = if use_buf {
        SelectObject(mem_dc, bmp as HGDIOBJ)
    } else {
        null_mut()
    };
    let dc = if use_buf { mem_dc } else { target_dc };

    if let Some(layout) = current_layout(state) {
        let theme = state.current_theme();
        let hover_idx = state.pin_menu.as_ref().and_then(|m| m.hover_idx);
        let s = layout.scale;

        let bg_colorref = if theme.is_dark { 0x0022_1E1E } else { 0x00FF_FFFF };
        let bg_argb = if theme.is_dark { 0xFF_1E1E22 } else { 0xFF_FFFFFF };
        let border_argb = if theme.is_dark { 0xFF_383840 } else { 0xFF_DCDFE6 };
        let div_argb = if theme.is_dark { 0xFF_2F2F37 } else { 0xFF_EAECEF };
        let pill_bg = if theme.is_dark { 0xFF_2A2A32 } else { 0xFF_F2F3F7 };
        let pill_hover = if theme.is_dark { 0xFF_363640 } else { 0xFF_E6E8EF };
        let row_hover = argb(theme.accent, Some(if theme.is_dark { 0x2E } else { 0x20 }));
        let danger_hover = if theme.is_dark { 0x35_EF4444 } else { 0x20_EF4444 };
        let active_pill_bg = argb(theme.accent, Some(if theme.is_dark { 0x38 } else { 0x24 }));
        let active_pill_border = argb(theme.accent, Some(0xAA));

        let text_primary = if theme.is_dark { 0x00F5_F5F7 } else { 0x0024_201E };
        let text_muted = if theme.is_dark { 0x009E_9AA0 } else { 0x0078_7470 };
        let text_accent = if theme.is_dark { 0x006A_DA4C } else { 0x003E_9916 };
        let text_danger = if theme.is_dark { 0x006B_6BFF } else { 0x003B_3BD9 };

        let bg_br = CreateSolidBrush(bg_colorref);
        FillRect(dc, &rc, bg_br);
        DeleteObject(bg_br as HGDIOBJ);

        ensure_gdiplus();
        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(dc, &mut g) == 0 && !g.is_null() {
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
            GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);

            // Outer rounded border
            draw_round_rect_gdip(
                g,
                0.5,
                0.5,
                w as f32 - 1.0,
                h as f32 - 1.0,
                12.0 * s,
                Some(bg_argb),
                Some((border_argb, (1.0 * s).max(1.0))),
            );

            // Dividers
            let mut div_pen: *mut GpPen = null_mut();
            if GdipCreatePen1(div_argb, (1.0 * s).max(1.0), UnitPixel, &mut div_pen) == 0
                && !div_pen.is_null()
            {
                for &dy in &layout.dividers_y {
                    GdipDrawLine(
                        g,
                        div_pen,
                        10.0 * s,
                        dy as f32,
                        w as f32 - 10.0 * s,
                        dy as f32,
                    );
                }
                GdipDeletePen(div_pen);
            }

            // Item backgrounds
            for (idx, item) in layout.items.iter().enumerate() {
                let hovered = hover_idx == Some(idx);
                let (l, t, r, b) = item.rect;
                if item.is_pill {
                    let fill = if item.active_pill {
                        active_pill_bg
                    } else if hovered {
                        pill_hover
                    } else {
                        pill_bg
                    };
                    let stroke = if item.active_pill {
                        Some((active_pill_border, (1.0 * s).max(1.0)))
                    } else if hovered {
                        Some((argb(theme.accent, Some(0x70)), (1.0 * s).max(1.0)))
                    } else {
                        None
                    };
                    draw_round_rect_gdip(
                        g,
                        l as f32,
                        t as f32,
                        (r - l) as f32,
                        (b - t) as f32,
                        7.0 * s,
                        Some(fill),
                        stroke,
                    );
                } else if hovered {
                    draw_round_rect_gdip(
                        g,
                        l as f32,
                        t as f32,
                        (r - l) as f32,
                        (b - t) as f32,
                        7.0 * s,
                        Some(if item.danger { danger_hover } else { row_hover }),
                        None,
                    );
                }
            }

            GdipDeleteGraphics(g);
        }

        // ClearType text pass
        let font_item = create_ui_font(13.0, s, false);
        let font_pill = create_ui_font(12.0, s, true);
        let font_lbl = create_ui_font(11.0, s, false);

        SetBkMode(dc, 1);

        // Group mini labels
        let old_f = SelectObject(dc, font_lbl as HGDIOBJ);
        SetTextColor(dc, text_muted);
        for &((l, t, r, b), lbl) in &layout.row_labels {
            let mut tr = RECT { left: l, top: t, right: r, bottom: b };
            let txt = wide(lbl);
            DrawTextW(dc, txt.as_ptr(), -1, &mut tr, 0x04 | 0x20 | 0x0800);
        }

        // Items
        let check_w = (22.0 * s).round() as i32;
        for (idx, item) in layout.items.iter().enumerate() {
            let hovered = hover_idx == Some(idx);
            let (l, t, r, b) = item.rect;
            if item.is_pill {
                SelectObject(dc, font_pill as HGDIOBJ);
                SetTextColor(
                    dc,
                    if item.active_pill || hovered {
                        text_accent
                    } else {
                        text_primary
                    },
                );
                let mut tr = RECT { left: l, top: t, right: r, bottom: b };
                let txt = wide(&item.label);
                DrawTextW(dc, txt.as_ptr(), -1, &mut tr, 0x01 | 0x04 | 0x20 | 0x0800);
            } else {
                if item.checked {
                    SelectObject(dc, font_pill as HGDIOBJ);
                    SetTextColor(dc, text_accent);
                    let mut cr = RECT { left: l + (4.0 * s).round() as i32, top: t, right: l + check_w, bottom: b };
                    let chk = wide("✓");
                    DrawTextW(dc, chk.as_ptr(), -1, &mut cr, 0x01 | 0x04 | 0x20);
                }
                SelectObject(dc, font_item as HGDIOBJ);
                SetTextColor(
                    dc,
                    if item.danger {
                        text_danger
                    } else if hovered {
                        text_accent
                    } else {
                        text_primary
                    },
                );
                let mut tr = RECT {
                    left: l + check_w + (2.0 * s).round() as i32,
                    top: t,
                    right: r - (8.0 * s).round() as i32,
                    bottom: b,
                };
                let txt = wide(&item.label);
                DrawTextW(dc, txt.as_ptr(), -1, &mut tr, 0x04 | 0x20 | 0x0800);
            }
        }

        SelectObject(dc, old_f);
        DeleteObject(font_item as HGDIOBJ);
        DeleteObject(font_pill as HGDIOBJ);
        DeleteObject(font_lbl as HGDIOBJ);
    }

    if use_buf {
        BitBlt(target_dc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
        SelectObject(mem_dc, old_bmp);
        DeleteObject(bmp as HGDIOBJ);
        DeleteDC(mem_dc);
    } else {
        if !bmp.is_null() {
            DeleteObject(bmp as HGDIOBJ);
        }
        if !mem_dc.is_null() {
            DeleteDC(mem_dc);
        }
    }
    EndPaint(hwnd, &ps);
}
