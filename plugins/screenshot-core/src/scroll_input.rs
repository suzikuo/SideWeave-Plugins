use crate::model::ScreenRect;
use crate::scroll::MatchAxis;

use std::mem::{take, zeroed};
use std::ptr::{null, null_mut};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetAncestor, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WindowFromPoint, HC_ACTION, GA_ROOT,
    MSLLHOOKSTRUCT, MSG, PM_NOREMOVE, WH_MOUSE_LL, WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_QUIT,
};

static ACTIVE_SCROLL_INPUT: Mutex<Option<Arc<Mutex<HookInputState>>>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct WheelInputTotals {
    pub(super) vertical_wheel_units: i64,
    pub(super) horizontal_wheel_units: i64,
    pub(super) shift_wheel_units: i64,
    pub(super) vertical_wheel_events: u32,
    pub(super) horizontal_wheel_events: u32,
    pub(super) shift_wheel_events: u32,
}

impl WheelInputTotals {
    fn has_events(self) -> bool {
        self.vertical_wheel_events > 0
            || self.horizontal_wheel_events > 0
            || self.shift_wheel_events > 0
    }

    fn merge(&mut self, other: Self) {
        self.vertical_wheel_units = self
            .vertical_wheel_units
            .saturating_add(other.vertical_wheel_units);
        self.horizontal_wheel_units = self
            .horizontal_wheel_units
            .saturating_add(other.horizontal_wheel_units);
        self.shift_wheel_units = self
            .shift_wheel_units
            .saturating_add(other.shift_wheel_units);
        self.vertical_wheel_events = self
            .vertical_wheel_events
            .saturating_add(other.vertical_wheel_events);
        self.horizontal_wheel_events = self
            .horizontal_wheel_events
            .saturating_add(other.horizontal_wheel_events);
        self.shift_wheel_events = self
            .shift_wheel_events
            .saturating_add(other.shift_wheel_events);
    }

    fn axis_units(self, axis: MatchAxis) -> i64 {
        match axis {
            MatchAxis::Vertical => self.vertical_wheel_units,
            MatchAxis::Horizontal => self
                .horizontal_wheel_units
                .saturating_add(self.shift_wheel_units),
        }
    }

