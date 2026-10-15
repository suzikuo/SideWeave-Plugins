use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(windows)]
use std::sync::Condvar;
#[cfg(windows)]
use std::thread::{self, JoinHandle};
#[cfg(windows)]
use std::time::Duration;

use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind,
    PluginResponse, PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION,
    MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{
    success_response, Plugin, PluginContext, ShortcutAction, ShortcutHandler, WindowSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetShellWindow};

const PLUGIN_ID: &str = "thiefbook";

sideweave_plugin_sdk::export_plugin_dylib!(ThiefbookPlugin);
const STATE_KEY: &str = "state";
const READER_WINDOW: &str = "thiefbook-reader";
const STYLE_EVENT: &str = "thief://style";
const OPEN_EVENT: &str = "thief://open";
const JUMP_EVENT: &str = "thief://jump";
const PROGRESS_EVENT: &str = "thief://progress";
const PAGE_EVENT: &str = "thief://page";
const CHAR_INDEX_STRIDE: u64 = 1024;
const MAX_READ_CHARS: u64 = 32_768;
const LINE_HEIGHT: f64 = 1.9;
const READER_VERTICAL_PADDING: f64 = 12.0;
static HOTKEYS_ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
struct DesktopMonitorState {
    stop: bool,
    enabled: bool,
}

