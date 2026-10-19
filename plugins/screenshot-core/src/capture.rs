use crate::model::ScreenRect;
use crate::scroll::{FrameProbe, SCROLL_PROBE_GRID};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, SetStretchBltMode, StretchBlt, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    COLORONCOLOR, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, SRCCOPY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, GetSystemMetrics, CURSORINFO, CURSOR_SHOWING, DI_NORMAL,
    ICONINFO, SM_CXCURSOR, SM_CYCURSOR, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// A virtual-desktop screenshot in physical pixels.
#[derive(Clone)]
pub(super) struct ScreenCapture {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) pixels: Vec<u8>,
    pub(super) dimmed_pixels: Vec<u8>,
}

impl ScreenCapture {
    fn rect_from_metrics(left: i32, top: i32, width: i32, height: i32) -> Option<ScreenRect> {
        (width > 0 && height > 0).then_some(ScreenRect {
            left,
            top,
            right: left.saturating_add(width),
            bottom: top.saturating_add(height),
        })
    }

    fn virtual_desktop_rect() -> Option<ScreenRect> {
        unsafe {
            let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            Self::rect_from_metrics(left, top, width, height)
        }
    }

    #[allow(dead_code)]
    pub(super) fn bounds(&self) -> ScreenRect {
        ScreenRect {
            left: self.x,
            top: self.y,
            right: self.x.saturating_add(self.width),
            bottom: self.y.saturating_add(self.height),
        }
    }
    pub(super) fn grab_probe(rect: ScreenRect) -> Option<FrameProbe> {
        let grid = SCROLL_PROBE_GRID as i32;
        let width = rect.width() as i32;
        let height = rect.height() as i32;
        if width <= 0 || height <= 0 {
            return None;
        }
        let bounds = Self::virtual_desktop_rect()?;
        if rect.clipped_to(bounds) != Some(rect) {
            return None;
        }
        unsafe {
            let screen_dc = GetDC(null_mut());
            if screen_dc.is_null() {
                return None;
            }
            let memory_dc = CreateCompatibleDC(screen_dc);
            if memory_dc.is_null() {
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            let info = bitmap_info(grid, -grid);
            let mut bits: *mut c_void = null_mut();
            let bitmap =
                CreateDIBSection(screen_dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap as HGDIOBJ);
                }
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            let previous = SelectObject(memory_dc, bitmap as HGDIOBJ);
            if previous.is_null() {
                DeleteObject(bitmap as HGDIOBJ);
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            let stretch_mode = SetStretchBltMode(memory_dc, COLORONCOLOR);
            // Downsample in one GDI operation instead of issuing one GetPixel
            // call per sample on the UI timer thread.
            let captured = stretch_mode != 0
                && StretchBlt(
                    memory_dc,
                    0,
                    0,
                    grid,
                    grid,
                    screen_dc,
                    rect.left,
                    rect.top,
                    width,
                    height,
                    SRCCOPY | 0x40000000,
                ) != 0;
            let samples = if captured {
                let bgra = std::slice::from_raw_parts(
                    bits as *const u8,
                    (grid * grid * 4) as usize,
                );
                bgra.chunks_exact(4)
                    .map(|pixel| bgra_luminance([pixel[0], pixel[1], pixel[2], pixel[3]]))
                    .collect()
            } else {
                Vec::new()
            };
            SelectObject(memory_dc, previous);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(memory_dc);
            ReleaseDC(null_mut(), screen_dc);
            (!samples.is_empty()).then(|| FrameProbe::new(samples))
        }
    }

    pub(super) fn grab_region(rect: ScreenRect) -> Option<Vec<u8>> {
        Self::grab_region_with_layered_windows(rect, true)
    }

    fn grab_region_with_layered_windows(
        rect: ScreenRect,
        include_layered_windows: bool,
    ) -> Option<Vec<u8>> {
        let width = rect.width() as i32;
        let height = rect.height() as i32;
        if width <= 0 || height <= 0 {
            return None;
        }

        let bounds = Self::virtual_desktop_rect()?;

        if rect.clipped_to(bounds) != Some(rect) {
            return None;
        }
        unsafe {
            let screen_dc = GetDC(null_mut());
            if screen_dc.is_null() {
                return None;
            }
            let memory_dc = CreateCompatibleDC(screen_dc);
            if memory_dc.is_null() {
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            let info = bitmap_info(width, -height);
            let mut bits: *mut c_void = null_mut();
            let bitmap =
                CreateDIBSection(screen_dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap as HGDIOBJ);
                }
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            let previous = SelectObject(memory_dc, bitmap as HGDIOBJ);
            let ok = BitBlt(
                memory_dc,
                0,
                0,
                width,
                height,
                screen_dc,
                rect.left,
                rect.top,
                if include_layered_windows {
                    SRCCOPY | 0x40000000
                } else {
                    SRCCOPY
                },
            );
            let pixels = if ok != 0 {
                let bgra =
                    std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);
                let mut rgba = vec![0u8; bgra.len()];
                let (bgra_chunks, _) = bgra.as_chunks::<4>();
                let (rgba_chunks, _) = rgba.as_chunks_mut::<4>();
                for (source, target) in bgra_chunks.iter().zip(rgba_chunks.iter_mut()) {
                    target[0] = source[2];
                    target[1] = source[1];
                    target[2] = source[0];
                    target[3] = 255;
                }
                rgba
            } else {
                Vec::new()
            };
            SelectObject(memory_dc, previous);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(memory_dc);
            ReleaseDC(null_mut(), screen_dc);
            (!pixels.is_empty()).then_some(pixels)
        }
    }

