use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK,
    KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_LOWER_IL_INJECTED, LLKHF_UP, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::input::{VK_A, VK_ESCAPE_CODE};
use super::{clear_error, set_error, UiState, DOUBLE_TAP_F1};

pub(super) const HOTKEY_ID: i32 = 41;
pub(super) const SCROLL_ESCAPE_HOTKEY_ID: i32 = 43;

const DOUBLE_TAP_WINDOW_MS: u64 = 460;
const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_SHIFT: u32 = 0x0004;
const MOD_WIN: u32 = 0x0008;
const MOD_NOREPEAT: u32 = 0x4000;

static DOUBLE_TAP_TARGET: AtomicIsize = AtomicIsize::new(0);
static F1_IS_DOWN: AtomicBool = AtomicBool::new(false);
static LAST_F1_DOWN_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub(super) struct HotkeySpec {
    pub(super) modifiers: u32,
    pub(super) key: u32,
}

impl Default for HotkeySpec {
    fn default() -> Self {
        Self {
            modifiers: MOD_CONTROL | MOD_SHIFT,
            key: VK_A,
        }
    }
}

pub(super) fn parse_hotkey(value: &str) -> Result<HotkeySpec, String> {
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

pub(super) fn install_double_tap_f1_hook(hwnd: HWND) -> HHOOK {
    DOUBLE_TAP_TARGET.store(hwnd as isize, Ordering::SeqCst);
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(double_tap_f1_hook_proc),
            GetModuleHandleW(null()),
            0,
        )
    };
    if hook.is_null() {
        DOUBLE_TAP_TARGET.store(0, Ordering::SeqCst);
    }
    hook
}

unsafe extern "system" fn double_tap_f1_hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 && lparam != 0 {
        let key = &*(lparam as *const KBDLLHOOKSTRUCT);
        if key.vkCode == 0x70
            && key.flags & (LLKHF_INJECTED | LLKHF_LOWER_IL_INJECTED) == 0
        {
            if key.flags & LLKHF_UP != 0
                || wparam == WM_KEYUP as usize
                || wparam == WM_SYSKEYUP as usize
            {
                F1_IS_DOWN.store(false, Ordering::SeqCst);
            } else if (wparam == WM_KEYDOWN as usize || wparam == WM_SYSKEYDOWN as usize)
                && !F1_IS_DOWN.swap(true, Ordering::SeqCst)
            {
                let now = GetTickCount64();
                let previous = LAST_F1_DOWN_MS.swap(now, Ordering::SeqCst);
                if previous != 0 && now.saturating_sub(previous) <= DOUBLE_TAP_WINDOW_MS {
                    LAST_F1_DOWN_MS.store(0, Ordering::SeqCst);
                    let target = DOUBLE_TAP_TARGET.load(Ordering::SeqCst);
                    if target != 0 {
                        PostMessageW(target as HWND, DOUBLE_TAP_F1, 0, 0);
                    }
                }
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
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

pub(super) fn update_hotkey(state: &mut UiState, value: &str) {
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

pub(super) fn register_hotkey(hwnd: HWND, hotkey: HotkeySpec) -> bool {
    unsafe {
        RegisterHotKey(
            hwnd,
            HOTKEY_ID,
            hotkey.modifiers | MOD_NOREPEAT,
            hotkey.key,
        ) != 0
    }
}

pub(super) fn register_scroll_escape_hotkey(state: &mut UiState) -> bool {
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

pub(super) fn unregister_scroll_escape_hotkey(state: &mut UiState) {
    if state.scroll_escape_hotkey_registered {
        unsafe {
            UnregisterHotKey(state.hwnd, SCROLL_ESCAPE_HOTKEY_ID);
        }
        state.scroll_escape_hotkey_registered = false;
    }
}

pub(super) fn shutdown(state: &mut UiState) {
    DOUBLE_TAP_TARGET.store(0, Ordering::SeqCst);
    if !state.double_tap_hook.is_null() {
        unsafe {
            UnhookWindowsHookEx(state.double_tap_hook);
        }
        state.double_tap_hook = null_mut();
    }
    F1_IS_DOWN.store(false, Ordering::SeqCst);
    LAST_F1_DOWN_MS.store(0, Ordering::SeqCst);
    if state.hotkey_registered {
        unsafe {
            UnregisterHotKey(state.hwnd, HOTKEY_ID);
        }
        state.hotkey_registered = false;
    }
    unregister_scroll_escape_hotkey(state);
}
