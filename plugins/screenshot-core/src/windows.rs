mod draw;
mod editor;
mod input;
mod ocr_ui;
mod output;
mod paint;
mod recording_ui;
mod scroll_ui;
mod toolbar_paint;

use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Instant;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{HFONT, InvalidateRect};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::SystemServices::MK_LBUTTON;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, RegisterHotKey, ReleaseCapture, SetCapture, SetFocus, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowLongPtrW, KillTimer, LoadCursorW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetCursor, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TranslateMessage, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW, MSG, SW_HIDE,
    SetLayeredWindowAttributes, SW_SHOW, WM_APP, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
    WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_NCCREATE, WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, GWL_EXSTYLE, LWA_ALPHA, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
};

use crate::capture::ScreenCapture;
use crate::model::{ScreenPoint, ScreenRect, Settings, StatusSnapshot};
use crate::recorder::{AnimationFormat, RecordingManager};
use crate::scroll::ScrollMotionModel;
use crate::scroll_input::{ScrollInputDelta, ScrollInputListener};
use crate::session::CaptureSession;
use crate::tencent_ocr::TencentCloudCredentials;

use self::editor::EditorCanvas;
use self::input::{
    char_input, key_down, mouse_down, mouse_move, mouse_up, screen_point_from_lparam,
    update_session_cursor, VK_A, VK_ESCAPE_CODE,
};
use self::output::{pin_last, pin_wnd_proc};
use self::paint::{create_overlay_render, paint, paint_editor, OverlayRender};
use self::scroll_ui::{
    create_scroll_border, create_scroll_masks, destroy_scroll_border, destroy_scroll_masks,
    destroy_scroll_preview, scroll_border_wnd_proc, scroll_control_wnd_proc,
    scroll_mask_wnd_proc, scroll_preview_wnd_proc, scroll_tick, scroll_tip_wnd_proc,
    scroll_tooltip_wnd_proc, stop_scroll_input,
};

pub(crate) const DEFAULT_GUI_FONT: i32 = 17;
pub(crate) const WAKE: u32 = WM_APP + 1;
pub(crate) const HOTKEY_ID: i32 = 41;
pub(crate) const SCROLL_ESCAPE_HOTKEY_ID: i32 = 43;
pub(crate) const TIMER_SCROLL: usize = 42;
pub(crate) const SCROLL_OBSERVE_INTERVAL: u32 = 33;
pub(crate) const MOD_ALT: u32 = 0x0001;
pub(crate) const MOD_CONTROL: u32 = 0x0002;
pub(crate) const MOD_SHIFT: u32 = 0x0004;
pub(crate) const MOD_WIN: u32 = 0x0008;
const MOD_NOREPEAT: u32 = 0x4000;

pub(crate) const MAX_SCROLL_EXTENT: usize = 60000;
pub(crate) const SCROLL_PREVIEW_WIDTH: i32 = 360;
pub(crate) const SCROLL_PREVIEW_HEIGHT: i32 = 440;
pub(crate) const SCROLL_PREVIEW_MARGIN: i32 = 18;
pub(crate) const SCROLL_PREVIEW_GAP: i32 = 8;
pub(crate) const SCROLL_MASK_ALPHA: u8 = 112;
pub(crate) const SCROLL_TIP_WIDTH: i32 = 400;
pub(crate) const SCROLL_TIP_HEIGHT: i32 = 64;
pub(crate) const SCROLL_TIP_MARGIN: i32 = 12;
pub(crate) type ImageStore = Arc<Mutex<Option<(usize, usize, Vec<u8>)>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScrollPreviewFeedback {
    Waiting,
    Matching,
    Matched,
    Rejected,
}

pub(crate) struct UiState {
    pub(crate) hwnd: HWND,
    pub(crate) overlay: HWND,
    pub(crate) pin_window: HWND,
    pub(crate) scroll_control: HWND,
    pub(crate) scroll_tooltip: HWND,
    pub(crate) scroll_tooltip_text: Option<String>,
    pub(crate) scroll_toolbar_tracking_mouse: bool,
    pub(crate) scroll_preview: HWND,
    pub(crate) scroll_preview_feedback: ScrollPreviewFeedback,
    pub(crate) scroll_tip: HWND,
    pub(crate) scroll_tip_text: Option<String>,
    pub(crate) ocr_result_window: HWND,
    pub(crate) ocr_result_edit: HWND,
    pub(crate) ocr_result_font: HFONT,
    pub(crate) ocr_result_copied: bool,
    pub(crate) ocr_result_title: &'static str,
    pub(crate) scroll_masks: Vec<HWND>,
    pub(crate) scroll_border: HWND,
    pub(crate) scroll_target: HWND,
    pub(crate) settings: Arc<Mutex<Settings>>,
    pub(crate) tencent_credentials: Arc<Mutex<Option<TencentCloudCredentials>>>,
    command_tx: mpsc::Sender<Command>,
    pub(crate) ocr_pending: bool,
    ocr_generation: u64,
    pub(crate) image: ImageStore,
    pub(crate) status: Arc<Mutex<StatusSnapshot>>,
    pub(crate) screen: Option<ScreenCapture>,
    pub(crate) render: Option<OverlayRender>,
    pub(crate) capture: Option<CaptureSession>,
    pub(crate) editor: Option<EditorCanvas>,
    pub(crate) last_path: Option<String>,
    pub(crate) last_size: Option<(usize, usize)>,
    pub(crate) last_image: Option<(usize, usize, Vec<u8>)>,
    pub(crate) pin_image: Option<(usize, usize, Vec<u8>)>,
    pub(crate) hotkey: HotkeySpec,
    pub(crate) hotkey_registered: bool,
    pub(crate) scroll_escape_hotkey_registered: bool,
    pub(crate) paint_pending: bool,
    pub(crate) last_hover_query: Option<(ScreenPoint, Instant)>,
    pub(crate) scroll_input: Option<ScrollInputListener>,
    pub(crate) pending_scroll_input: ScrollInputDelta,
    pub(crate) scroll_motion_model: ScrollMotionModel,
    pub(crate) recording: RecordingManager,
    pub(crate) recording_requested_fps: Option<u32>,
    pub(crate) recording_setup_format: AnimationFormat,
    pub(crate) recording_show_cursor: bool,
    pub(crate) recording_toolbar: HWND,
    pub(crate) recording_toolbar_drag: Option<(ScreenPoint, ScreenPoint)>,
    pub(crate) recording_toolbar_signature: Option<String>,
    pub(crate) recording_toolbar_static_signature: Option<String>,
    pub(crate) recording_editor_pending: bool,
    pub(crate) recording_overlay: bool,
}