    pub(super) fn grab_virtual_desktop() -> Option<Self> {
        let bounds = Self::virtual_desktop_rect()?;
        let x = bounds.left;
        let y = bounds.top;
        let width = bounds.width() as i32;
        let height = bounds.height() as i32;
        let pixels = super::hdr_capture::capture_virtual_desktop(bounds)
            .or_else(|| Self::grab_virtual_desktop_gdi(bounds))?;
        let dimmed_pixels = pixels
            .iter()
            .map(|value| (*value as u16 * 55 / 100) as u8)
            .collect();
        Some(Self {
            x,
            y,
            width,
            height,
            pixels,
            dimmed_pixels,
        })
    }

    fn grab_virtual_desktop_gdi(bounds: ScreenRect) -> Option<Vec<u8>> {
        unsafe {
            let x = bounds.left;
            let y = bounds.top;
            let width = bounds.width() as i32;
            let height = bounds.height() as i32;
            let screen_dc = GetDC(null_mut());
            if screen_dc.is_null() {
                return None;
            }
            let memory_dc = CreateCompatibleDC(screen_dc);
            if memory_dc.is_null() {
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            let info = bitmap_info(width, -height);
            let mut bits: *mut c_void = null_mut();
            let bitmap =
                CreateDIBSection(screen_dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap as HGDIOBJ);
                }
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            let previous = SelectObject(memory_dc, bitmap as HGDIOBJ);
            let ok = BitBlt(
                memory_dc,
                0,
                0,
                width,
                height,
                screen_dc,
                x,
                y,
                SRCCOPY | 0x40000000,
            );
            let pixels = if ok != 0 {
                std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize)
                    .to_vec()
            } else {
                Vec::new()
            };
            SelectObject(memory_dc, previous);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(memory_dc);
            ReleaseDC(null_mut(), screen_dc);
            (!pixels.is_empty()).then_some(pixels)
        }
    }

    pub(super) fn dimmed_pixels(&self) -> &[u8] {
        &self.dimmed_pixels
    }

    pub(super) fn crop(&self, rect: ScreenRect) -> Vec<u8> {
        let width = rect.width();
        let height = rect.height();
        let mut output = vec![0u8; width * height * 4];
        for y in 0..height {
            let sy = rect.top - self.y + y as i32;
            let sx = rect.left - self.x;
            let src = ((sy * self.width + sx) * 4) as usize;
            for x in 0..width {
                let source = src + x * 4;
                let target = (y * width + x) * 4;
                output[target] = self.pixels[source + 2];
                output[target + 1] = self.pixels[source + 1];
                output[target + 2] = self.pixels[source];
                output[target + 3] = 255;
            }
        }
        output
    }
    pub(super) fn probe(&self, rect: ScreenRect) -> FrameProbe {
        let grid = SCROLL_PROBE_GRID;
        let width = rect.width();
        let height = rect.height();
        let mut samples = Vec::with_capacity(grid * grid);
        for gy in 0..grid {
            let y = ((gy * 2 + 1) * height / (grid * 2)).min(height.saturating_sub(1));
            for gx in 0..grid {
                let x = ((gx * 2 + 1) * width / (grid * 2)).min(width.saturating_sub(1));
                let sx = rect.left - self.x + x as i32;
                let sy = rect.top - self.y + y as i32;
                let offset = ((sy * self.width + sx) * 4) as usize;
                if offset + 2 < self.pixels.len() {
                    let b = self.pixels[offset] as u16;
                    let g = self.pixels[offset + 1] as u16;
                    let r = self.pixels[offset + 2] as u16;
                    samples.push(((b * 29 + g * 150 + r * 77) / 256) as u8);
                }
            }
        }
        FrameProbe::new(samples)
    }
}