    fn record(&mut self, horizontal: bool, shift: bool, delta: i64) {
        if horizontal {
            self.horizontal_wheel_units = self.horizontal_wheel_units.saturating_add(delta);
            self.horizontal_wheel_events = self.horizontal_wheel_events.saturating_add(1);
        } else if shift {
            self.shift_wheel_units = self.shift_wheel_units.saturating_add(delta);
            self.shift_wheel_events = self.shift_wheel_events.saturating_add(1);
        } else {
            self.vertical_wheel_units = self.vertical_wheel_units.saturating_add(delta);
            self.vertical_wheel_events = self.vertical_wheel_events.saturating_add(1);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ScrollInputDelta {
    pub(super) axis_units_raw: i64,
    pub(super) target_window_axis_units_raw: i64,
    pub(super) in_selection: WheelInputTotals,
    /// Events inside the foreground target window but outside the capture rectangle.
    pub(super) target_window_outside_selection: WheelInputTotals,
    pub(super) outside_selection_events: u32,
    pub(super) outside_target_window_events: u32,
    pub(super) hook_processing_count: u32,
    pub(super) hook_processing_total_us: u64,
    pub(super) hook_processing_max_us: u64,
    pub(super) interval_elapsed_ms: u64,
    pub(super) first_event_tick_ms: Option<u64>,
    pub(super) last_event_tick_ms: Option<u64>,
}

impl ScrollInputDelta {
    pub(super) fn has_events(self) -> bool {
        self.in_selection.has_events()
            || self.target_window_outside_selection.has_events()
            || self.outside_selection_events > 0
    }

    pub(super) fn has_in_selection_events(self) -> bool {
        self.in_selection.has_events()
    }

    pub(super) fn merge(&mut self, other: Self) {
        self.axis_units_raw = self.axis_units_raw.saturating_add(other.axis_units_raw);
        self.target_window_axis_units_raw = self
            .target_window_axis_units_raw
            .saturating_add(other.target_window_axis_units_raw);
        self.in_selection.merge(other.in_selection);
        self.target_window_outside_selection
            .merge(other.target_window_outside_selection);
        self.outside_selection_events = self
            .outside_selection_events
            .saturating_add(other.outside_selection_events);
        self.outside_target_window_events = self
            .outside_target_window_events
            .saturating_add(other.outside_target_window_events);
        self.hook_processing_count = self
            .hook_processing_count
            .saturating_add(other.hook_processing_count);
        self.hook_processing_total_us = self
            .hook_processing_total_us
            .saturating_add(other.hook_processing_total_us);
        self.hook_processing_max_us = self
            .hook_processing_max_us
            .max(other.hook_processing_max_us);
        self.interval_elapsed_ms = self
            .interval_elapsed_ms
            .saturating_add(other.interval_elapsed_ms);
        self.first_event_tick_ms = match (self.first_event_tick_ms, other.first_event_tick_ms) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (left, right) => left.or(right),
        };
        self.last_event_tick_ms = match (self.last_event_tick_ms, other.last_event_tick_ms) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (left, right) => left.or(right),
        };
    }
}

struct HookInputState {
    selection: (i32, i32, i32, i32),
    target_window_root: isize,
    axis: MatchAxis,
    pending: ScrollInputDelta,
    last_drained_tick_ms: u64,
}

pub(super) struct ScrollInputListener {
    thread_id: u32,
    worker: Option<JoinHandle<()>>,
    state: Arc<Mutex<HookInputState>>,
}

impl ScrollInputListener {
    pub(super) fn install(
        selection: ScreenRect,
        target_window: HWND,
        axis: MatchAxis,
    ) -> Result<Self, String> {
        let now = unsafe { GetTickCount64() };
        let mut target_window_root = if target_window.is_null() {
            null_mut()
        } else {
            unsafe { GetAncestor(target_window, GA_ROOT) }
        };
        if target_window_root.is_null() {
            target_window_root = target_window;
        }
        let state = Arc::new(Mutex::new(HookInputState {
            selection: (selection.left, selection.top, selection.right, selection.bottom),
            target_window_root: target_window_root as isize,
            axis,
            pending: ScrollInputDelta::default(),
            last_drained_tick_ms: now,
        }));
        {
            let mut active = ACTIVE_SCROLL_INPUT
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if active.is_some() {
                return Err("已有滚动输入监听会话".into());
            }
            *active = Some(Arc::clone(&state));
        }

        let callback_state = Arc::clone(&state);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = match thread::Builder::new()
            .name("screenshot-scroll-input".into())
            .spawn(move || unsafe {
                let thread_id = GetCurrentThreadId();
                let mut message: MSG = zeroed();
                PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
                let hook = SetWindowsHookExW(
                    WH_MOUSE_LL,
                    Some(scroll_input_hook_proc),
                    GetModuleHandleW(null()),
                    0,
                );
                if hook.is_null() {
                    clear_active_state(&callback_state);
                    let _ = ready_tx.send(Err("SetWindowsHookExW 安装失败".into()));
                    return;
                }
                if ready_tx.send(Ok(thread_id)).is_err() {
                    UnhookWindowsHookEx(hook);
                    clear_active_state(&callback_state);
                    return;
                }

                loop {
                    let result = GetMessageW(&mut message, null_mut(), 0, 0);
                    if result <= 0 {
                        break;
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }

                UnhookWindowsHookEx(hook);
                clear_active_state(&callback_state);
            })
        {
            Ok(worker) => worker,
            Err(error) => {
                clear_active_state(&state);
                return Err(format!("无法启动滚动输入监听线程：{error}"));
            }
        };

        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                worker: Some(worker),
                state,
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                clear_active_state(&state);
                let _ = worker.join();
                Err(format!("滚动输入监听线程提前退出：{error}"))
            }
        }
    }