pub(crate) fn configured_recording_fps(
    settings: &Arc<Mutex<Settings>>,
    format: AnimationFormat,
) -> u32 {
    let Ok(settings) = settings.lock() else {
        return 15;
    };
    match format {
        AnimationFormat::Mp4 => settings.recording_mp4_fps,
        AnimationFormat::Gif => settings.recording_gif_fps,
        AnimationFormat::Webp => settings.recording_webp_fps,
    }
}

pub(crate) fn configured_mp4_recording_options(
    settings: &Arc<Mutex<Settings>>,
) -> (u32, u32) {
    let Ok(settings) = settings.lock() else {
        return (1080, 16);
    };
    (
        settings.recording_mp4_max_height,
        settings.recording_mp4_bitrate_mbps,
    )
}

#[derive(Clone, Copy)]
pub(crate) struct HotkeySpec {
    pub(crate) modifiers: u32,
    pub(crate) key: u32,
}

impl Default for HotkeySpec {
    fn default() -> Self {
        Self {
            modifiers: MOD_CONTROL | MOD_SHIFT,
            key: VK_A,
        }
    }
}

enum Command {
    Start(bool),
    StartRecording(u32),
    StopRecording,
    CancelRecording,
    ExportRecording(AnimationFormat),
    ClearRecording,
    Cancel,
    Shutdown,
    OcrFinished {
        generation: u64,
        result: Result<String, String>,
    },
    QrFinished {
        generation: u64,
        result: Result<String, String>,
    },
    Hotkey(String),
    Pin,
}

pub struct UiBridge {
    hwnd: Arc<Mutex<isize>>,
    tx: mpsc::Sender<Command>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl UiBridge {
    pub fn start(&self, scroll: bool) {
        self.send(Command::Start(scroll));
    }

    pub fn start_recording(&self, fps: u32) {
        self.send(Command::StartRecording(fps));
    }

    pub fn stop_recording(&self) {
        self.send(Command::StopRecording);
    }

    pub fn cancel_recording(&self) {
        self.send(Command::CancelRecording);
    }

    pub fn export_recording(&self, format: AnimationFormat) {
        self.send(Command::ExportRecording(format));
    }

    pub fn clear_recording(&self) {
        self.send(Command::ClearRecording);
    }

    pub fn cancel(&self) {
        self.send(Command::Cancel);
    }

    pub fn shutdown(&self) {
        self.send(Command::Shutdown);
        if let Ok(mut thread) = self.thread.lock() {
            if let Some(handle) = thread.take() {
                let _ = handle.join();
            }
        }
    }

    pub fn update_hotkey(&self, value: &str) {
        self.send(Command::Hotkey(value.to_string()));
    }

    pub fn pin_last(&self) {
        self.send(Command::Pin);
    }

    fn send(&self, command: Command) {
        let _ = self.tx.send(command);
        let hwnd = *self
            .hwnd
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) as HWND;
        if !hwnd.is_null() {
            unsafe {
                PostMessageW(hwnd, WAKE, 0, 0);
            }
        }
    }
}

impl Drop for UiBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn start(
    settings: Arc<Mutex<Settings>>,
    image: ImageStore,
    status: Arc<Mutex<StatusSnapshot>>,
    recording: RecordingManager,
    tencent_credentials: Arc<Mutex<Option<TencentCloudCredentials>>>,
) -> UiBridge {
    let (tx, rx) = mpsc::channel();
    let ui_tx = tx.clone();
    let (init_tx, init_rx) = mpsc::sync_channel(1);
    let hwnd = Arc::new(Mutex::new(0isize));
    let shared_hwnd = Arc::clone(&hwnd);
    let thread = thread::spawn(move || {
        ui_thread(
            settings,
            image,
            status,
            recording,
            tencent_credentials,
            rx,
            ui_tx,
            shared_hwnd,
            init_tx,
        );
    });
    let _ = init_rx.recv();
    UiBridge {
        hwnd,
        tx,
        thread: Mutex::new(Some(thread)),
    }
}

#[allow(clippy::manual_c_str_literals)]
pub fn enable_dpi_awareness() {
    unsafe {
        type SetProcessDpiAwarenessContextFn =
            unsafe extern "system" fn(isize) -> windows_sys::Win32::Foundation::BOOL;
        type SetProcessDPIAwareFn =
            unsafe extern "system" fn() -> windows_sys::Win32::Foundation::BOOL;
        let module = LoadLibraryW(wide("user32.dll").as_ptr());
        if !module.is_null() {
            if let Some(proc) = GetProcAddress(module, b"SetProcessDpiAwarenessContext\0".as_ptr())
            {
                let func: SetProcessDpiAwarenessContextFn = std::mem::transmute(proc);
                if func(-4) != 0 {
                    return;
                }
            }
            if let Some(proc) = GetProcAddress(module, b"SetProcessDPIAware\0".as_ptr()) {
                let func: SetProcessDPIAwareFn = std::mem::transmute(proc);
                let _ = func();
            }
        }
    }
}

