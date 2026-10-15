//! Native Windows rendering owned by the time plugin.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{Local, Timelike};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, CreateFontW, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GdiFlush, GetDC, GetStockObject,
    InvalidateRect, ReleaseDC,
    RoundRect, SelectObject, SetBkMode, SetTextColor, SetWindowRgn, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DEFAULT_CHARSET, DEFAULT_PITCH,
    DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, FW_NORMAL,
    OUT_DEFAULT_PRECIS, PS_SOLID, TRANSPARENT,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, SUBCLASSPROC,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowLongPtrW, GetWindowRect, KillTimer, SendMessageW,
    SetTimer, SetWindowLongPtrW, SetWindowPos, UpdateLayeredWindow, GWL_EXSTYLE, HTCAPTION,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, ULW_ALPHA,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_NCDESTROY, WM_NCLBUTTONDOWN, WM_PAINT,
    WM_RBUTTONUP, WM_SHOWWINDOW, WM_SIZE, WM_TIMER, WS_EX_LAYERED,
};

const SUBCLASS_ID: usize = 0xA61C_0201;
const CLOCK_TIMER_ID: usize = 0xA61C_0202;

#[derive(Clone)]
struct ClockStyle {
    background_opacity: f64,
    content_opacity: f64,
    scale: f64,
    color: String,
    skin: String,
    mini_time_format: String,
    mini_layout: String,
    font_weight: u16,
    pomo_work_min: u32,
    pomo_break_min: u32,
    pinned: bool,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Alarm {
    id: String,
    label: String,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct NextAlarm {
    alarm_id: String,
    at_secs: i64,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct ClockSnapshot {
    alarms: Vec<Alarm>,
    next: Option<NextAlarm>,
    alarm_flash_enabled: bool,
}

#[derive(Clone)]
struct Pomodoro {
    phase_work: bool,
    remaining: i64,
    running: bool,
    rounds: u32,
}

#[derive(Clone)]
struct ClockState {
    style: ClockStyle,
    snapshot: ClockSnapshot,
    pomo: Pomodoro,
    alert_phase: bool,
    flash_until: Option<Instant>,
    applying_region: bool,
    visible: bool,
}

struct RegistryEntry {
    hwnd: isize,
    state: Arc<Mutex<ClockState>>,
    callbacks: NativeClockCallbacks,
}

#[derive(Clone)]
pub(super) struct NativeClockCallbacks {
    pub pomo_finished: Arc<dyn Fn(&str) + Send + Sync>,
}

fn registry() -> &'static Mutex<HashMap<String, RegistryEntry>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, RegistryEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn states_by_hwnd() -> &'static Mutex<HashMap<isize, Arc<Mutex<ClockState>>>> {
    static STATES: OnceLock<Mutex<HashMap<isize, Arc<Mutex<ClockState>>>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn clock_style(style: &super::ClockStyle) -> ClockStyle {
    ClockStyle {
        background_opacity: style.background_opacity,
        content_opacity: style.content_opacity,
        scale: style.scale,
        color: style.color.clone(),
        skin: style.skin.clone(),
        mini_time_format: style.mini_time_format.clone(),
        mini_layout: style.mini_layout.clone(),
        font_weight: style.font_weight,
        pomo_work_min: style.pomo_work_min,
        pomo_break_min: style.pomo_break_min,
        pinned: style.pinned,
    }
}

fn clock_snapshot(snapshot: &super::TimeSnapshot) -> ClockSnapshot {
    ClockSnapshot {
        alarms: snapshot
            .alarms
            .iter()
            .map(|alarm| Alarm {
                id: alarm.id.clone(),
                label: alarm.label.clone(),
            })
            .collect(),
        next: snapshot.next.as_ref().map(|next| NextAlarm {
            alarm_id: next.alarm_id.clone(),
            at_secs: next.at_secs,
        }),
        alarm_flash_enabled: snapshot.alarm_clock_flash_enabled,
    }
}

pub(super) fn attach(
    id: &str,
    hwnd_value: u64,
    style: &super::ClockStyle,
    snapshot: &super::TimeSnapshot,
    callbacks: NativeClockCallbacks,
) -> Result<(), String> {
    let hwnd = hwnd_value as HWND;
    let style = clock_style(style);
    let state = Arc::new(Mutex::new(ClockState {
        pomo: Pomodoro {
            phase_work: true,
            remaining: i64::from(style.pomo_work_min.max(1)) * 60,
            running: false,
            rounds: 0,
        },
        style,
        snapshot: clock_snapshot(snapshot),
        alert_phase: true,
        flash_until: None,
        applying_region: false,
        visible: false,
    }));
    let key = hwnd as isize;
    registry()
        .lock()
        .map_err(|_| "native clock registry is poisoned".to_string())?
        .insert(
            id.to_string(),
            RegistryEntry {
                hwnd: key,
                state: Arc::clone(&state),
                callbacks,
            },
        );
    states_by_hwnd()
        .lock()
        .map_err(|_| "native clock state registry is poisoned".to_string())?
        .insert(key, Arc::clone(&state));
    let subclass: SUBCLASSPROC = Some(subclass_proc);
    unsafe {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED as isize);
        let _ = SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        if SetWindowSubclass(hwnd, subclass, SUBCLASS_ID, 0) == 0 {
            detach(id);
            return Err("failed to install native clock renderer".into());
        }
    }
    apply_window_style(hwnd);
    apply_region_guarded(hwnd, &state);
    invalidate(hwnd);
    Ok(())
}

pub(super) fn update_style(id: &str, style: &super::ClockStyle) {
    let Some((hwnd, state)) = registry().lock().ok().and_then(|items| {
        items
            .get(id)
            .map(|entry| (entry.hwnd, Arc::clone(&entry.state)))
    }) else {
        return;
    };
    if let Ok(mut state) = state.lock() {
        let new_style = clock_style(style);
        state.style = new_style;
        if state.pomo.remaining <= 0 {
            state.pomo.remaining = i64::from(state.style.pomo_work_min.max(1)) * 60;
        }
    }
    apply_region_guarded(hwnd as HWND, &state);
    refresh_timer(hwnd as HWND, &state);
    invalidate(hwnd as HWND);
}

pub(super) fn update_snapshot(id: &str, snapshot: &super::TimeSnapshot) {
    let Some((hwnd, state)) = registry().lock().ok().and_then(|items| {
        items
            .get(id)
            .map(|entry| (entry.hwnd, Arc::clone(&entry.state)))
    }) else {
        return;
    };
    let changed = if let Ok(mut state) = state.lock() {
        let snapshot = clock_snapshot(snapshot);
        if state.snapshot == snapshot {
            false
        } else {
            state.snapshot = snapshot;
            true
        }
    } else {
        false
    };
    if changed {
        refresh_timer(hwnd as HWND, &state);
        invalidate(hwnd as HWND);
    }
}

pub(super) fn alarm(id: &str) {
    let Some((hwnd, state)) = registry().lock().ok().and_then(|items| {
        items
            .get(id)
            .map(|entry| (entry.hwnd, Arc::clone(&entry.state)))
    }) else {
        return;
    };
    if let Ok(mut state) = state.lock() {
        state.flash_until = Some(Instant::now() + Duration::from_secs(6));
    }
    refresh_timer(hwnd as HWND, &state);
    invalidate(hwnd as HWND);
}

pub(super) fn set_visible(id: &str, visible: bool) {
    let entry = registry().lock().ok().and_then(|items| {
        items
            .get(id)
            .map(|entry| (entry.hwnd as HWND, Arc::clone(&entry.state)))
    });
    if let Some((hwnd, state)) = entry {
        if let Ok(mut state) = state.lock() {
            state.visible = visible;
        }
        if visible {
            // Re-enable the layered style after the host shows the native HWND;
            // WM_PAINT will then apply the saved per-pixel opacity values.
            reapply_window_style(hwnd, &state);
            invalidate(hwnd);
        }
        refresh_timer(hwnd, &state);
    }
}

pub(super) fn detach(id: &str) {
    let hwnd = registry()
        .lock()
        .ok()
        .and_then(|mut items| items.remove(id).map(|entry| entry.hwnd as HWND));
    let Some(hwnd) = hwnd else {
        return;
    };
    unregister(hwnd);
}

fn unregister(hwnd: HWND) {
    if let Ok(mut items) = registry().lock() {
        items.retain(|_, entry| entry.hwnd != hwnd as isize);
    }
    if let Ok(mut items) = states_by_hwnd().lock() {
        items.remove(&(hwnd as isize));
    }
    unsafe {
        KillTimer(hwnd, CLOCK_TIMER_ID);
        let subclass: SUBCLASSPROC = Some(subclass_proc);
        let _ = RemoveWindowSubclass(hwnd, subclass, SUBCLASS_ID);
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _reference_data: usize,
) -> LRESULT {
    let Some(state) = states_by_hwnd()
        .lock()
        .ok()
        .and_then(|items| items.get(&(hwnd as isize)).cloned())
    else {
        return DefSubclassProc(hwnd, message, wparam, lparam);
    };

    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd, &state);
            0
        }
        WM_TIMER if wparam == CLOCK_TIMER_ID => {
            tick(&state, hwnd);
            0
        }
        WM_SIZE => {
            apply_region_guarded(hwnd, &state);
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        WM_SHOWWINDOW => {
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            if let Ok(mut state) = state.lock() {
                state.visible = wparam != 0;
            }
            if wparam != 0 {
                // Tauri/Windows can finish changing layered-window state while
                // processing the show message. Apply the plugin's saved style
                // after the default window procedure has completed.
                reapply_window_style(hwnd, &state);
                invalidate(hwnd);
            }
            refresh_timer(hwnd, &state);
            result
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point_from_lparam(lparam);
            let size = client_size(hwnd);
            let dpi_scale = window_scale(hwnd);
            let drag = if let Ok(state) = state.lock() {
                !state.style.pinned
                    && !(state.style.skin == "pomodoro"
                        && pomo_button_at(x, y, size.0, size.1, state.style.scale, dpi_scale)
                            .is_some())
            } else {
                false
            };
            if drag {
                ReleaseCapture();
                SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, 0);
                return 0;
            }
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        WM_LBUTTONUP => {
            let (x, y) = point_from_lparam(lparam);
            let size = client_size(hwnd);
            let dpi_scale = window_scale(hwnd);
            let pomo_handled = if let Ok(mut state) = state.lock() {
                if state.style.skin == "pomodoro" && !state.style.pinned {
                    match pomo_button_at(x, y, size.0, size.1, state.style.scale, dpi_scale) {
                        Some(false) => {
                            state.pomo.running = !state.pomo.running;
                            true
                        }
                        Some(true) => {
                            state.pomo.phase_work = true;
                            state.pomo.remaining = i64::from(state.style.pomo_work_min.max(1)) * 60;
                            state.pomo.running = false;
                            true
                        }
                        None => false,
                    }
                } else {
                    false
                }
            } else {
                false
            };
            if pomo_handled {
                refresh_timer(hwnd, &state);
                invalidate(hwnd);
                return 0;
            }
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        WM_RBUTTONUP => 0,
        WM_NCDESTROY => {
            let subclass: SUBCLASSPROC = Some(subclass_proc);
            let _ = RemoveWindowSubclass(hwnd, subclass, SUBCLASS_ID);
            if let Ok(mut items) = states_by_hwnd().lock() {
                items.remove(&(hwnd as isize));
            }
            if let Ok(mut items) = registry().lock() {
                items.retain(|_, entry| entry.hwnd != hwnd as isize);
            }
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        _ => DefSubclassProc(hwnd, message, wparam, lparam),
    }
}

fn tick(state: &Arc<Mutex<ClockState>>, hwnd: HWND) {
    if !state.lock().map(|state| state.visible).unwrap_or(false) {
        unsafe { KillTimer(hwnd, CLOCK_TIMER_ID) };
        return;
    }

    let mut phase_finished = None;
    if let Ok(mut state) = state.lock() {
        let now = Local::now().timestamp();
        let final_minute = state.snapshot.alarm_flash_enabled
            && state.snapshot.next.as_ref().is_some_and(|next| {
                (0..=60).contains(&(next.at_secs - now))
            });
        if state.style.skin == "mini" && final_minute {
            state.alert_phase = !state.alert_phase;
        } else {
            state.alert_phase = true;
        }
        if state.style.skin == "pomodoro" && state.pomo.running {
            state.pomo.remaining -= 1;
            if state.pomo.remaining <= 0 {
                let finished_work = state.pomo.phase_work;
                state.pomo.phase_work = !state.pomo.phase_work;
                if state.pomo.phase_work {
                    state.pomo.rounds = state.pomo.rounds.saturating_add(1);
                }
                let minutes = if state.pomo.phase_work {
                    state.style.pomo_work_min
                } else {
                    state.style.pomo_break_min
                };
                state.pomo.remaining = i64::from(minutes.max(1)) * 60;
                phase_finished = Some(if finished_work { "work" } else { "break" });
            }
        }
        if state
            .flash_until
            .is_some_and(|until| until <= Instant::now())
        {
            state.flash_until = None;
        }
    }
    if let Some(phase) = phase_finished {
        let callback = registry().lock().ok().and_then(|items| {
            items
                .values()
                .find(|entry| entry.hwnd == hwnd as isize)
                .map(|entry| Arc::clone(&entry.callbacks.pomo_finished))
        });
        if let Some(callback) = callback {
            callback(phase);
        }
    }
    refresh_timer(hwnd, state);
    invalidate(hwnd);
}

fn apply_window_style(hwnd: HWND) {
    unsafe {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex_style & WS_EX_LAYERED as isize == 0 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED as isize);
        }
    }
}

fn reapply_window_style(hwnd: HWND, state: &Arc<Mutex<ClockState>>) {
    apply_window_style(hwnd);
    apply_region_guarded(hwnd, state);
}

fn apply_region_guarded(hwnd: HWND, state: &Arc<Mutex<ClockState>>) {
    let style = {
        let Ok(mut state) = state.lock() else {
            return;
        };
        if state.applying_region {
            return;
        }
        state.applying_region = true;
        state.style.clone()
    };
    unsafe { apply_region(hwnd, &style) };
    if let Ok(mut state) = state.lock() {
        state.applying_region = false;
    }
}

unsafe fn apply_region(hwnd: HWND, style: &ClockStyle) {
    if style.skin == "mini" {
        let _ = SetWindowRgn(hwnd, std::ptr::null_mut(), 1);
        return;
    }
    let (width, height) = client_size(hwnd);
    let dpi_scale = window_scale(hwnd);
    let radius = (16.0 * dpi_scale).round() as i32;
    let region = windows_sys::Win32::Graphics::Gdi::CreateRoundRectRgn(
        0,
        0,
        width.max(1),
        height.max(1),
        radius.max(1) * 2,
        radius.max(1) * 2,
    );
    if !region.is_null() {
        let _ = SetWindowRgn(hwnd, region, 1);
    }
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn invalidate(hwnd: HWND) {
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

fn refresh_timer(hwnd: HWND, state: &Arc<Mutex<ClockState>>) {
    let interval = state
        .lock()
        .ok()
        .and_then(|state| next_timer_delay_ms(&state));
    unsafe {
        if let Some(interval) = interval {
            let _ = SetTimer(hwnd, CLOCK_TIMER_ID, interval, None);
        } else {
            KillTimer(hwnd, CLOCK_TIMER_ID);
        }
    }
}

fn next_timer_delay_ms(state: &ClockState) -> Option<u32> {
    if !state.visible {
        return None;
    }

    let now = Local::now();
    let skin = state.style.skin.as_str();
    let millis = u64::from(now.timestamp_subsec_millis());
    let mut delays = Vec::with_capacity(3);

    let urgent = state
        .snapshot
        .next
        .as_ref()
        .is_some_and(|next| {
            (0..=super::ALARM_WARNING_SECS).contains(&(next.at_secs - now.timestamp()))
        });
    let second_based = skin == "mini" && state.style.mini_time_format == "hms";

    if skin == "pomodoro" && state.pomo.running {
        // The countdown advances from the moment it starts, not on a wall-clock boundary.
        delays.push(1000);
    } else if second_based || (skin == "mini" && urgent) || (skin == "pomodoro" && urgent) {
        delays.push(1000 - millis);
    } else if skin == "mini" {
        let elapsed_ms = u64::from(now.second()) * 1000 + millis;
        delays.push(60_000 - elapsed_ms);
    }

    // Wake at the alarm warning boundary without polling the mini clock every second.
    if skin == "mini" || (skin == "pomodoro" && !state.pomo.running) {
        if let Some(next) = &state.snapshot.next {
            let seconds_to_warning = next.at_secs - super::ALARM_WARNING_SECS - now.timestamp();
            if seconds_to_warning > 0 {
                let delay = seconds_to_warning as u64 * 1000;
                if delay > millis {
                    delays.push(delay - millis);
                }
            }
        }
    }

    if let Some(until) = state.flash_until {
        let remaining = until.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            delays.push(remaining.as_millis().max(1) as u64);
        }
    }

    delays
        .into_iter()
        .min()
        .map(|delay| delay.clamp(1, u64::from(u32::MAX)) as u32)
}

fn point_from_lparam(lparam: LPARAM) -> (i32, i32) {
    let value = lparam as u64;
    (
        (value as u16 as i16) as i32,
        ((value >> 16) as u16 as i16) as i32,
    )
}

unsafe fn paint(hwnd: HWND, state: &Arc<Mutex<ClockState>>) {
    let mut paint_struct = std::mem::zeroed();
    let paint_dc = BeginPaint(hwnd, &mut paint_struct);
    if paint_dc.is_null() {
        return;
    }
    if let Ok(state) = state.lock().map(|state| state.clone()) {
        render_layered(hwnd, &state);
    }
    EndPaint(hwnd, &paint_struct);
}

unsafe fn render_layered(hwnd: HWND, state: &ClockState) {
    let mut window_rect: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut window_rect) == 0 {
        return;
    }
    let width = window_rect.right - window_rect.left;
    let height = window_rect.bottom - window_rect.top;
    if width <= 0 || height <= 0 {
        return;
    }

    let screen_dc = GetDC(std::ptr::null_mut());
    if screen_dc.is_null() {
        return;
    }
    let buffer_dc = CreateCompatibleDC(screen_dc);
    if buffer_dc.is_null() {
        let _ = ReleaseDC(std::ptr::null_mut(), screen_dc);
        return;
    }

    let mut bitmap_info: BITMAPINFO = std::mem::zeroed();
    bitmap_info.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let mut pixels: *mut c_void = std::ptr::null_mut();
    let bitmap = CreateDIBSection(
        screen_dc,
        &bitmap_info,
        DIB_RGB_COLORS,
        &mut pixels,
        std::ptr::null_mut(),
        0,
    );
    if bitmap.is_null() || pixels.is_null() {
        if !bitmap.is_null() {
            let _ = DeleteObject(bitmap);
        }
        let _ = DeleteDC(buffer_dc);
        let _ = ReleaseDC(std::ptr::null_mut(), screen_dc);
        return;
    }

    let previous_bitmap = SelectObject(buffer_dc, bitmap);
    if !previous_bitmap.is_null() {
        let rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let dpi_scale = window_scale(hwnd);
        draw_clock(buffer_dc, &rect, state, dpi_scale, None);
        let _ = GdiFlush();
        let source_pixels = dib_rgb_pixels(pixels, width, height);
        draw_clock(
            buffer_dc,
            &rect,
            state,
            dpi_scale,
            Some(Color(239, 17, 193)),
        );
        let _ = GdiFlush();
        apply_pixel_opacity(pixels, &source_pixels, width, height, state);

        let destination = POINT {
            x: window_rect.left,
            y: window_rect.top,
        };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let _ = UpdateLayeredWindow(
            hwnd,
            screen_dc,
            &destination,
            &size,
            buffer_dc,
            &source,
            0,
            &blend,
            ULW_ALPHA,
        );
        let _ = SelectObject(buffer_dc, previous_bitmap);
    }

    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(buffer_dc);
    let _ = ReleaseDC(std::ptr::null_mut(), screen_dc);
}

fn window_scale(hwnd: HWND) -> f64 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        f64::from(dpi) / 96.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Color(u8, u8, u8);

impl Color {
    fn ref_value(self) -> u32 {
        u32::from(self.0) | (u32::from(self.1) << 8) | (u32::from(self.2) << 16)
    }
}

fn parse_color(value: &str) -> Color {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() == 6 {
        if let Ok(rgb) = u32::from_str_radix(hex, 16) {
            return Color((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        }
    }
    Color(77, 128, 255)
}

fn alarm_flash_color(base: Color) -> Color {
    let red = u16::from(base.0);
    let green = u16::from(base.1);
    let blue = u16::from(base.2);
    if red * 5 > green * 6 && red * 5 > blue * 6 {
        Color(67, 141, 255)
    } else {
        Color(255, 59, 48)
    }
}

fn skin_colors(skin: &str) -> (Color, Color, Color, Color) {
    match skin {
        "mini" => (
            Color(1, 2, 3),
            Color(232, 233, 237),
            Color(154, 156, 166),
            Color(0, 0, 0),
        ),
        _ => (
            Color(20, 20, 25),
            Color(232, 233, 237),
            Color(154, 156, 166),
            Color(58, 59, 66),
        ),
    }
}

unsafe fn dib_rgb_pixels(bits: *mut c_void, width: i32, height: i32) -> Vec<[u8; 3]> {
    let pixel_count = width as usize * height as usize;
    std::slice::from_raw_parts(bits.cast::<u8>(), pixel_count * 4)
        .chunks_exact(4)
        .map(|pixel| [pixel[2], pixel[1], pixel[0]])
        .collect()
}

unsafe fn apply_pixel_opacity(
    bits: *mut c_void,
    source_pixels: &[[u8; 3]],
    width: i32,
    height: i32,
    state: &ClockState,
) {
    let skin = state.style.skin.as_str();
    let (background, _, _, _) = skin_colors(skin);
    let background_opacity = if skin == "mini" {
        0.0
    } else {
        state.style.background_opacity.clamp(0.0, 1.0)
    };
    let content_opacity = state.style.content_opacity.clamp(0.0, 1.0);
    let pixel_count = width as usize * height as usize;
    let bytes = std::slice::from_raw_parts_mut(bits.cast::<u8>(), pixel_count * 4);
    let alternate_background = [239.0, 17.0, 193.0];
    let background_rgb = [
        background.0 as f64,
        background.1 as f64,
        background.2 as f64,
    ];
    let background_delta = [
        background_rgb[0] - alternate_background[0],
        background_rgb[1] - alternate_background[1],
        background_rgb[2] - alternate_background[2],
    ];
    let background_delta_squared = background_delta
        .iter()
        .map(|value| value * value)
        .sum::<f64>();

    for (index, pixel) in bytes.chunks_exact_mut(4).enumerate() {
        // GDI draws glyph edge coverage into the RGB channels. Rendering once
        // over a second base color reveals that coverage without fading text.
        let source = source_pixels[index].map(f64::from);
        let alternate = [pixel[2] as f64, pixel[1] as f64, pixel[0] as f64];
        let difference = [
            source[0] - alternate[0],
            source[1] - alternate[1],
            source[2] - alternate[2],
        ];
        let base_coverage = (difference[0] * background_delta[0]
            + difference[1] * background_delta[1]
            + difference[2] * background_delta[2])
            / background_delta_squared;
        let residual = (0..3)
            .map(|channel| {
                let remaining = difference[channel] - base_coverage * background_delta[channel];
                remaining * remaining
            })
            .sum::<f64>();
        let separates_background = residual <= 16.0 && (-0.02..=1.02).contains(&base_coverage);
        let base_coverage = if separates_background {
            base_coverage.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let foreground_coverage = 1.0 - base_coverage;
        let foreground_color = if separates_background && foreground_coverage > 0.001 {
            Color(
                ((source[0] - base_coverage * background_rgb[0]) / foreground_coverage)
                    .round()
                    .clamp(0.0, 255.0) as u8,
                ((source[1] - base_coverage * background_rgb[1]) / foreground_coverage)
                    .round()
                    .clamp(0.0, 255.0) as u8,
                ((source[2] - base_coverage * background_rgb[2]) / foreground_coverage)
                    .round()
                    .clamp(0.0, 255.0) as u8,
            )
        } else {
            Color(
                source_pixels[index][0],
                source_pixels[index][1],
                source_pixels[index][2],
            )
        };
        let base_weight = base_coverage * background_opacity;
        let foreground_weight = foreground_coverage * content_opacity;
        let alpha = (base_weight + foreground_weight).clamp(0.0, 1.0);
        let red = background.0 as f64 * base_weight + foreground_color.0 as f64 * foreground_weight;
        let green =
            background.1 as f64 * base_weight + foreground_color.1 as f64 * foreground_weight;
        let blue =
            background.2 as f64 * base_weight + foreground_color.2 as f64 * foreground_weight;
        pixel[0] = blue.round().clamp(0.0, 255.0) as u8;
        pixel[1] = green.round().clamp(0.0, 255.0) as u8;
        pixel[2] = red.round().clamp(0.0, 255.0) as u8;
        pixel[3] = (alpha * 255.0).round() as u8;
    }
}

fn font_face() -> &'static str {
    "Segoe UI Variable Display"
}

fn draw_clock(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    rect: &RECT,
    state: &ClockState,
    dpi_scale: f64,
    background_override: Option<Color>,
) {
    let height = rect.bottom - rect.top;
    let text_scale = state.style.scale.clamp(0.3, 2.0) * dpi_scale;
    let skin = state.style.skin.as_str();
    let accent = parse_color(&state.style.color);
    let font_face = font_face();
    let (background, foreground, muted, border) = skin_colors(skin);

    unsafe {
        let brush = CreateSolidBrush(background_override.unwrap_or(background).ref_value());
        let _ = FillRect(hdc, rect, brush);
        let _ = DeleteObject(brush);
        if skin != "mini" {
            draw_border(
                hdc,
                rect,
                border,
                state.flash_until.is_some(),
                dpi_scale,
            );
        }
        SetBkMode(hdc, TRANSPARENT as i32);
    }

    let now = Local::now();
    let alarm_remaining = state
        .snapshot
        .next
        .as_ref()
        .map(|next| next.at_secs - now.timestamp());
    let alarm_soon = alarm_remaining
        .is_some_and(|seconds| (0..=super::ALARM_WARNING_SECS).contains(&seconds));
    let final_minute = state.snapshot.alarm_flash_enabled
        && alarm_remaining.is_some_and(|seconds| (0..=60).contains(&seconds));
    let time_color = if final_minute && !state.alert_phase {
        alarm_flash_color(accent)
    } else {
        accent
    };
    let alarm_status = if alarm_soon {
        alarm_remaining.map(|remaining| {
            format!(
                "{:02}:{:02}",
                remaining / 60,
                remaining % 60
            )
        })
    } else {
        None
    };
    if skin == "pomodoro" {
        let urgent = alarm_remaining
            .filter(|seconds| (0..=super::ALARM_WARNING_SECS).contains(seconds));
        if let Some(remaining) = urgent {
            let countdown = format!("{:02}:{:02}", remaining / 60, remaining % 60);
            let time_height = (62.0 * text_scale) as i32;
            let label_height = (18.0 * dpi_scale) as i32;
            let gap = (2.0 * dpi_scale) as i32;
            let top = ((height - time_height - label_height - gap) / 2).max(0);
            draw_centered(
                hdc,
                &countdown,
                rect,
                top,
                time_height,
                (56.0 * text_scale) as i32,
                accent,
                state.style.font_weight,
                font_face,
            );
            let label = state
                .snapshot
                .next
                .as_ref()
                .and_then(|next| {
                    state
                        .snapshot
                        .alarms
                        .iter()
                        .find(|alarm| alarm.id == next.alarm_id)
                })
                .map(|alarm| alarm.label.as_str())
                .unwrap_or("\u{95f9}\u{949f}");
            draw_centered(
                hdc,
                &format!("{label} \u{5373}\u{5c06}\u{54cd}\u{8d77}"),
                rect,
                top + time_height + gap,
                label_height,
                (13.0 * dpi_scale) as i32,
                foreground,
                FW_NORMAL as u16,
                font_face,
            );
        } else {
            draw_pomodoro(
                hdc, rect, state, accent, foreground, muted, text_scale, dpi_scale, font_face,
            );
        }
        return;
    }

    let hour = format!("{:02}", now.hour());
    let minute = format!("{:02}", now.minute());
    let seconds = format!("{:02}", now.second());
    if state.style.mini_layout == "vertical" {
        let mut parts = vec![hour.as_str(), minute.as_str()];
        if state.style.mini_time_format == "hms" {
            parts.push(seconds.as_str());
        }
        let line_height = (43.0 * text_scale) as i32;
        let separator_height = (14.0 * text_scale).round().max(8.0) as i32;
        let total_height = line_height * parts.len() as i32
            + separator_height * parts.len().saturating_sub(1) as i32;
        let status_font_size = 14.0 * text_scale;
        let status_height = (status_font_size.round() as i32 + 2).max(10);
        let status_gap = (2.0 * text_scale).round().max(1.0) as i32;
        let status_extra = if alarm_status.is_some() {
            status_height + status_gap
        } else {
            0
        };
        let mut top = ((height - total_height - status_extra) / 2).max(0);
        for (index, part) in parts.iter().enumerate() {
            draw_centered(
                hdc,
                *part,
                rect,
                top,
                line_height,
                (38.0 * text_scale) as i32,
                time_color,
                state.style.font_weight,
                font_face,
            );
            top += line_height;
            if index + 1 < parts.len() {
                draw_centered(
                    hdc,
                    ":",
                    rect,
                    top,
                    separator_height,
                    (18.0 * text_scale) as i32,
                    time_color,
                    state.style.font_weight,
                    font_face,
                );
                top += separator_height;
            }
        }
        if let Some(label) = alarm_status.as_deref() {
            draw_centered(
                hdc,
                label,
                rect,
                top + status_gap,
                status_height,
                status_font_size.round() as i32,
                Color(255, 191, 105),
                FW_NORMAL as u16,
                font_face,
            );
        }
    } else {
        let time = if state.style.mini_time_format == "hms" {
            format!("{hour}:{minute}:{seconds}")
        } else {
            format!("{hour}:{minute}")
        };
        if let Some(label) = alarm_status.as_deref() {
            let time_height = (50.0 * text_scale) as i32;
            let status_font_size = 16.0 * text_scale;
            let status_height = (status_font_size.round() as i32 + 2).max(10);
            let status_gap = (2.0 * text_scale).round().max(1.0) as i32;
            let content_height = time_height + status_gap + status_height;
            let top = ((height - content_height) / 2).max(0);
            draw_centered(
                hdc,
                &time,
                rect,
                top,
                time_height,
                (46.0 * text_scale) as i32,
                time_color,
                state.style.font_weight,
                font_face,
            );
            draw_centered(
                hdc,
                label,
                rect,
                top + time_height + status_gap,
                status_height,
                status_font_size.round() as i32,
                Color(255, 191, 105),
                FW_NORMAL as u16,
                font_face,
            );
        } else {
            draw_centered(
                hdc,
                &time,
                rect,
                0,
                height,
                (46.0 * text_scale) as i32,
                accent,
                state.style.font_weight,
                font_face,
            );
        }
    }
}

fn draw_border(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    rect: &RECT,
    color: Color,
    flashing: bool,
    dpi_scale: f64,
) {
    let (width, height) = (rect.right, rect.bottom);
    let border = if flashing { Color(255, 110, 96) } else { color };
    unsafe {
        let pen = CreatePen(
            PS_SOLID,
            dpi_scale.round().max(1.0) as i32,
            border.ref_value(),
        );
        let old_pen = SelectObject(hdc, pen);
        let null_brush = GetStockObject(5); // NULL_BRUSH
        let old_brush = SelectObject(hdc, null_brush);
        let radius = (16.0 * dpi_scale).round() as i32;
        let _ = RoundRect(hdc, 0, 0, width, height, radius, radius);
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old_pen);
        let _ = DeleteObject(pen);
    }
}

fn draw_centered(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    text: &str,
    bounds: &RECT,
    y: i32,
    height: i32,
    font_size: i32,
    color: Color,
    weight: u16,
    font_face: &str,
) {
    draw_text(
        hdc,
        text,
        RECT {
            left: 0,
            top: y,
            right: bounds.right,
            bottom: y + height,
        },
        font_size,
        color,
        weight,
        true,
        font_face,
    );
}

fn draw_text(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    text: &str,
    mut rect: RECT,
    font_size: i32,
    color: Color,
    weight: u16,
    centered: bool,
    font_face: &str,
) {
    let text: Vec<u16> = text.encode_utf16().collect();
    let mut face: Vec<u16> = font_face.encode_utf16().collect();
    face.push(0);
    let weight = i32::from(weight.clamp(100, 900));
    unsafe {
        let font = CreateFontW(
            -font_size.max(9),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            OUT_DEFAULT_PRECIS as u32,
            4,
            DEFAULT_PITCH as u32,
            face.as_ptr(),
        );
        let old_font = SelectObject(hdc, font);
        SetTextColor(hdc, color.ref_value());
        let align = if centered { DT_CENTER } else { 0 };
        let _ = DrawTextW(
            hdc,
            text.as_ptr(),
            text.len() as i32,
            &mut rect,
            align | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        let _ = SelectObject(hdc, old_font);
        let _ = DeleteObject(font);
    }
}

struct PomoLayout {
    phase: RECT,
    time: RECT,
    bar: RECT,
    start_button: RECT,
    reset_button: RECT,
}

fn pomo_layout(width: i32, height: i32, style_scale: f64, dpi_scale: f64) -> PomoLayout {
    let phase_height = (16.0 * dpi_scale) as i32;
    let time_height = (51.0 * style_scale * dpi_scale) as i32;
    let bar_height = (5.0 * dpi_scale).round().max(1.0) as i32;
    let button_height = (20.0 * dpi_scale) as i32;
    let gap = (4.0 * dpi_scale) as i32;
    let button_margin = (2.0 * dpi_scale) as i32;
    let total_height =
        phase_height + gap + time_height + gap + bar_height + gap + button_margin + button_height;
    let top = ((height - total_height) / 2).max(0);
    let phase = RECT {
        left: 0,
        top,
        right: width,
        bottom: top + phase_height,
    };
    let time_top = phase.bottom + gap;
    let time = RECT {
        left: 0,
        top: time_top,
        right: width,
        bottom: time_top + time_height,
    };
    let bar_top = time.bottom + gap;
    let bar_left = width / 10;
    let bar = RECT {
        left: bar_left,
        top: bar_top,
        right: width - bar_left,
        bottom: bar_top + bar_height,
    };
    let button_top = bar.bottom + gap + button_margin;
    let button_width = (58.0 * dpi_scale) as i32;
    let button_gap = (8.0 * dpi_scale) as i32;
    let group_width = button_width * 2 + button_gap;
    let button_left = (width - group_width) / 2;
    let start_button = RECT {
        left: button_left,
        top: button_top,
        right: button_left + button_width,
        bottom: button_top + button_height,
    };
    let reset_button = RECT {
        left: start_button.right + button_gap,
        top: button_top,
        right: start_button.right + button_gap + button_width,
        bottom: button_top + button_height,
    };
    PomoLayout {
        phase,
        time,
        bar,
        start_button,
        reset_button,
    }
}

fn pomo_button_at(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    style_scale: f64,
    dpi_scale: f64,
) -> Option<bool> {
    let layout = pomo_layout(width, height, style_scale, dpi_scale);
    if x >= layout.start_button.left
        && x <= layout.start_button.right
        && y >= layout.start_button.top
        && y <= layout.start_button.bottom
    {
        Some(false)
    } else if x >= layout.reset_button.left
        && x <= layout.reset_button.right
        && y >= layout.reset_button.top
        && y <= layout.reset_button.bottom
    {
        Some(true)
    } else {
        None
    }
}

fn draw_pomodoro(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    rect: &RECT,
    state: &ClockState,
    accent: Color,
    foreground: Color,
    muted: Color,
    text_scale: f64,
    dpi_scale: f64,
    font_face: &str,
) {
    let layout = pomo_layout(rect.right, rect.bottom, state.style.scale, dpi_scale);
    let phase = if state.pomo.phase_work {
        "\u{4e13}\u{6ce8}\u{4e2d}"
    } else {
        "\u{4f11}\u{606f}\u{4e2d}"
    };
    let phase_text = format!(
        "{phase} \u{00b7} \u{7b2c} {} \u{8f6e}",
        state.pomo.rounds + 1
    );
    draw_centered(
        hdc,
        &phase_text,
        rect,
        layout.phase.top,
        layout.phase.bottom - layout.phase.top,
        (11.0 * dpi_scale) as i32,
        muted,
        FW_NORMAL as u16,
        font_face,
    );

    let remaining = state.pomo.remaining.max(0);
    draw_centered(
        hdc,
        &format!("{:02}:{:02}", remaining / 60, remaining % 60),
        rect,
        layout.time.top,
        layout.time.bottom - layout.time.top,
        (46.0 * text_scale) as i32,
        accent,
        500,
        font_face,
    );

    let total = i64::from(
        if state.pomo.phase_work {
            state.style.pomo_work_min
        } else {
            state.style.pomo_break_min
        }
        .max(1),
    ) * 60;
    let progress = 1.0 - remaining as f64 / total as f64;
    unsafe {
        let track = CreateSolidBrush(Color(58, 59, 66).ref_value());
        let track_pen = CreatePen(PS_SOLID, 1, Color(58, 59, 66).ref_value());
        let old_pen = SelectObject(hdc, track_pen);
        let old_brush = SelectObject(hdc, track);
        let _ = RoundRect(
            hdc,
            layout.bar.left,
            layout.bar.top,
            layout.bar.right,
            layout.bar.bottom,
            layout.bar.bottom - layout.bar.top,
            layout.bar.bottom - layout.bar.top,
        );
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old_pen);
        let _ = DeleteObject(track_pen);
        let _ = DeleteObject(track);

        let fill_width =
            ((layout.bar.right - layout.bar.left) as f64 * progress.clamp(0.0, 1.0)) as i32;
        if fill_width > 0 {
            let fill = CreateSolidBrush(accent.ref_value());
            let fill_pen = CreatePen(PS_SOLID, 1, accent.ref_value());
            let old_pen = SelectObject(hdc, fill_pen);
            let old_brush = SelectObject(hdc, fill);
            let _ = RoundRect(
                hdc,
                layout.bar.left,
                layout.bar.top,
                layout.bar.left + fill_width,
                layout.bar.bottom,
                layout.bar.bottom - layout.bar.top,
                layout.bar.bottom - layout.bar.top,
            );
            let _ = SelectObject(hdc, old_brush);
            let _ = SelectObject(hdc, old_pen);
            let _ = DeleteObject(fill_pen);
            let _ = DeleteObject(fill);
        }
    }

    let start_label = if state.pomo.running {
        "\u{6682}\u{505c}"
    } else {
        "\u{5f00}\u{59cb}"
    };
    draw_button(
        hdc,
        layout.start_button,
        start_label,
        foreground,
        (11.0 * dpi_scale) as i32,
        dpi_scale,
        font_face,
    );
    draw_button(
        hdc,
        layout.reset_button,
        "\u{91cd}\u{7f6e}",
        foreground,
        (11.0 * dpi_scale) as i32,
        dpi_scale,
        font_face,
    );
}

fn draw_button(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    rect: RECT,
    label: &str,
    color: Color,
    font_size: i32,
    dpi_scale: f64,
    font_face: &str,
) {
    unsafe {
        let pen = CreatePen(
            PS_SOLID,
            (1.0 * dpi_scale).round() as i32,
            Color(100, 101, 111).ref_value(),
        );
        let old_pen = SelectObject(hdc, pen);
        let old_brush = SelectObject(hdc, GetStockObject(5));
        let radius = (7.0 * dpi_scale).round() as i32;
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius.max(1) * 2,
            radius.max(1) * 2,
        );
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old_pen);
        let _ = DeleteObject(pen);
    }
    draw_text(hdc, label, rect, font_size, color, 500, true, font_face);
}