    pub(super) fn drain(&self) -> ScrollInputDelta {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = unsafe { GetTickCount64() };
        let mut delta = take(&mut state.pending);
        delta.interval_elapsed_ms = now.saturating_sub(state.last_drained_tick_ms);
        state.last_drained_tick_ms = now;
        delta.axis_units_raw = delta.in_selection.axis_units(state.axis);
        delta.target_window_axis_units_raw =
            delta.target_window_outside_selection.axis_units(state.axis);
        delta
    }
}

impl Drop for ScrollInputListener {
    fn drop(&mut self) {
        clear_active_state(&self.state);
        let posted = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) } != 0;
        if let Some(worker) = self.worker.take() {
            if posted || worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

fn clear_active_state(state: &Arc<Mutex<HookInputState>>) {
    let mut active = ACTIVE_SCROLL_INPUT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if active
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, state))
    {
        *active = None;
    }
}

unsafe extern "system" fn scroll_input_hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code == HC_ACTION as i32
        && (wparam as u32 == WM_MOUSEWHEEL || wparam as u32 == WM_MOUSEHWHEEL)
    {
        let callback_started = std::time::Instant::now();
        let event = &*(lparam as *const MSLLHOOKSTRUCT);
        let shift_down = (GetAsyncKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0;
        let delta = ((event.mouseData >> 16) as u16 as i16) as i64;
        let tick_ms = GetTickCount64();
        let state = ACTIVE_SCROLL_INPUT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .cloned();

        if let Some(state) = state {
            let (selection, target_window_root) = {
                let state = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                (state.selection, state.target_window_root as HWND)
            };
            let (left, top, right, bottom) = selection;
            let inside = event.pt.x >= left
                && event.pt.x < right
                && event.pt.y >= top
                && event.pt.y < bottom;
            let inside_target_window = if inside {
                false
            } else {
                let point = POINT {
                    x: event.pt.x,
                    y: event.pt.y,
                };
                let hovered_window = WindowFromPoint(point);
                !target_window_root.is_null()
                    && !hovered_window.is_null()
                    && GetAncestor(hovered_window, GA_ROOT) == target_window_root
            };
            let mut state = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if inside {
                if state.pending.first_event_tick_ms.is_none() {
                    state.pending.first_event_tick_ms = Some(tick_ms);
                }
                state.pending.last_event_tick_ms = Some(tick_ms);
                state.pending.in_selection.record(
                    wparam as u32 == WM_MOUSEHWHEEL,
                    shift_down,
                    delta,
                );
            } else {
                state.pending.outside_selection_events = state
                    .pending
                    .outside_selection_events
                    .saturating_add(1);
                if inside_target_window {
                    if state.pending.first_event_tick_ms.is_none() {
                        state.pending.first_event_tick_ms = Some(tick_ms);
                    }
                    state.pending.last_event_tick_ms = Some(tick_ms);
                    state.pending.target_window_outside_selection.record(
                        wparam as u32 == WM_MOUSEHWHEEL,
                        shift_down,
                        delta,
                    );
                } else {
                    state.pending.outside_target_window_events = state
                        .pending
                        .outside_target_window_events
                        .saturating_add(1);
                }
            }
            let processing_us = callback_started
                .elapsed()
                .as_micros()
                .min(u64::MAX as u128) as u64;
            state.pending.hook_processing_count =
                state.pending.hook_processing_count.saturating_add(1);
            state.pending.hook_processing_total_us = state
                .pending
                .hook_processing_total_us
                .saturating_add(processing_us);
            state.pending.hook_processing_max_us =
                state.pending.hook_processing_max_us.max(processing_us);
        }
    }

    CallNextHookEx(null_mut(), code, wparam, lparam)
}