fn ui_thread(
    settings: Arc<Mutex<Settings>>,
    image: ImageStore,
    status: Arc<Mutex<StatusSnapshot>>,
    recording: RecordingManager,
    tencent_credentials: Arc<Mutex<Option<TencentCloudCredentials>>>,
    rx: mpsc::Receiver<Command>,
    command_tx: mpsc::Sender<Command>,
    shared_hwnd: Arc<Mutex<isize>>,
    init_tx: mpsc::SyncSender<()>,
) {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class_name.as_ptr(),
            ..zeroed()
        };
        RegisterClassW(&class);

        let state = Box::new(UiState {
            hwnd: null_mut(),
            overlay: null_mut(),
            pin_window: null_mut(),
            scroll_control: null_mut(),
            scroll_tooltip: null_mut(),
            scroll_tooltip_text: None,
            scroll_toolbar_tracking_mouse: false,
            scroll_preview: null_mut(),
            scroll_preview_feedback: ScrollPreviewFeedback::Waiting,
            scroll_tip: null_mut(),
            scroll_tip_text: None,
            ocr_result_window: null_mut(),
            ocr_result_edit: null_mut(),
            ocr_result_font: null_mut(),
            ocr_result_copied: false,
            ocr_result_title: "文字提取结果",
            scroll_masks: Vec::new(),
            scroll_border: null_mut(),
            scroll_target: null_mut(),
            settings: Arc::clone(&settings),
            tencent_credentials,
            command_tx,
            ocr_pending: false,
            ocr_generation: 0,
            image,
            status,
            screen: None,
            render: None,
            capture: None,
            editor: None,
            last_path: None,
            last_size: None,
            last_image: None,
            pin_image: None,
            hotkey: HotkeySpec::default(),
            hotkey_registered: false,
            scroll_escape_hotkey_registered: false,
            paint_pending: false,
            last_hover_query: None,
            scroll_input: None,
            pending_scroll_input: ScrollInputDelta::default(),
            scroll_motion_model: ScrollMotionModel::default(),
            recording,
            recording_requested_fps: None,
            recording_setup_format: AnimationFormat::Mp4,
            recording_show_cursor: true,
            recording_toolbar: null_mut(),
            recording_toolbar_drag: None,
            recording_toolbar_signature: None,
            recording_toolbar_static_signature: None,
            recording_editor_pending: false,
            recording_overlay: false,
        });
        let state_ptr = Box::into_raw(state);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1,
            1,
            null_mut(),
            null_mut(),
            instance,
            state_ptr as *const c_void,
        );
        let state = &mut *state_ptr;
        state.hwnd = hwnd;
        *shared_hwnd
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = hwnd as isize;

        if let Some(spec) = state
            .settings
            .lock()
            .ok()
            .and_then(|value| parse_hotkey(&value.hotkey).ok())
        {
            state.hotkey = spec;
        }
        state.hotkey_registered = register_hotkey(hwnd, state.hotkey);
        let _ = init_tx.send(());
        update_status(state, false);

        let mut message: MSG = zeroed();
        loop {
            let code = GetMessageW(&mut message, null_mut(), 0, 0);
            if code <= 0 {
                break;
            }
            if message.message == WAKE {
                while let Ok(command) = rx.try_recv() {
                    handle_command(state, command);
                }
            } else if message.message == WM_KEYDOWN
                && message.wParam as u32 == VK_ESCAPE_CODE
                && !state.ocr_result_window.is_null()
                && (message.hwnd == state.ocr_result_window
                    || message.hwnd == state.ocr_result_edit)
            {
                DestroyWindow(state.ocr_result_window);
            } else {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        *shared_hwnd
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = 0;

        stop_scroll_input(state);
        state.recording.shutdown();
        if state.hotkey_registered {
            UnregisterHotKey(hwnd, HOTKEY_ID);
        }
        unregister_scroll_escape_hotkey(state);
        if !state.overlay.is_null() {
            DestroyWindow(state.overlay);
        }
        if !state.pin_window.is_null() {
            DestroyWindow(state.pin_window);
        }
        if !state.scroll_control.is_null() {
            DestroyWindow(state.scroll_control);
        }
        if !state.scroll_preview.is_null() {
            DestroyWindow(state.scroll_preview);
        }
        if !state.scroll_tip.is_null() {
            DestroyWindow(state.scroll_tip);
        }
        if !state.ocr_result_window.is_null() {
            DestroyWindow(state.ocr_result_window);
        }
        destroy_scroll_masks(state);
        if !state.scroll_border.is_null() {
            DestroyWindow(state.scroll_border);
        }
        recording_ui::destroy_control_toolbar(state);
        drop(Box::from_raw(state_ptr));
    }
}

