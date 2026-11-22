//! SnowAir-inspired dual-theme color tokens for native floating UI windows.

use crate::model::{parse_color, Settings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    pub is_dark: bool,
    pub bg: [u8; 4],
    pub hover: [u8; 4],
    pub icon: [u8; 4],
    pub icon_disabled: [u8; 4],
    pub accent: [u8; 4],
    pub selection: [u8; 4],
    pub border: [u8; 4],
    pub divider: [u8; 4],
    pub danger: [u8; 4],
    pub shadow: [u8; 4],
    pub info_bg: [u8; 4],
    pub info_text: [u8; 4],
    pub info_muted: [u8; 4],
    pub slider_track: [u8; 4],
    pub slider_thumb: [u8; 4],
}

impl ThemePalette {
    pub fn dark() -> Self {
        Self {
            is_dark: true,
            bg: [0x1E, 0x1E, 0x1F, 0xFF],
            hover: [0x33, 0x33, 0x36, 0xFF],
            icon: [0xF2, 0xF2, 0xF5, 0xFF],
            icon_disabled: [0x78, 0x78, 0x7E, 0xFF],
            accent: [0x34, 0xC7, 0x59, 0xFF],
            selection: [0x34, 0xC7, 0x59, 0xFF],
            border: [0x38, 0x38, 0x3C, 0xFF],
            divider: [0x3A, 0x3A, 0x3E, 0xFF],
            danger: [0xEF, 0x44, 0x44, 0xFF],
            shadow: [0x0D, 0x0D, 0x0F, 0xFF],
            info_bg: [0x1E, 0x1E, 0x20, 0xFF],
            info_text: [0xFA, 0xFA, 0xFC, 0xFF],
            info_muted: [0xA1, 0xA1, 0xAA, 0xFF],
            slider_track: [0x3B, 0x3B, 0x40, 0xFF],
            slider_thumb: [0xF4, 0xF4, 0xF6, 0xFF],
        }
    }

    pub fn light() -> Self {
        Self {
            is_dark: false,
            bg: [0xFA, 0xFA, 0xFA, 0xFF],
            hover: [0xE6, 0xE6, 0xE8, 0xFF],
            icon: [0x45, 0x45, 0x4A, 0xFF],
            icon_disabled: [0xAD, 0xAD, 0xB2, 0xFF],
            accent: [0x34, 0xC7, 0x59, 0xFF],
            selection: [0x34, 0xC7, 0x59, 0xFF],
            border: [0xDC, 0xDC, 0xE0, 0xFF],
            divider: [0xE2, 0xE2, 0xE6, 0xFF],
            danger: [0xEF, 0x44, 0x44, 0xFF],
            shadow: [0xCE, 0xD2, 0xD9, 0xFF],
            info_bg: [0xFA, 0xFA, 0xFA, 0xFF],
            info_text: [0x1F, 0x23, 0x29, 0xFF],
            info_muted: [0x71, 0x71, 0x7A, 0xFF],
            slider_track: [0xDC, 0xDC, 0xE0, 0xFF],
            slider_thumb: [0xFF, 0xFF, 0xFF, 0xFF],
        }
    }

    pub fn from_settings(settings: &Settings) -> Self {
        match settings.toolbar_theme.as_str() {
            "dark" => Self::dark(),
            "light" => Self::light(),
            "custom" => {
                let bg = parse_color(&settings.toolbar_custom_bg).unwrap_or([0x1E, 0x1E, 0x1F, 0xFF]);
                let hover = parse_color(&settings.toolbar_custom_hover).unwrap_or([0x33, 0x33, 0x36, 0xFF]);
                let icon = parse_color(&settings.toolbar_custom_icon).unwrap_or([0xF2, 0xF2, 0xF5, 0xFF]);
                let accent = parse_color(&settings.toolbar_custom_accent).unwrap_or([0x34, 0xC7, 0x59, 0xFF]);
                let selection = parse_color(&settings.toolbar_custom_selection).unwrap_or(accent);
                let luma = (bg[0] as u32 * 299 + bg[1] as u32 * 587 + bg[2] as u32 * 114) / 1000;
                let is_dark = luma < 128;
                let base = if is_dark { Self::dark() } else { Self::light() };
                Self {
                    is_dark,
                    bg,
                    hover,
                    icon,
                    icon_disabled: blend_rgba(icon, bg, 110),
                    accent,
                    selection,
                    border: blend_rgba(icon, bg, 45),
                    divider: blend_rgba(icon, bg, 50),
                    info_bg: bg,
                    info_text: icon,
                    info_muted: blend_rgba(icon, bg, 150),
                    slider_track: hover,
                    slider_thumb: icon,
                    ..base
                }
            }
            _ => {
                if system_prefers_light() {
                    Self::light()
                } else {
                    Self::dark()
                }
            }
        }
    }

    pub fn colorref(c: [u8; 4]) -> u32 {
        u32::from(c[0]) | (u32::from(c[1]) << 8) | (u32::from(c[2]) << 16)
    }

    pub fn argb(c: [u8; 4]) -> u32 {
        (u32::from(c[3]) << 24) | (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2])
    }
}

pub fn blend_rgba(fg: [u8; 4], bg: [u8; 4], alpha: u8) -> [u8; 4] {
    let a = u16::from(alpha);
    let inv = 255 - a;
    [
        ((u16::from(fg[0]) * a + u16::from(bg[0]) * inv) / 255) as u8,
        ((u16::from(fg[1]) * a + u16::from(bg[1]) * inv) / 255) as u8,
        ((u16::from(fg[2]) * a + u16::from(bg[2]) * inv) / 255) as u8,
        255,
    ]
}

#[cfg(windows)]
pub fn system_dpi_scale() -> f32 {
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC};
    const LOGPIXELSX: i32 = 88;
    unsafe {
        let dc = GetDC(null_mut());
        if dc.is_null() {
            return 1.0;
        }
        let dpi = GetDeviceCaps(dc, LOGPIXELSX);
        ReleaseDC(null_mut(), dc);
        if dpi <= 0 {
            1.0
        } else {
            (dpi as f32 / 96.0).clamp(1.0, 2.5)
        }
    }
}

#[cfg(not(windows))]
pub fn system_dpi_scale() -> f32 {
    1.0
}

pub fn ui_scale_for_settings(settings: &Settings) -> f32 {
    let pref = match settings.toolbar_size.as_str() {
        "small" => 0.88,
        "large" => 1.18,
        _ => 1.0,
    };
    (system_dpi_scale() * pref).clamp(0.85, 2.75)
}

#[cfg(windows)]
pub fn system_prefers_light() -> bool {
    use std::ptr::null_mut;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    let subkey: Vec<u16> = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let value_name: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
    let mut hkey: HKEY = null_mut();
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return false;
        }
        let mut data: u32 = 0;
        let mut size: u32 = std::mem::size_of::<u32>() as u32;
        let ok = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            null_mut(),
            null_mut(),
            &mut data as *mut u32 as *mut u8,
            &mut size,
        ) == 0;
        RegCloseKey(hkey);
        ok && data != 0
    }
}

#[cfg(not(windows))]
pub fn system_prefers_light() -> bool {
    false
}