#[cfg(windows)]
struct DesktopMonitorControl {
    state: Mutex<DesktopMonitorState>,
    changed: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

#[cfg(windows)]
fn desktop_monitor_control() -> &'static DesktopMonitorControl {
    static CONTROL: OnceLock<DesktopMonitorControl> = OnceLock::new();
    CONTROL.get_or_init(|| DesktopMonitorControl {
        state: Mutex::new(DesktopMonitorState { stop: false, enabled: false }),
        changed: Condvar::new(),
        thread: Mutex::new(None),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ThiefBook {
    pub id: String,
    pub name: String,
    pub path: String,
    pub offset: u64,
    pub total_chars: u64,
    pub page_history: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThiefStyle {
    pub font_size: u32,
    pub opacity: f64,
    pub width: u32,
    pub lines: u32,
    pub color: String,
    pub pinned: bool,
    pub hotkey_toggle: String,
    pub hotkey_prev: String,
    pub hotkey_next: String,
    pub auto_page_secs: u32,
    pub hide_on_desktop: bool,
}

impl Default for ThiefStyle {
    fn default() -> Self {
        Self { font_size: 14, opacity: 0.35, width: 420, lines: 3, color: "#e8e8ea".into(), pinned: false, hotkey_toggle: String::new(), hotkey_prev: String::new(), hotkey_next: String::new(), auto_page_secs: 0, hide_on_desktop: true }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ReaderPosition {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct Config {
    books: Vec<ThiefBook>,
    style: ThiefStyle,
    current_book_id: Option<String>,
    reader_position: Option<ReaderPosition>,
    reader_visible: bool,
}

fn load_config(context: &PluginContext) -> Config {
    context.storage().get(context.plugin_id(), STATE_KEY).ok().flatten()
        .and_then(|value| serde_json::from_str(&value).ok()).unwrap_or_default()
}

fn save_config(context: &PluginContext, config: &Config) -> Result<(), String> {
    let value = serde_json::to_string(config).map_err(|error| error.to_string())?;
    context.storage().set(context.plugin_id(), STATE_KEY, &value).map_err(|error| error.message)
}

fn config_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct CachedText {
    text: Arc<str>,
    char_offsets: Vec<usize>,
    total_chars: u64,
}

fn text_cache() -> &'static Mutex<HashMap<String, Arc<CachedText>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<CachedText>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_text(mut text: String) -> CachedText {
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    let mut char_offsets = Vec::new();
    let mut total_chars = 0_u64;
    for (byte, _) in text.char_indices() {
        if total_chars % CHAR_INDEX_STRIDE == 0 {
            char_offsets.push(byte);
        }
        total_chars += 1;
    }
    CachedText { text: Arc::from(text), char_offsets, total_chars }
}

fn load_text(context: &PluginContext, path: &str) -> Result<Arc<CachedText>, String> {
    if let Some(text) = text_cache().lock().unwrap().get(path).cloned() { return Ok(text); }
    let bytes = context.files().read_external(path).map_err(|error| format!("读取文件失败: {}", error.message))?;
    let text = Arc::new(cache_text(String::from_utf8_lossy(&bytes).into_owned()));
    text_cache().lock().unwrap().insert(path.into(), Arc::clone(&text));
    Ok(text)
}

fn char_byte_offset(text: &CachedText, index: u64) -> usize {
    if index >= text.total_chars {
        return text.text.len();
    }
    let checkpoint = (index / CHAR_INDEX_STRIDE) as usize;
    let base_char = checkpoint as u64 * CHAR_INDEX_STRIDE;
    let base_byte = text.char_offsets.get(checkpoint).copied().unwrap_or(text.text.len());
    text.text[base_byte..]
        .char_indices()
        .nth((index - base_char) as usize)
        .map(|(relative, _)| base_byte + relative)
        .unwrap_or(text.text.len())
}

fn char_index_at_byte(text: &CachedText, byte_offset: usize) -> u64 {
    let byte_offset = byte_offset.min(text.text.len());
    let checkpoint = text.char_offsets.partition_point(|byte| *byte <= byte_offset).saturating_sub(1);
    let base_byte = text.char_offsets.get(checkpoint).copied().unwrap_or(0);
    let base_char = checkpoint as u64 * CHAR_INDEX_STRIDE;
    base_char + text.text[base_byte..byte_offset].chars().count() as u64
}

fn find_text(text: &CachedText, query: &str, from_offset: u64, direction: i64) -> Option<u64> {
    if query.is_empty() || query.len() > text.text.len() {
        return None;
    }

    let query_len = query.len();
    let last_start = text.text.len().saturating_sub(query_len);
    let start = char_byte_offset(text, from_offset.min(text.total_chars));
    let insensitive_ascii = query.is_ascii();
    let matches_at = |byte: usize| {
        if !text.text.is_char_boundary(byte + query_len) { return false; }
        let candidate = &text.text[byte..byte + query_len];
        if insensitive_ascii { candidate.eq_ignore_ascii_case(query) } else { candidate == query }
    };

    let found = if direction < 0 {
        // Search strictly before the requested character, then wrap from the end.
        (0..start).rev().find(|byte| text.text.is_char_boundary(*byte) && *byte <= last_start && matches_at(*byte))
            .or_else(|| (start..=last_start).rev().find(|byte| text.text.is_char_boundary(*byte) && matches_at(*byte)))
    } else {
        // Search at or after the requested character, then wrap to the beginning.
        (start..=last_start).find(|byte| text.text.is_char_boundary(*byte) && matches_at(*byte))
            .or_else(|| (0..start).find(|byte| text.text.is_char_boundary(*byte) && *byte <= last_start && matches_at(*byte)))
    }?;

    Some(char_index_at_byte(text, found))
}

struct ReaderChunk {
    text: String,
    offset: u64,
    total: u64,
}

fn read_chunk(text: &CachedText, offset: u64, limit: u64) -> ReaderChunk {
    let start = offset.min(text.total_chars);
    let start_byte = char_byte_offset(text, start);
    let content = &text.text[start_byte..];
    let limit = limit.clamp(64, MAX_READ_CHARS) as usize;
    let end_byte = content.char_indices().nth(limit)
        .map(|(byte, _)| byte)
        .unwrap_or(content.len());
    ReaderChunk {
        text: content[..end_byte].to_owned(),
        offset: start,
        total: text.total_chars,
    }
}

fn clamp_color(value: &str) -> String {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if value.len() == 6 && value.chars().all(|char| char.is_ascii_hexdigit()) { format!("#{}", value.to_ascii_lowercase()) } else { "#e8e8ea".into() }
}

fn clamp_style(style: &ThiefStyle) -> ThiefStyle {
    ThiefStyle { font_size: style.font_size.clamp(10, 32), opacity: if style.opacity.is_finite() { style.opacity.clamp(0.0, 1.0) } else { 0.35 }, width: style.width.clamp(240, 1600), lines: style.lines.clamp(1, 20), color: clamp_color(&style.color), pinned: style.pinned, hotkey_toggle: style.hotkey_toggle.trim().into(), hotkey_prev: style.hotkey_prev.trim().into(), hotkey_next: style.hotkey_next.trim().into(), auto_page_secs: style.auto_page_secs.clamp(0, 120), hide_on_desktop: style.hide_on_desktop }
}

fn reader_width(style: &ThiefStyle) -> f64 {
    f64::from(style.width)
}

fn reader_height(style: &ThiefStyle) -> f64 {
    (f64::from(style.lines) * f64::from(style.font_size) * LINE_HEIGHT + READER_VERTICAL_PADDING).ceil()
}

#[cfg(windows)]
fn is_desktop_foreground() -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return false;
    }
    let shell = unsafe { GetShellWindow() };
    if !shell.is_null() && foreground == shell {
        return true;
    }

    let mut class_name = [0_u16; 256];
    let length = unsafe { GetClassNameW(foreground, class_name.as_mut_ptr(), class_name.len() as i32) };
    if length <= 0 {
        return false;
    }
    matches!(String::from_utf16_lossy(&class_name[..length as usize]).as_str(), "Progman" | "WorkerW")
}

#[cfg(windows)]
fn auto_hide_on_desktop(context: &PluginContext) {
    if !is_desktop_foreground() {
        return;
    }
    let position = current_reader_position(context).ok().flatten();
    let should_hide = {
        let _guard = config_lock().lock().unwrap();
        let mut config = load_config(context);
        if !config.style.hide_on_desktop || !config.reader_visible || !is_desktop_foreground() {
            return;
        }
        if let Some(position) = position {
            config.reader_position = Some(position);
        }
        config.reader_visible = false;
        save_config(context, &config).is_ok()
    };

    if should_hide {
        set_desktop_monitor_enabled(false);
        let _ = context.windows().set_clickthrough(READER_WINDOW, false);
        let _ = context.windows().hide(READER_WINDOW);
    }
}

fn start_desktop_monitor(context: &PluginContext) {
    #[cfg(windows)]
    {
        let config = load_config(context);
        if !config.style.hide_on_desktop || !config.reader_visible || current_reader_position(context).ok().flatten().is_none() {
            return;
        }

        let control = desktop_monitor_control();
        let mut monitor_thread = control.thread.lock().unwrap();
        if monitor_thread.is_none() {
            {
                let mut state = control.state.lock().unwrap();
                state.stop = false;
                state.enabled = true;
                control.changed.notify_all();
            }
            let plugin_id = context.plugin_id().clone();
            let services = context.services_arc();
            let result = thread::Builder::new().name("thiefbook-desktop-monitor".into()).spawn(move || {
                let context = PluginContext::new(plugin_id, services);
                desktop_monitor_loop(&context, control);
            });
            match result {
                Ok(handle) => *monitor_thread = Some(handle),
                Err(_) => {
                    let mut state = control.state.lock().unwrap();
                    state.enabled = false;
                    state.stop = true;
                }
            }
        } else {
            set_desktop_monitor_enabled(true);
        }
    }
    #[cfg(not(windows))]
    let _ = context;
}

#[cfg(windows)]
fn desktop_monitor_loop(context: &PluginContext, control: &'static DesktopMonitorControl) {
    loop {
        let mut state = control.state.lock().unwrap();
        while !state.stop && !state.enabled {
            state = control.changed.wait(state).unwrap();
        }
        if state.stop {
            break;
        }
        drop(state);

        auto_hide_on_desktop(context);

        state = control.state.lock().unwrap();
        if !state.stop && state.enabled {
            let _ = control.changed.wait_timeout(state, Duration::from_millis(300)).unwrap();
        }
    }
}

fn set_desktop_monitor_enabled(enabled: bool) {
    #[cfg(windows)]
    {
        let control = desktop_monitor_control();
        let mut state = control.state.lock().unwrap();
        if !state.stop {
            state.enabled = enabled;
        }
        control.changed.notify_all();
    }
    #[cfg(not(windows))]
    let _ = enabled;
}

fn stop_desktop_monitor() {
    #[cfg(windows)]
    {
        let control = desktop_monitor_control();
        let mut monitor_thread = control.thread.lock().unwrap();
        {
            let mut state = control.state.lock().unwrap();
            state.enabled = false;
            state.stop = true;
            control.changed.notify_all();
        }
        if let Some(handle) = monitor_thread.take() {
            let _ = handle.join();
        }
    }
}

fn current_reader_position(context: &PluginContext) -> Result<Option<ReaderPosition>, String> {
    context.windows().position(READER_WINDOW)
        .map(|position| position.map(|(x, y)| ReaderPosition { x, y }))
        .map_err(|error| error.message)
}

fn save_reader_position(context: &PluginContext, x: i32, y: i32) -> Result<(), String> {
    let _guard = config_lock().lock().unwrap();
    let mut config = load_config(context);
    config.reader_position = Some(ReaderPosition { x, y });
    save_config(context, &config)
}

fn remember_reader_hidden(context: &PluginContext, x: i32, y: i32) -> Result<(), String> {
    let _guard = config_lock().lock().unwrap();
    let mut config = load_config(context);
    config.reader_position = Some(ReaderPosition { x, y });
    config.reader_visible = false;
    save_config(context, &config)?;
    set_desktop_monitor_enabled(false);
    Ok(())
}

fn toggle_reader(context: &PluginContext) -> Result<(), String> {
    let position = current_reader_position(context)?;
    let exists = position.is_some();
    let (visible, config) = {
        let _guard = config_lock().lock().unwrap();
        let mut config = load_config(context);
        let visible = if exists { !config.reader_visible } else { true };
        if let Some(position) = position {
            config.reader_position = Some(position);
        }
        config.reader_visible = visible;
        save_config(context, &config)?;
        (visible, config)
    };

    if !exists {
        create_reader(context, &config)?;
    } else if visible {
        context.windows().set_size(READER_WINDOW, reader_width(&config.style), reader_height(&config.style)).map_err(|error| error.message)?;
        context.windows().set_clickthrough(READER_WINDOW, config.style.pinned).map_err(|error| error.message)?;
        context.windows().show(READER_WINDOW).map_err(|error| error.message)?;
    } else {
        context.windows().set_clickthrough(READER_WINDOW, false).map_err(|error| error.message)?;
        context.windows().hide(READER_WINDOW).map_err(|error| error.message)?;
        set_desktop_monitor_enabled(false);
    }
    if visible { start_desktop_monitor(context); }
    Ok(())
}

fn create_reader(context: &PluginContext, config: &Config) -> Result<(), String> {
    context.windows().create(WindowSpec {
        id: READER_WINDOW.into(),
        title: "SideWeave Reader".into(),
        url: "ui/reader.html".into(),
        width: reader_width(&config.style),
        height: reader_height(&config.style),
        position_px: config.reader_position.map(|position| (position.x, position.y)),
        always_on_top: true,
        skip_taskbar: true,
        resizable: false,
        transparent: true,
        decorations: false,
        maximizable: false,
    }, None).map_err(|error| error.message)?;
    context.windows().set_clickthrough(READER_WINDOW, config.style.pinned).map_err(|error| error.message)
}

fn save_style(context: &PluginContext, style: ThiefStyle) -> Result<Value, String> {
    let style = clamp_style(&style);
    let (size_changed, pinned_changed, hotkeys_changed) = {
        let _guard = config_lock().lock().unwrap();
        let mut config = load_config(context);
        let previous = config.style.clone();
        let size_changed = previous.font_size != style.font_size
            || previous.width != style.width
            || previous.lines != style.lines;
        let pinned_changed = previous.pinned != style.pinned;
        let hotkeys_changed = previous.hotkey_toggle != style.hotkey_toggle
            || previous.hotkey_prev != style.hotkey_prev
            || previous.hotkey_next != style.hotkey_next;
        config.style = style.clone();
        save_config(context, &config)?;
        (size_changed, pinned_changed, hotkeys_changed)
    };
    if current_reader_position(context)?.is_some() {
        if size_changed {
            context.windows().set_size(READER_WINDOW, reader_width(&style), reader_height(&style)).map_err(|error| error.message)?;
        }
        if pinned_changed {
            context.windows().set_clickthrough(READER_WINDOW, style.pinned).map_err(|error| error.message)?;
        }
        context.windows().emit(READER_WINDOW, STYLE_EVENT, json!(style)).map_err(|error| error.message)?;
    }
    if style.hide_on_desktop { start_desktop_monitor(context); } else { set_desktop_monitor_enabled(false); }
    if hotkeys_changed { schedule_hotkey_registration(context); }
    Ok(Value::Null)
}

fn open_book(context: &PluginContext, book_id: String) -> Result<Value, String> {
    let position = current_reader_position(context)?;
    let exists = position.is_some();
    let (config, was_visible) = {
        let _guard = config_lock().lock().unwrap();
        let mut config = load_config(context);
        if !config.books.iter().any(|book| book.id == book_id) {
            return Err("书不存在".into());
        }
        if let Some(position) = position {
            config.reader_position = Some(position);
        }
        let was_visible = exists && config.reader_visible;
        config.current_book_id = Some(book_id.clone());
        config.reader_visible = true;
        save_config(context, &config)?;
        (config, was_visible)
    };

    if !exists {
        // The initial book id is in config before the queued window is created,
        // so first-open does not depend on an event arriving after mount.
        create_reader(context, &config)?;
    } else if !was_visible {
        context.windows().set_size(READER_WINDOW, reader_width(&config.style), reader_height(&config.style)).map_err(|error| error.message)?;
        context.windows().set_clickthrough(READER_WINDOW, config.style.pinned).map_err(|error| error.message)?;
        context.windows().show(READER_WINDOW).map_err(|error| error.message)?;
        context.windows().emit(READER_WINDOW, OPEN_EVENT, Value::String(book_id)).map_err(|error| error.message)?;
    } else {
        context.windows().emit(READER_WINDOW, OPEN_EVENT, Value::String(book_id)).map_err(|error| error.message)?;
    }
    start_desktop_monitor(context);
    Ok(Value::Null)
}

fn jump_to(context: &PluginContext, book_id: String, offset: u64) -> Result<Value, String> {
    let position = current_reader_position(context)?;
    let exists = position.is_some();
    let (config, actual_offset) = {
        let _guard = config_lock().lock().unwrap();
        let mut config = load_config(context);
        let book = config.books.iter_mut().find(|book| book.id == book_id).ok_or_else(|| "书不存在".to_string())?;
        let actual_offset = offset.min(book.total_chars);
        book.offset = actual_offset;
        book.page_history.clear();
        config.current_book_id = Some(book_id.clone());
        if let Some(position) = position {
            config.reader_position = Some(position);
        }
        config.reader_visible = true;
        save_config(context, &config)?;
        (config, actual_offset)
    };
    emit_progress(context, &book_id, actual_offset);

    if !exists {
        create_reader(context, &config)?;
    } else {
        context.windows().set_size(READER_WINDOW, reader_width(&config.style), reader_height(&config.style)).map_err(|error| error.message)?;
        context.windows().set_clickthrough(READER_WINDOW, config.style.pinned).map_err(|error| error.message)?;
        context.windows().show(READER_WINDOW).map_err(|error| error.message)?;
        context.windows().emit(READER_WINDOW, JUMP_EVENT, json!({ "book_id": book_id, "offset": actual_offset })).map_err(|error| error.message)?;
    }
    start_desktop_monitor(context);
    Ok(Value::Null)
}

fn register_hotkeys(context: &PluginContext) -> Result<(), String> {
    let config = load_config(context);
    let bindings = [
        (config.style.hotkey_toggle, ShortcutAction::Toggle),
        (config.style.hotkey_prev, ShortcutAction::Previous),
        (config.style.hotkey_next, ShortcutAction::Next),
    ].into_iter().filter(|(key, _)| !key.trim().is_empty()).collect::<Vec<_>>();
    let services = context.services_arc();
    let plugin_id = context.plugin_id().clone();
    let handler: ShortcutHandler = Arc::new(move |action| {
        let context = PluginContext::new(plugin_id.clone(), Arc::clone(&services));
        match action {
            ShortcutAction::Toggle => { let _ = toggle_reader(&context); }
            ShortcutAction::Previous => { let _ = context.windows().emit(READER_WINDOW, PAGE_EVENT, json!(-1)); }
            ShortcutAction::Next => { let _ = context.windows().emit(READER_WINDOW, PAGE_EVENT, json!(1)); }
        }
    });
    context.shortcuts().register(context.plugin_id(), &bindings, handler)
        .map_err(|error| error.message)
}

fn schedule_hotkey_registration(context: &PluginContext) {
    let plugin_id = context.plugin_id().clone();
    let services = context.services_arc();
    let result = std::thread::Builder::new()
        .name("thiefbook-hotkey-registration".into())
        .spawn(move || {
            let context = PluginContext::new(plugin_id, services);
            match register_hotkeys(&context) {
                Ok(()) => {
                    if !HOTKEYS_ACTIVE.load(Ordering::SeqCst) {
                        let _ = context.shortcuts().unregister(context.plugin_id());
                    }
                }
                Err(error) => eprintln!("[thiefbook] hotkey registration failed: {error}"),
            }
        });
    if let Err(error) = result {
        eprintln!("[thiefbook] failed to schedule hotkey registration: {error}");
    }
}

fn save_progress(context: &PluginContext, book_id: &str, offset: u64, history: Vec<u64>) -> Result<(), String> {
    let _guard = config_lock().lock().unwrap();
    let mut config = load_config(context);
    let book = config.books.iter_mut().find(|book| book.id == book_id).ok_or_else(|| "书不存在".to_string())?;
    book.offset = offset;
    book.page_history = history.into_iter().rev().take(1000).collect::<Vec<_>>().into_iter().rev().collect();
    save_config(context, &config)
}

fn emit_progress(context: &PluginContext, book_id: &str, offset: u64) {
    let _ = context.windows().emit("detail", PROGRESS_EVENT, json!({ "book_id": book_id, "offset": offset }));
}

pub struct ThiefbookPlugin { manifest: PluginManifest }

impl Default for ThiefbookPlugin {
    fn default() -> Self { Self::new() }
}

impl ThiefbookPlugin {
    pub fn new() -> Self {
        Self { manifest: PluginManifest { manifest_version: MANIFEST_VERSION, api_version: API_VERSION, id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"), name: "书架".into(), icon: "ui/icon.svg".into(), version: env!("CARGO_PKG_VERSION").into(), description: "TXT reading shelf with floating reader and progress tracking".into(), author: "SideWeave".into(), runtime: PluginRuntime::DynamicLibrary, backend: Some("plugin.dll".into()), trust: PluginTrust::TrustedLocal, auto_start: true, permissions: vec!["storage".into(), "filesystem.read".into(), "filesystem.external_read".into(), "window.create".into(), "window.manage".into(), "shortcut.global".into(), "system.file_picker".into()], ui: Some(PluginUi { entry: "ui/index.html".into(), kind: Some(PluginUiKind::Html), asset_root: "ui".into(), initial_width: Some(820), initial_height: Some(820), ..Default::default() }), hover_preview: Some(sideweave_plugin_api::PluginHoverPreview { width: Some(280), height: Some(180), data_method: Some("get_preview".into()), entry: None }), minimum_host_version: None } }
    }

    fn call(&self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        match request.method.as_deref() {
            Some("get_config") => Ok(json!(load_config(context))),
            Some("get_preview") => {
                let config = load_config(context);
                let visible = config.reader_visible && current_reader_position(context)?.is_some();
                let current_book = config.current_book_id.as_deref()
                    .and_then(|id| config.books.iter().find(|book| book.id == id))
                    .or_else(|| config.books.first());
                let subtitle = current_book
                    .map(|book| format!("当前：{}", book.name))
                    .unwrap_or_else(|| "书架暂无书籍".into());
                Ok(json!({
                    "title": "书架悬浮窗",
                    "status": if visible { "online" } else { "offline" },
                    "statusText": if visible { "正在显示" } else { "已隐藏" },
                    "subtitle": subtitle,
                    "actions": [{
                        "id": "toggle_reader",
                        "label": if visible { "隐藏悬浮窗" } else { "打开悬浮窗" },
                        "method": "toggle_reader",
                        "primary": !visible
                    }]
                }))
            }
            Some("pick_book") => {
                let Some(path) = context.system().pick_file(&["txt".into()]).map_err(|error| error.message)? else { return Ok(Value::Null); };
                if !path.to_ascii_lowercase().ends_with(".txt") { return Err("请选择 txt 文本文件".into()); }
                let text = load_text(context, &path)?;
                let _guard = config_lock().lock().unwrap();
                let mut config = load_config(context);
                if let Some(book) = config.books.iter().find(|book| book.path == path).cloned() { return Ok(json!(book)); }
                let book = ThiefBook { id: uuid::Uuid::new_v4().to_string(), name: std::path::Path::new(&path).file_stem().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "未命名".into()), path, offset: 0, total_chars: text.total_chars, page_history: Vec::new() };
                config.books.push(book.clone());
                config.current_book_id = Some(book.id.clone());
                save_config(context, &config)?;
                Ok(json!(book))
            }
            Some("remove_book") => {
                let id = string_field(&request.payload, "id")?;
                let _guard = config_lock().lock().unwrap();
                let mut config = load_config(context);
                let path = config.books.iter().find(|book| book.id == id).map(|book| book.path.clone());
                let before = config.books.len();
                config.books.retain(|book| book.id != id);
                if before == config.books.len() { return Err("书不存在".into()); }
                if config.current_book_id.as_deref() == Some(id.as_str()) {
                    config.current_book_id = config.books.first().map(|book| book.id.clone());
                }
                save_config(context, &config)?;
                if let Some(path) = path { text_cache().lock().unwrap().remove(&path); }
                Ok(Value::Null)
            }
            Some("read") => {
                let id = string_field(&request.payload, "book_id")?;
                let offset = request.payload.get("offset").and_then(Value::as_u64).unwrap_or(0);
                let limit = request.payload.get("limit").and_then(Value::as_u64).unwrap_or(2048).clamp(64, MAX_READ_CHARS);
                let book = load_config(context).books.into_iter().find(|book| book.id == id).ok_or_else(|| "书不存在".to_string())?;
                let text = load_text(context, &book.path)?;
                let chunk = read_chunk(&text, offset, limit);
                Ok(json!({
                    "text": chunk.text,
                    "total": chunk.total,
                    "offset": chunk.offset
                }))
            }
            Some("find") => {
                let id = string_field(&request.payload, "book_id")?;
                let query = string_field(&request.payload, "query")?;
                if query.chars().count() > 256 { return Err("关键词不能超过 256 个字符".into()); }
                let from_offset = request.payload.get("from_offset").and_then(Value::as_u64).unwrap_or(0);
                let direction = request.payload.get("direction").and_then(Value::as_i64).unwrap_or(1);
                let book = load_config(context).books.into_iter().find(|book| book.id == id).ok_or_else(|| "书不存在".to_string())?;
                let text = load_text(context, &book.path)?;
                Ok(json!({
                    "offset": find_text(&text, &query, from_offset, direction),
                    "total": text.total_chars
                }))
            }
            Some("save_progress") => { let id = string_field(&request.payload, "book_id")?; let offset = request.payload.get("offset").and_then(Value::as_u64).unwrap_or(0); let history = request.payload.get("page_history").and_then(Value::as_array).map(|values| values.iter().filter_map(Value::as_u64).collect()).unwrap_or_default(); save_progress(context, &id, offset, history)?; emit_progress(context, &id, offset); Ok(Value::Null) }
            Some("open_book") => open_book(context, string_field(&request.payload, "book_id")?),
            Some("jump_to") => {
                let id = string_field(&request.payload, "book_id")?;
                let offset = request.payload.get("offset").and_then(Value::as_u64).unwrap_or(0);
                jump_to(context, id, offset)
            }
            Some("toggle_reader") => { toggle_reader(context)?; Ok(Value::Null) }
            Some("save_reader_position") => {
                let x = request.payload.get("x").and_then(Value::as_i64).unwrap_or(0).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                let y = request.payload.get("y").and_then(Value::as_i64).unwrap_or(0).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                save_reader_position(context, x, y)?;
                Ok(Value::Null)
            }
            Some("reader_hidden") => {
                let x = request.payload.get("x").and_then(Value::as_i64).unwrap_or(0).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                let y = request.payload.get("y").and_then(Value::as_i64).unwrap_or(0).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                remember_reader_hidden(context, x, y)?;
                Ok(Value::Null)
            }
            Some("save_style") => { let style: ThiefStyle = parse(&request.payload, "style")?; save_style(context, style) }
            Some("set_pinned") => {
                let pinned = request.payload.get("pinned").and_then(Value::as_bool).unwrap_or(false);
                let config = {
                    let _guard = config_lock().lock().unwrap();
                    let mut config = load_config(context);
                    config.style.pinned = pinned;
                    save_config(context, &config)?;
                    config
                };
                if config.reader_visible || current_reader_position(context)?.is_some() {
                    context.windows().set_clickthrough(READER_WINDOW, pinned).map_err(|error| error.message)?;
                    context.windows().emit(READER_WINDOW, STYLE_EVENT, json!(config.style)).map_err(|error| error.message)?;
                }
                Ok(Value::Null)
            }
            Some("rename_book") => { let id = string_field(&request.payload, "id")?; let name = string_field(&request.payload, "name")?; let name = name.trim(); if name.is_empty() || name.chars().count() > 60 { return Err("展示名无效".into()); } let _guard = config_lock().lock().unwrap(); let mut config = load_config(context); let book = config.books.iter_mut().find(|book| book.id == id).ok_or_else(|| "书不存在".to_string())?; book.name = name.into(); let result = book.clone(); save_config(context, &config)?; Ok(json!(result)) }
            Some("notify_progress") => { emit_progress(context, &string_field(&request.payload, "book_id")?, request.payload.get("offset").and_then(Value::as_u64).unwrap_or(0)); Ok(Value::Null) }
            Some(method) => Err(format!("不支持的书架方法: {method}")),
            None => Err("缺少插件方法".into()),
        }
    }
}

impl Plugin for ThiefbookPlugin {
    fn manifest(&self) -> &PluginManifest { &self.manifest }
    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        match request.kind {
            PluginRequestKind::Activate => { HOTKEYS_ACTIVE.store(true, Ordering::SeqCst); schedule_hotkey_registration(context); start_desktop_monitor(context); success_response(&request, Value::Null) }
            PluginRequestKind::Deactivate | PluginRequestKind::Shutdown => { HOTKEYS_ACTIVE.store(false, Ordering::SeqCst); stop_desktop_monitor(); let _ = context.shortcuts().unregister(context.plugin_id()); success_response(&request, Value::Null) }
            PluginRequestKind::Call => match self.call(&request, context) { Ok(result) => success_response(&request, result), Err(message) => error_response(request, message) },
            PluginRequestKind::GetInfo => success_response(&request, json!(self.manifest())),
        }
    }
}

fn error_response(request: PluginRequest, message: String) -> PluginResponse { PluginResponse { request_id: request.request_id, plugin_id: request.plugin_id, ok: false, result: Value::Null, error: Some(PluginError { code: ErrorCode::Internal, message }) } }

fn parse<T: for<'de> Deserialize<'de>>(payload: &Value, key: &str) -> Result<T, String> { payload.get(key).cloned().ok_or_else(|| format!("缺少字段: {key}")).and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string())) }
fn string_field(payload: &Value, key: &str) -> Result<String, String> { payload.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("缺少字段: {key}")) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_chunks_preserve_unicode_and_newline_offsets() {
        let text = cache_text("中文abc\r\ndef😀xyz".into());
        let first = read_chunk(&text, 0, 64);
        assert_eq!(first.text, "中文abc\r\ndef😀xyz");
        assert_eq!(first.offset, 0);
        assert_eq!(first.total, 14);

        let second = read_chunk(&text, 5, 64);
        assert_eq!(second.text, "\r\ndef😀xyz");
        assert_eq!(second.offset, 5);
        assert_eq!(read_chunk(&text, 100, 64).text, "");
    }

    #[test]
    fn reader_dimensions_keep_width_and_fit_exact_row_height() {
        let style = ThiefStyle { font_size: 20, width: 560, lines: 2, ..Default::default() };
        assert_eq!(reader_width(&style), 560.0);
        assert_eq!(reader_height(&style), 88.0);
        for font_size in [10, 32] {
            for lines in [1, 2, 20] {
                let style = ThiefStyle { font_size, lines, ..Default::default() };
                assert_eq!(reader_height(&style), (f64::from(lines) * f64::from(font_size) * 1.9 + 12.0).ceil());
            }
        }
    }

    #[test]
    fn clamp_style_keeps_supported_bounds() {
        let value = clamp_style(&ThiefStyle { font_size: 99, opacity: 4.0, width: 10, lines: 0, color: "bad".into(), ..Default::default() });
        assert_eq!(value.font_size, 32);
        assert_eq!(value.opacity, 1.0);
        assert_eq!(value.width, 240);
        assert_eq!(value.lines, 1);
        assert_eq!(value.color, "#e8e8ea");
    }
}