fn handle_command(state: &mut UiState, command: Command) {
    match command {
        Command::Start(scroll) => begin_capture(state, scroll),
        Command::StartRecording(fps) => start_recording_selection(state, fps),
        Command::StopRecording => stop_recording(state),
        Command::CancelRecording => {
            state.recording.cancel();
            state.recording_requested_fps = None;
            cancel(state);
        }
        Command::ExportRecording(format) => recording_ui::export_recording(state, format),
        Command::ClearRecording => {
            state.recording.cancel();
            state.recording_requested_fps = None;
        }
        Command::Cancel => cancel(state),
        Command::Shutdown => {
            cancel(state);
            unsafe {
                PostQuitMessage(0);
            }
        }
        Command::OcrFinished { generation, result } => {
            ocr_ui::finish_text_extraction(state, generation, result)
        }
        Command::QrFinished { generation, result } => {
            ocr_ui::finish_qr_recognition(state, generation, result)
        }
        Command::Hotkey(value) => update_hotkey(state, &value),
        Command::Pin => pin_last(state),
    }
}

fn parse_hotkey(value: &str) -> Result<HotkeySpec, String> {
    let mut modifiers = 0;
    let mut key = None;
    for token in value
        .split('+')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        match token.to_ascii_lowercase().as_str() {
            "alt" => modifiers |= MOD_ALT,
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "meta" | "cmd" | "command" | "windows" => modifiers |= MOD_WIN,
            token if key.is_none() => key = parse_key(token),
            _ => return Err(format!("无法识别快捷键：{value}")),
        }
    }
    key.map(|key| HotkeySpec { modifiers, key })
        .ok_or_else(|| "快捷键必须包含按键".into())
}

fn parse_key(value: &str) -> Option<u32> {
    if value.len() == 1 {
        let byte = value.as_bytes()[0];
        return byte
            .is_ascii_alphanumeric()
            .then_some(byte.to_ascii_uppercase() as u32);
    }
    value
        .strip_prefix('f')
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| (1..=12).contains(value))
        .map(|value| 0x70 + value - 1)
}

fn update_hotkey(state: &mut UiState, value: &str) {
    let next = match parse_hotkey(value) {
        Ok(value) => value,
        Err(error) => {
            set_error(state, error);
            return;
        }
    };
    if state.hotkey_registered {
        unsafe {
            UnregisterHotKey(state.hwnd, HOTKEY_ID);
        }
    }
    state.hotkey_registered = register_hotkey(state.hwnd, next);
    if state.hotkey_registered {
        state.hotkey = next;
        clear_error(state);
    } else {
        state.hotkey_registered = register_hotkey(state.hwnd, state.hotkey);
        set_error(state, "快捷键已被其他应用占用".into());
    }
}

fn register_hotkey(hwnd: HWND, hotkey: HotkeySpec) -> bool {
    unsafe {
        RegisterHotKey(
            hwnd,
            HOTKEY_ID,
            hotkey.modifiers | MOD_NOREPEAT,
            hotkey.key,
        ) != 0
    }
}

pub(crate) fn register_scroll_escape_hotkey(state: &mut UiState) -> bool {
    if state.scroll_escape_hotkey_registered {
        return true;
    }
    state.scroll_escape_hotkey_registered = unsafe {
        RegisterHotKey(
            state.hwnd,
            SCROLL_ESCAPE_HOTKEY_ID,
            MOD_NOREPEAT,
            VK_ESCAPE_CODE,
        ) != 0
    };
    state.scroll_escape_hotkey_registered
}

pub(crate) fn unregister_scroll_escape_hotkey(state: &mut UiState) {
    if state.scroll_escape_hotkey_registered {
        unsafe {
            UnregisterHotKey(state.hwnd, SCROLL_ESCAPE_HOTKEY_ID);
        }
        state.scroll_escape_hotkey_registered = false;
    }
}

fn current_color(state: &UiState) -> [u8; 4] {
    state
        .settings
        .lock()
        .map(|settings| settings.annotation_rgba())
        .unwrap_or([255, 58, 72, 255])
}

fn text_size(state: &UiState) -> u32 {
    state
        .settings
        .lock()
        .map(|settings| settings.text_size)
        .unwrap_or(18)
}

fn begin_capture(state: &mut UiState, scrolling: bool) {
    if state.capture.is_some() {
        return;
    }
    let Some(screen) = ScreenCapture::grab_virtual_desktop() else {
        if state.recording_requested_fps.is_some() {
            state.recording.fail("\u{65e0}\u{6cd5}\u{8bfb}\u{53d6}\u{865a}\u{62df}\u{684c}\u{9762}\u{56fe}\u{50cf}".into());
            state.recording_requested_fps = None;
        }
        set_error(state, "无法读取虚拟桌面图像".into());
        return;
    };
    let target_window = unsafe { GetForegroundWindow() };
    let screen_rect = ScreenRect {
        left: screen.x,
        top: screen.y,
        right: screen.x + screen.width,
        bottom: screen.y + screen.height,
    };
    let initial_color = current_color(state);
    let initial_text_size = text_size(state);
    let capture = CaptureSession::new(screen_rect, initial_color, initial_text_size, scrolling);
    state.capture = Some(capture);
    state.scroll_target = target_window;
    state.last_hover_query = None;
    state.screen = Some(screen);
    unsafe {
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let instance = GetModuleHandleW(null());
        state.overlay = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            screen_rect.left,
            screen_rect.top,
            screen_rect.width() as i32,
            screen_rect.height() as i32,
            null_mut(),
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        state.render = state
            .screen
            .as_ref()
            .and_then(|screen| create_overlay_render(state.overlay, screen));
        ShowWindow(state.overlay, SW_SHOW);
        SetForegroundWindow(state.overlay);
        SetFocus(state.overlay);
    }
    update_status(state, true);
}

fn set_recording_capture_style(hwnd: HWND, recording: bool) {
    if hwnd.is_null() {
        return;
    }
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let updated = if recording {
            current | WS_EX_LAYERED
        } else {
            current & !WS_EX_LAYERED
        };
        if current == updated {
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, updated as isize);
        if recording {
            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        }
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        );
    }
}