/// Reusable GDI surfaces for recording. Frames are scaled while they are
/// copied from the desktop, avoiding a full-size intermediate allocation and
/// a second CPU resize pass on every frame.
pub(super) struct RecordingFrameGrabber {
    screen_dc: HDC,
    memory_dc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    bits: *mut u8,
    rect: ScreenRect,
    width: i32,
    height: i32,
    include_cursor: bool,
}

impl RecordingFrameGrabber {
    pub(super) fn new(
        rect: ScreenRect,
        width: u32,
        height: u32,
        include_cursor: bool,
    ) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let bounds = ScreenCapture::virtual_desktop_rect()?;
        if rect.clipped_to(bounds) != Some(rect) {
            return None;
        }

        unsafe {
            let screen_dc = GetDC(null_mut());
            if screen_dc.is_null() {
                return None;
            }
            let memory_dc = CreateCompatibleDC(screen_dc);
            if memory_dc.is_null() {
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            let width = width as i32;
            let height = height as i32;
            let info = bitmap_info(width, -height);
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                screen_dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                null_mut(),
                0,
            );
            if bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap as HGDIOBJ);
                }
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            let previous_bitmap = SelectObject(memory_dc, bitmap as HGDIOBJ);
            if previous_bitmap.is_null() {
                DeleteObject(bitmap as HGDIOBJ);
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }
            if SetStretchBltMode(memory_dc, COLORONCOLOR) == 0 {
                SelectObject(memory_dc, previous_bitmap);
                DeleteObject(bitmap as HGDIOBJ);
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
                return None;
            }

            Some(Self {
                screen_dc,
                memory_dc,
                bitmap,
                previous_bitmap,
                bits: bits.cast(),
                rect,
                width,
                height,
                include_cursor,
            })
        }
    }

    pub(super) fn capture(&mut self) -> Option<Vec<u8>> {
        unsafe {
            if StretchBlt(
                    self.memory_dc,
                    0,
                    0,
                    self.width,
                    self.height,
                    self.screen_dc,
                    self.rect.left,
                    self.rect.top,
                    self.rect.width() as i32,
                    self.rect.height() as i32,
                    SRCCOPY,
                ) == 0 {
                return None;
            }
            if self.include_cursor {
                self.draw_cursor();
            }

            let byte_count = (self.width as usize)
                .checked_mul(self.height as usize)?
                .checked_mul(4)?;
            let bgra = std::slice::from_raw_parts(self.bits, byte_count);
            let mut rgba = vec![0u8; byte_count];
            let (source_pixels, _) = bgra.as_chunks::<4>();
            let (target_pixels, _) = rgba.as_chunks_mut::<4>();
            for (source, target) in source_pixels.iter().zip(target_pixels.iter_mut()) {
                target[0] = source[2];
                target[1] = source[1];
                target[2] = source[0];
                target[3] = 255;
            }
            Some(rgba)
        }
    }

    unsafe fn draw_cursor(&self) {
        let mut cursor = CURSORINFO {
            cbSize: size_of::<CURSORINFO>() as u32,
            ..zeroed()
        };
        if GetCursorInfo(&mut cursor) == 0
            || cursor.flags & CURSOR_SHOWING == 0
            || cursor.ptScreenPos.x < self.rect.left
            || cursor.ptScreenPos.x >= self.rect.right
            || cursor.ptScreenPos.y < self.rect.top
            || cursor.ptScreenPos.y >= self.rect.bottom
            || cursor.hCursor.is_null()
        {
            return;
        }

        let mut icon_info: ICONINFO = zeroed();
        let (hotspot_x, hotspot_y) = if GetIconInfo(cursor.hCursor, &mut icon_info) != 0 {
            let hotspot = (icon_info.xHotspot as i32, icon_info.yHotspot as i32);
            if !icon_info.hbmColor.is_null() {
                DeleteObject(icon_info.hbmColor as HGDIOBJ);
            }
            if !icon_info.hbmMask.is_null() {
                DeleteObject(icon_info.hbmMask as HGDIOBJ);
            }
            hotspot
        } else {
            (0, 0)
        };

        let source_width = i64::from(self.rect.width() as i32);
        let source_height = i64::from(self.rect.height() as i32);
        let output_x = |position: i32, hotspot: i32| {
            (i64::from(position - self.rect.left - hotspot) * i64::from(self.width)
                / source_width) as i32
        };
        let output_y = |position: i32, hotspot: i32| {
            (i64::from(position - self.rect.top - hotspot) * i64::from(self.height)
                / source_height) as i32
        };
        let cursor_width = (i64::from(GetSystemMetrics(SM_CXCURSOR).max(1))
            * i64::from(self.width)
            / source_width)
            .max(1) as i32;
        let cursor_height = (i64::from(GetSystemMetrics(SM_CYCURSOR).max(1))
            * i64::from(self.height)
            / source_height)
            .max(1) as i32;
        DrawIconEx(
            self.memory_dc,
            output_x(cursor.ptScreenPos.x, hotspot_x),
            output_y(cursor.ptScreenPos.y, hotspot_y),
            cursor.hCursor as _,
            cursor_width,
            cursor_height,
            0,
            null_mut(),
            DI_NORMAL,
        );
    }
}