pub(crate) fn restore_recording_selection(state: &mut UiState) {
    state.recording_overlay = false;
    destroy_scroll_masks(state);
    destroy_scroll_border(state);
    set_recording_capture_style(state.overlay, false);
    if !state.overlay.is_null() {
        unsafe {
            ShowWindow(state.overlay, SW_SHOW);
            SetForegroundWindow(state.overlay);
            SetFocus(state.overlay);
        }
    }
    invalidate(state);
}

fn start_recording_selection(state: &mut UiState, fps: u32) {
    if state.capture.is_some() {
        set_error(state, "请先完成或取消当前截图".into());
        return;
    }
    if let Err(error) = state.recording.request_selection(fps) {
        set_error(state, error);
        return;
    }
    state.recording_requested_fps = Some(fps);
    begin_capture(state, false);
}

pub(crate) fn start_recording_selected_region(state: &mut UiState) {
    let Some(fps) = state.recording_requested_fps.take() else {
        return;
    };
    let Some(selection) = state
        .capture
        .as_ref()
        .and_then(|capture| capture.selection)
    else {
        return;
    };
    let Some(screen_bounds) = state.screen.as_ref().map(ScreenCapture::bounds) else {
        state.recording.fail("无法读取录制屏幕范围".into());
        return;
    };
    let (mp4_max_height, mp4_bitrate_mbps) =
        configured_mp4_recording_options(&state.settings);
    set_recording_capture_style(state.overlay, true);
    if let Err(error) = state
        .recording
        .start_capture(
            selection,
            fps,
            state.recording_show_cursor,
            state.recording_setup_format,
            mp4_max_height,
            mp4_bitrate_mbps,
        )
    {
        state.recording.fail(error.clone());
        set_error(state, error);
        cancel(state);
        return;
    }
    state.recording_overlay = true;
    if let Some(capture) = state.capture.as_mut() {
        capture.close_menu();
        capture.toolbar_hover = None;
        capture.secondary_hover = None;
        capture.menu_hover = None;
    }
    create_scroll_masks(state, screen_bounds, selection);
    create_scroll_border(state, selection);
    invalidate(state);
    if let Err(error) = recording_ui::create_control_toolbar(state, selection, screen_bounds) {
        state.recording.cancel();
        restore_recording_selection(state);
        set_error(state, error);
    } else {
        // The selection overlay is a full-screen, opaque screenshot. Keep it
        // out of the way during capture so the live desktop stays visible and
        // receives mouse, wheel, and keyboard input.
        unsafe { ShowWindow(state.overlay, SW_HIDE) };
    }
}

pub(crate) fn start_recording_from_toolbar(state: &mut UiState, fps: u32) {
    let Some(selection) = state
        .capture
        .as_ref()
        .and_then(|capture| capture.selection)
    else {
        return;
    };
    let Some(screen_bounds) = state.screen.as_ref().map(ScreenCapture::bounds) else {
        set_error(state, "无法读取录制屏幕范围".into());
        return;
    };
    let (mp4_max_height, mp4_bitrate_mbps) =
        configured_mp4_recording_options(&state.settings);
    if let Err(error) = state.recording.request_selection(fps) {
        set_error(state, error);
        return;
    }
    set_recording_capture_style(state.overlay, true);
    state.recording_overlay = true;
    if let Some(capture) = state.capture.as_mut() {
        capture.close_menu();
        capture.toolbar_hover = None;
        capture.secondary_hover = None;
        capture.menu_hover = None;
    }
    create_scroll_masks(state, screen_bounds, selection);
    create_scroll_border(state, selection);
    invalidate(state);

    if let Err(error) = state
        .recording
        .start_capture(
            selection,
            fps,
            state.recording_show_cursor,
            state.recording_setup_format,
            mp4_max_height,
            mp4_bitrate_mbps,
        )
    {
        state.recording.fail(error.clone());
        restore_recording_selection(state);
        set_error(state, error);
        return;
    }
    if let Err(error) = recording_ui::create_control_toolbar(state, selection, screen_bounds) {
        state.recording.cancel();
        restore_recording_selection(state);
        set_error(state, error);
    } else {
        // Let the target application remain interactive while frames are
        // captured from the selected screen region.
        unsafe { ShowWindow(state.overlay, SW_HIDE) };
    }
}

pub(crate) fn open_recording_editor(state: &mut UiState) {
    state.recording_overlay = false;
    recording_ui::destroy_control_toolbar(state);
    editor::open_recording_editor(state);
}

pub(crate) fn export_recording_from_editor(
    state: &mut UiState,
    format: AnimationFormat,
) {
    recording_ui::export_recording(state, format);
}

fn stop_recording(state: &mut UiState) {
    if let Err(error) = state.recording.request_stop() {
        set_error(state, error);
    }
}

pub(crate) fn cancel(state: &mut UiState) {
    ocr_ui::close_session_transients(state);
    state.recording_overlay = false;
    recording_ui::destroy_control_toolbar(state);
    if state.recording_requested_fps.take().is_some() {
        state.recording.cancel();
    }
    if state.editor.take().is_some() {
        stop_scroll_input(state);
        if !state.overlay.is_null() {
            unsafe {
                KillTimer(state.overlay, TIMER_SCROLL);
                KillTimer(state.overlay, editor::TIMER_RECORDING_PREVIEW);
                ShowWindow(state.overlay, SW_HIDE);
                DestroyWindow(state.overlay);
            }
            state.overlay = null_mut();
        }
        state.render = None;
        state.last_hover_query = None;
        state.capture = None;
        state.screen = None;
        update_status(state, false);
        return;
    }
    stop_scroll_input(state);
    if !state.overlay.is_null() {
        unsafe {
            KillTimer(state.overlay, TIMER_SCROLL);
            KillTimer(state.overlay, editor::TIMER_RECORDING_PREVIEW);
            ShowWindow(state.overlay, SW_HIDE);
            DestroyWindow(state.overlay);
        }
        state.overlay = null_mut();
    }
    state.render = None;
    state.last_hover_query = None;
    if !state.scroll_control.is_null() {
        unsafe {
            DestroyWindow(state.scroll_control);
        }
        state.scroll_control = null_mut();
    }
    destroy_scroll_preview(state);
    destroy_scroll_masks(state);
    destroy_scroll_border(state);
    state.capture = None;
    state.screen = None;
    update_status(state, false);
}

pub(super) fn request_cancel(state: &UiState) {
    let _ = state.command_tx.send(Command::Cancel);
    if !state.hwnd.is_null() {
        unsafe {
            PostMessageW(state.hwnd, WAKE, 0, 0);
        }
    }
}

pub(crate) fn update_status(state: &UiState, active: bool) {
    if let Ok(mut status) = state.status.lock() {
        status.active = active;
        status.last_path = state.last_path.clone();
        status.last_size = state.last_size;
    }
}

pub(crate) fn set_error(state: &mut UiState, error: String) {
    self::scroll_ui::set_scroll_status(state, error);
}

pub(crate) fn clear_error(state: &mut UiState) {
    if let Ok(mut status) = state.status.lock() {
        status.error = None;
    }
    if !state.scroll_preview.is_null() {
        self::scroll_ui::show_scroll_tip(state, "等待下一次滚动变化".into());
    }
}

pub(crate) fn invalidate(state: &mut UiState) {
    if !state.overlay.is_null() && !state.paint_pending {
        state.paint_pending = true;
        unsafe {
            InvalidateRect(state.overlay, null(), 0);
        }
    }
    if !state.scroll_control.is_null() {
        unsafe {
            InvalidateRect(state.scroll_control, null(), 0);
        }
    }
}

pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = if message == WM_NCCREATE {
        let create =
            &*(lparam as *const windows_sys::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        let state = create.lpCreateParams as *mut UiState;
        SetWindowLongPtrW(hwnd, -21, state as isize);
        state
    } else {
        GetWindowLongPtrW(hwnd, -21) as *mut UiState
    };
    if state.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let state = &mut *state;

    if message == WM_HOTKEY && hwnd == state.hwnd {
        if wparam == SCROLL_ESCAPE_HOTKEY_ID as usize {
            if state.scroll_escape_hotkey_registered {
                cancel(state);
            }
            return 0;
        }
        match state.recording.snapshot().phase.as_str() {
            "recording" => stop_recording(state),
            "selecting" | "processing" | "exporting" | "cancelling" => {}
            _ => begin_capture(state, false),
        }
        return 0;
    }
    if hwnd == state.scroll_tooltip {
        return scroll_tooltip_wnd_proc(hwnd, message, wparam, lparam, state);
    }
    if hwnd == state.scroll_control {
        return scroll_control_wnd_proc(hwnd, message, wparam, lparam, state);
    }
    if hwnd == state.scroll_border {
        return scroll_border_wnd_proc(hwnd, message, wparam, lparam, state);
    }
    if hwnd == state.scroll_preview {
        return scroll_preview_wnd_proc(hwnd, message, wparam, lparam, state);
    }
    if state.scroll_masks.contains(&hwnd) {
        return scroll_mask_wnd_proc(hwnd, message, wparam, lparam, state);
    }
    if hwnd == state.scroll_tip {
        return scroll_tip_wnd_proc(hwnd, message, wparam, lparam, state);
    }
    if hwnd == state.ocr_result_window {
        return ocr_ui::result_window_wnd_proc(hwnd, message, wparam, lparam, state);
    }
    if hwnd == state.pin_window {
        return pin_wnd_proc(hwnd, message, wparam, lparam, state);
    }
    if hwnd == state.recording_toolbar {
        return recording_ui::control_wnd_proc(hwnd, message, wparam, lparam, state);
    }
    if hwnd != state.overlay {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    if state.recording_overlay {
        match message {
            // The full-screen capture overlay sits over the recording controls.
            // Pass hit-testing through so the floating toolbar can receive clicks.
            WM_NCHITTEST => return -1,
            WM_KEYDOWN if wparam as u32 == 0x1b => {
                state.recording.cancel();
                cancel(state);
                return 0;
            }
            WM_LBUTTONUP => {
                let capture = GetCapture();
                if capture == hwnd {
                    ReleaseCapture();
                }
                return 0;
            }
            WM_LBUTTONDBLCLK | WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_RBUTTONUP | WM_CHAR => {
                return 0
            }
            _ => {}
        }
    }

    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if state.editor.is_some() {
                paint_editor(hwnd, state);
            } else {
                paint(hwnd, state);
            }
            0
        }
        WM_CLOSE => {
            cancel(state);
            0
        }
        WM_LBUTTONDBLCLK => {
            if state
                .editor
                .as_ref()
                .is_some_and(|editor| editor.recording_preview)
            {
                return 0;
            }
            let point = screen_point_from_lparam(hwnd, lparam, state.editor.is_some());
            SetCapture(hwnd);
            mouse_down(state, point);
            0
        }
        WM_RBUTTONUP => {
            if state
                .editor
                .as_ref()
                .is_some_and(|editor| editor.recording_preview)
            {
                return 0;
            }
            if !state
                .capture
                .as_ref()
                .is_some_and(|capture| capture.selection.is_some() || capture.drag.is_some())
            {
                cancel(state);
            }
            0
        }
        WM_LBUTTONDOWN => {
            if editor::is_recording_preview(state) {
                let point = screen_point_from_lparam(hwnd, lparam, true);
                if editor::begin_recording_preview_seek(state, point) {
                    SetCapture(hwnd);
                }
                return 0;
            }
            let point = screen_point_from_lparam(hwnd, lparam, state.editor.is_some());
            SetCapture(hwnd);
            mouse_down(state, point);
            0
        }
        WM_MOUSEMOVE => {
            if editor::is_recording_preview(state) {
                let point = screen_point_from_lparam(hwnd, lparam, true);
                editor::move_recording_preview_seek(
                    state,
                    point,
                    wparam & MK_LBUTTON as usize != 0,
                );
                return 0;
            }
            let point = screen_point_from_lparam(hwnd, lparam, state.editor.is_some());
            mouse_move(
                state,
                point,
                wparam & MK_LBUTTON as usize != 0,
            );
            0
        }
        WM_LBUTTONUP => {
            let point = screen_point_from_lparam(hwnd, lparam, state.editor.is_some());
            if editor::is_recording_preview(state) {
                if editor::end_recording_preview_seek(state, point) {
                    ReleaseCapture();
                } else {
                    editor::handle_recording_preview_click(state, point);
                }
                return 0;
            }
            mouse_up(state, point);
            ReleaseCapture();
            0
        }
        WM_CHAR => {
            char_input(state, wparam as u16);
            0
        }
        WM_KEYDOWN => {
            key_down(state, wparam as u32);
            0
        }
        WM_TIMER => {
            if wparam == TIMER_SCROLL {
                scroll_tick(state);
            } else if wparam == editor::TIMER_RECORDING_PREVIEW {
                editor::advance_recording_preview(state);
            }
            0
        }
        WM_SETCURSOR => {
            if editor::is_recording_preview(state) {
                let cursor = LoadCursorW(null_mut(), IDC_ARROW);
                if !cursor.is_null() {
                    SetCursor(cursor);
                }
                return 1;
            }
            let ht = (lparam & 0xffff) as u16;
            if ht == 1 {
                let point = state
                    .capture
                    .as_ref()
                    .and_then(|c| c.current_pos)
                    .unwrap_or_else(|| {
                        let mut pt: POINT = zeroed();
                        GetCursorPos(&mut pt);
                        ScreenPoint { x: pt.x, y: pt.y }
                    });
                update_session_cursor(state, point);
                1
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_DESTROY => DefWindowProcW(hwnd, message, wparam, lparam),
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[cfg(test)]
mod layout_tests {
    use super::paint::rgba_preview_bgra;
    use super::scroll_ui::{scroll_preview_position, selection_fits_capture_bounds};
    use super::toolbar_paint::{toolbar_layout, toolbar_position};
    use super::*;

    #[test]
    fn hotkey_parse_error_is_readable() {
        let error = match parse_hotkey("ctrl+a+bogus") {
            Ok(_) => panic!("invalid hotkey unexpectedly parsed"),
            Err(error) => error,
        };
        assert_eq!(error, "无法识别快捷键：ctrl+a+bogus");
    }

    #[test]
    fn scroll_seed_rejects_selection_outside_capture_bounds() {
        let screen = ScreenRect {
            left: -1920,
            top: -200,
            right: 1920,
            bottom: 1080,
        };
        let inside = ScreenRect {
            left: -400,
            top: 100,
            right: 500,
            bottom: 700,
        };
        let partial = ScreenRect {
            left: -2000,
            top: 100,
            right: 500,
            bottom: 700,
        };
        let outside = ScreenRect {
            left: 2000,
            top: 100,
            right: 2500,
            bottom: 700,
        };
        assert!(selection_fits_capture_bounds(screen, inside));
        assert!(!selection_fits_capture_bounds(screen, partial));
        assert!(!selection_fits_capture_bounds(screen, outside));
    }

    #[test]
    fn toolbar_position_stays_inside_negative_virtual_desktop() {
        let screen = ScreenRect {
            left: -1920,
            top: -200,
            right: 1920,
            bottom: 1080,
        };
        let selection = ScreenRect {
            left: -400,
            top: 100,
            right: 500,
            bottom: 700,
        };
        let (x, y) = toolbar_position(selection, screen);
        let layout = toolbar_layout(screen);
        assert!(x >= screen.left + 8);
        assert!(x + layout.width <= screen.right - 8);
        assert!(y >= screen.top + 8);
        assert!(y + layout.height <= screen.bottom - 8);
    }

    #[test]
    fn layout_matrix_stays_in_bounds_for_scaled_virtual_desktops() {
        let cases = [
            (
                ScreenRect {
                    left: 0,
                    top: 0,
                    right: 1707,
                    bottom: 960,
                },
                ScreenRect {
                    left: 100,
                    top: 100,
                    right: 800,
                    bottom: 700,
                },
            ),
            (
                ScreenRect {
                    left: 0,
                    top: 0,
                    right: 2560,
                    bottom: 1440,
                },
                ScreenRect {
                    left: 200,
                    top: 200,
                    right: 1300,
                    bottom: 1000,
                },
            ),
            (
                ScreenRect {
                    left: -2560,
                    top: -1440,
                    right: 0,
                    bottom: 0,
                },
                ScreenRect {
                    left: -2300,
                    top: -1300,
                    right: -1300,
                    bottom: -500,
                },
            ),
            (
                ScreenRect {
                    left: -1920,
                    top: -200,
                    right: 1920,
                    bottom: 1080,
                },
                ScreenRect {
                    left: -1600,
                    top: -100,
                    right: -600,
                    bottom: 700,
                },
            ),
        ];

        for (screen, selection) in cases {
            let (toolbar_x, toolbar_y) = toolbar_position(selection, screen);
            let toolbar = toolbar_layout(screen);
            assert!(toolbar_x >= screen.left + 8);
            assert!(toolbar_y >= screen.top + 8);
            assert!(toolbar_x + toolbar.width <= screen.right - 8);
            assert!(toolbar_y + toolbar.height <= screen.bottom - 8);

            let (preview_x, preview_y) = scroll_preview_position(screen, selection);
            let preview_right = preview_x + SCROLL_PREVIEW_WIDTH;
            let preview_bottom = preview_y + SCROLL_PREVIEW_HEIGHT;
            assert!(preview_x >= screen.left + SCROLL_PREVIEW_MARGIN);
            assert!(preview_y >= screen.top + SCROLL_PREVIEW_MARGIN);
            assert!(preview_right <= screen.right - SCROLL_PREVIEW_MARGIN);
            assert!(preview_bottom <= screen.bottom - SCROLL_PREVIEW_MARGIN);
            assert!(
                preview_right <= selection.left
                    || preview_x >= selection.right
                    || preview_bottom <= selection.top
                    || preview_y >= selection.bottom
            );
        }
    }

    #[test]
    fn layout_matrix_covers_requested_dpi_scales() {
        for scale in [100i32, 125, 150, 175, 200] {
            let screen_width = 1920 * scale / 100;
            let screen_height = 1080 * scale / 100;
            let screen = ScreenRect {
                left: 0,
                top: 0,
                right: screen_width,
                bottom: screen_height,
            };
            let selection = ScreenRect {
                left: 400 * scale / 100,
                top: 300 * scale / 100,
                right: 1200 * scale / 100,
                bottom: 800 * scale / 100,
            };
            let (toolbar_x, toolbar_y) = toolbar_position(selection, screen);
            let toolbar = toolbar_layout(screen);
            assert!(toolbar_x >= screen.left + 8);
            assert!(toolbar_y >= screen.top + 8);
            assert!(toolbar_x + toolbar.width <= screen.right - 8);
            assert!(toolbar_y + toolbar.height <= screen.bottom - 8);

            let (preview_x, preview_y) = scroll_preview_position(screen, selection);
            let preview_right = preview_x + SCROLL_PREVIEW_WIDTH;
            let preview_bottom = preview_y + SCROLL_PREVIEW_HEIGHT;
            assert!(preview_x >= screen.left + SCROLL_PREVIEW_MARGIN);
            assert!(preview_y >= screen.top + SCROLL_PREVIEW_MARGIN);
            assert!(preview_right <= screen.right - SCROLL_PREVIEW_MARGIN);
            assert!(preview_bottom <= screen.bottom - SCROLL_PREVIEW_MARGIN);
            assert!(
                preview_right <= selection.left
                    || preview_x >= selection.right
                    || preview_bottom <= selection.top
                    || preview_y >= selection.bottom
            );
        }
    }

    #[test]
    fn rgba_scroll_preview_preserves_channel_order() {
        let rgba = vec![
            255, 0, 0, 255, // red
            0, 0, 255, 255, // blue
            0, 255, 0, 255, // green
            255, 255, 255, 255,
        ];
        let bgra = rgba_preview_bgra(&rgba, 2, 2, 2, 2).expect("valid preview");
        assert_eq!(
            bgra,
            vec![
                0, 0, 255, 255, // red in BGRA
                255, 0, 0, 255, // blue in BGRA
                0, 255, 0, 255, // green
                255, 255, 255, 255,
            ]
        );
    }

    #[test]
    fn rgba_scroll_preview_downsamples_to_target_buffer() {
        let rgba = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36,
        ];
        let bgra = rgba_preview_bgra(&rgba, 3, 3, 2, 2).unwrap();
        assert_eq!(bgra.len(), 2 * 2 * 4);
        assert_eq!(
            bgra,
            vec![3, 2, 1, 4, 7, 6, 5, 8, 15, 14, 13, 16, 19, 18, 17, 20,]
        );
        assert!(rgba_preview_bgra(&rgba, 0, 3, 2, 2).is_none());
        assert!(rgba_preview_bgra(&rgba[..4], 3, 3, 2, 2).is_none());
    }

    #[test]
    fn scroll_preview_avoids_selection_on_negative_desktop() {
        let screen = ScreenRect {
            left: -1920,
            top: -200,
            right: 1920,
            bottom: 1080,
        };
        let selection = ScreenRect {
            left: -600,
            top: -100,
            right: 600,
            bottom: 700,
        };
        let (x, y) = scroll_preview_position(screen, selection);
        let right = x + SCROLL_PREVIEW_WIDTH;
        let bottom = y + SCROLL_PREVIEW_HEIGHT;
        assert!(
            right <= selection.left
                || x >= selection.right
                || bottom <= selection.top
                || y >= selection.bottom
        );
        assert!(x >= screen.left + SCROLL_PREVIEW_MARGIN);
        assert!(y >= screen.top + SCROLL_PREVIEW_MARGIN);
        assert!(right <= screen.right - SCROLL_PREVIEW_MARGIN);
        assert!(bottom <= screen.bottom - SCROLL_PREVIEW_MARGIN);
    }
}