impl Drop for RecordingFrameGrabber {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.memory_dc, self.previous_bitmap);
            DeleteObject(self.bitmap as HGDIOBJ);
            DeleteDC(self.memory_dc);
            ReleaseDC(null_mut(), self.screen_dc);
        }
    }
}

/// Convert one 32-bit BI_RGB sample in little-endian BGRA order to luminance.
fn bgra_luminance(pixel: [u8; 4]) -> u8 {
    let blue = pixel[0] as u16;
    let green = pixel[1] as u16;
    let red = pixel[2] as u16;
    ((red * 77 + green * 150 + blue * 29) / 256) as u8
}

fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
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

#[cfg(test)]
mod tests {
    use super::{bgra_luminance, ScreenCapture};
    use crate::model::ScreenRect;

    #[test]
    fn capture_bounds_preserve_physical_origin() {
        let capture = ScreenCapture {
            x: -1920,
            y: -200,
            width: 3840,
            height: 1280,
            pixels: Vec::new(),
            dimmed_pixels: Vec::new(),
        };
        let bounds = capture.bounds();
        assert_eq!(bounds.left, -1920);
        assert_eq!(bounds.top, -200);
        assert_eq!(bounds.right, 1920);
        assert_eq!(bounds.bottom, 1080);
    }
    #[test]
    fn metrics_support_negative_origin_and_physical_scale() {
        let bounds = ScreenCapture::rect_from_metrics(-2560, -1440, 5120, 2880).unwrap();
        assert_eq!(bounds.left, -2560);
        assert_eq!(bounds.top, -1440);
        assert_eq!(bounds.right, 2560);
        assert_eq!(bounds.bottom, 1440);
        let selection = ScreenRect {
            left: -1920,
            top: -1080,
            right: 1920,
            bottom: 1080,
        };
        assert!(selection
            .clipped_to(bounds)
            .is_some_and(|clipped| clipped == selection));
    }

    #[test]
    fn metrics_reject_non_positive_desktop_size() {
        assert!(ScreenCapture::rect_from_metrics(0, 0, 0, 1080).is_none());
        assert!(ScreenCapture::rect_from_metrics(0, 0, 1920, -1).is_none());
    }
    #[test]
    fn bgra_probe_luminance_uses_channel_order() {
        assert!(bgra_luminance([0, 0, 255, 255]) > bgra_luminance([255, 0, 0, 255]));
        assert_eq!(bgra_luminance([0, 255, 0, 255]), 149);
    }

    #[test]
    fn physical_virtual_desktop_bounds_guard_region_capture() {
        crate::windows::enable_dpi_awareness();
        let Some(bounds) = ScreenCapture::virtual_desktop_rect() else {
            return;
        };
        assert!(bounds.width() > 0 && bounds.height() > 0);
        let inside = ScreenRect {
            left: bounds.left,
            top: bounds.top,
            right: bounds.left + 1,
            bottom: bounds.top + 1,
        };
        assert!(ScreenCapture::grab_region(inside).is_some());
        assert!(ScreenCapture::grab_probe(inside).is_some());
        let outside = ScreenRect {
            left: bounds.left - 1,
            top: bounds.top,
            right: bounds.left,
            bottom: bounds.top + 1,
        };
        assert!(ScreenCapture::grab_region(outside).is_none());
    }

    #[test]
    fn region_capture_returns_expected_rgba_length_for_small_physical_rect() {
        crate::windows::enable_dpi_awareness();
        let Some(bounds) = ScreenCapture::virtual_desktop_rect() else {
            return;
        };
        let width = bounds.width().min(8) as i32;
        let height = bounds.height().min(8) as i32;
        if width <= 0 || height <= 0 {
            return;
        }
        let rect = ScreenRect {
            left: bounds.left,
            top: bounds.top,
            right: bounds.left + width,
            bottom: bounds.top + height,
        };
        let pixels = ScreenCapture::grab_region(rect).unwrap();
        assert_eq!(pixels.len(), width as usize * height as usize * 4);
        assert!(ScreenCapture::grab_probe(rect).is_some());
    }
}
