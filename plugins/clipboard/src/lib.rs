use agile_tiles_plugin_sdk::{
    ErrorCode, PluginError, PluginEvent, PluginManifest, PluginRequest, PluginRequestKind, PluginResponse,
    PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION, MANIFEST_VERSION,
};
use agile_tiles_plugin_sdk::{success_response, HostServices, Plugin, PluginContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const PLUGIN_ID: &str = "clipboard";
const STATE_KEY: &str = "state";

agile_tiles_plugin_sdk::export_plugin_dylib!(ClipboardPlugin);
const POLL_MS: u64 = 1500;
const MAX_TEXT_CHARS: usize = 100_000;
const DEFAULT_MAX: u32 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ClipItem {
    pub id: String,
    pub text: String,
    pub ts: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipboardStore {
    pub enabled: bool,
    pub max: u32,
    pub items: Vec<ClipItem>,
}

impl Default for ClipboardStore {
    fn default() -> Self {
        Self {
            enabled: true,
            max: DEFAULT_MAX,
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardSettings {
    pub enabled: bool,
    pub max: u32,
}

pub struct ClipboardPlugin {
    manifest: PluginManifest,
    stop: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    poller: Option<JoinHandle<()>>,
    lock: Arc<Mutex<()>>,
    last_seen: Arc<Mutex<Option<String>>>,
}

impl Default for ClipboardPlugin {
    fn default() -> Self { Self::new() }
}

impl ClipboardPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: agile_tiles_plugin_sdk::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "剪贴板".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "剪贴板历史：自动记录文本、搜索、一键复制".into(),
                author: "Agile Tiles".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec!["storage".into(), "clipboard".into()],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Html),
                    asset_root: "ui".into(),
                    initial_width: Some(580),
                    initial_height: Some(760),
                    ..Default::default()
                }),
                hover_preview: None,
                auto_start: true,
                minimum_host_version: None,
            },
            stop: Arc::new(AtomicBool::new(false)),
            enabled: Arc::new(AtomicBool::new(true)),
            poller: None,
            lock: Arc::new(Mutex::new(())),
            last_seen: Arc::new(Mutex::new(None)),
        }
    }

    fn start_poller(&mut self, context: &PluginContext) {
        if self.poller.is_some() {
            return;
        }
        self.stop.store(false, Ordering::SeqCst);
        self.enabled.store(load(context).enabled, Ordering::SeqCst);
        let stop = Arc::clone(&self.stop);
        let enabled = Arc::clone(&self.enabled);
        let lock = Arc::clone(&self.lock);
        let last_seen = Arc::clone(&self.last_seen);
        let services = context.services_arc();
        let plugin_id = context.plugin_id().clone();
        self.poller = Some(thread::spawn(move || {
            poll_loop(services, plugin_id, stop, enabled, lock, last_seen);
        }));
    }

    fn stop_poller(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.poller.take() {
            let _ = handle.join();
        }
    }

    fn call(&self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match request.method.as_deref() {
            Some("list") => {
                let mut items = load(context).items;
                items.sort_by(|left, right| {
                    right.pinned.cmp(&left.pinned).then(right.ts.cmp(&left.ts))
                });
                Ok(json!(items))
            }
            Some("get_settings") => {
                let store = load(context);
                Ok(json!(ClipboardSettings {
                    enabled: store.enabled,
                    max: store.max,
                }))
            }
            Some("set_enabled") => {
                let enabled = bool_field(&request.payload, "enabled")?;
                let mut store = load(context);
                store.enabled = enabled;
                save(context, &store)?;
                self.enabled.store(enabled, Ordering::SeqCst);
                Ok(Value::Null)
            }
            Some("set_max") => {
                let max = request
                    .payload
                    .get("max")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "参数 max 必须是数字".to_string())?
                    .clamp(10, 1000) as u32;
                let mut store = load(context);
                store.max = max;
                store.items.truncate(max as usize);
                save(context, &store)?;
                Ok(Value::Null)
            }
            Some("copy_item") => {
                let id = string_field(&request.payload, "id")?;
                let mut store = load(context);
                let index = store
                    .items
                    .iter()
                    .position(|item| item.id == id)
                    .ok_or_else(|| "记录不存在".to_string())?;
                let text = store.items[index].text.clone();
                context
                    .clipboard()
                    .write_text(&text)
                    .map_err(|error| error.message)?;
                let mut item = store.items.remove(index);
                item.ts = now_string();
                store.items.insert(0, item);
                save(context, &store)?;
                self.last_seen
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .replace(text);
                Ok(Value::Null)
            }
            Some("delete_item") => {
                let id = string_field(&request.payload, "id")?;
                let mut store = load(context);
                let before = store.items.len();
                store.items.retain(|item| item.id != id);
                if store.items.len() == before {
                    return Err("记录不存在".into());
                }
                save(context, &store)?;
                Ok(Value::Null)
            }
            Some("clear") => {
                let mut store = load(context);
                store.items.clear();
                save(context, &store)?;
                Ok(Value::Null)
            }
            Some("pin_toggle") => {
                let id = string_field(&request.payload, "id")?;
                let mut store = load(context);
                let item = store
                    .items
                    .iter_mut()
                    .find(|item| item.id == id)
                    .ok_or_else(|| "记录不存在".to_string())?;
                item.pinned = !item.pinned;
                save(context, &store)?;
                Ok(Value::Null)
            }
            Some(method) => Err(format!("不支持的剪贴板方法: {method}")),
            None => Err("缺少插件方法".into()),
        }
    }
}

impl Plugin for ClipboardPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        match request.kind {
            PluginRequestKind::Activate => {
                self.start_poller(context);
                success_response(&request, Value::Null)
            }
            PluginRequestKind::Deactivate | PluginRequestKind::Shutdown => {
                self.stop_poller();
                success_response(&request, Value::Null)
            }
            PluginRequestKind::Call => match self.call(&request, context) {
                Ok(result) => success_response(&request, result),
                Err(message) => error_response(&request, message),
            },
            PluginRequestKind::GetInfo => success_response(&request, Value::Null),
        }
    }
}

fn poll_loop(
    services: Arc<dyn HostServices>,
    plugin_id: agile_tiles_plugin_sdk::PluginId,
    stop: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    lock: Arc<Mutex<()>>,
    last_seen: Arc<Mutex<Option<String>>>,
) {
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(POLL_MS));
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if !enabled.load(Ordering::SeqCst) {
            continue;
        }
        let Ok(Some(text)) = services.clipboard().read_text() else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let text = truncate_chars(&text, MAX_TEXT_CHARS);
        {
            let mut last = last_seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if last.as_deref() == Some(text.as_str()) {
                continue;
            }
            *last = Some(text.clone());
        }
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut store = load_with(&*services, &plugin_id);
        if !store.enabled {
            enabled.store(false, Ordering::SeqCst);
            continue;
        }
        store.items.retain(|item| item.text != text);
        store.items.insert(
            0,
            ClipItem {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                ts: now_string(),
                pinned: false,
            },
        );
        let pinned_count = store.items.iter().filter(|item| item.pinned).count();
        store.items.truncate((store.max as usize).max(pinned_count));
        if save_with(&*services, &plugin_id, &store).is_ok() {
            let _ = services.events().publish(PluginEvent {
                plugin_id: plugin_id.clone(),
                topic: "clipboard.changed".into(),
                payload: Value::Null,
            });
        }
    }
}

fn load(context: &PluginContext) -> ClipboardStore {
    load_with(context.services(), context.plugin_id())
}

fn load_with(
    services: &dyn HostServices,
    plugin_id: &agile_tiles_plugin_sdk::PluginId,
) -> ClipboardStore {
    services
        .storage()
        .get(plugin_id, STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn save(context: &PluginContext, store: &ClipboardStore) -> Result<(), String> {
    save_with(context.services(), context.plugin_id(), store)
}

fn save_with(
    services: &dyn HostServices,
    plugin_id: &agile_tiles_plugin_sdk::PluginId,
    store: &ClipboardStore,
) -> Result<(), String> {
    let value = serde_json::to_string(store).map_err(|error| error.to_string())?;
    services
        .storage()
        .set(plugin_id, STATE_KEY, &value)
        .map_err(|error| error.message)
}

fn bool_field(payload: &Value, field: &str) -> Result<bool, String> {
    payload
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("参数 {field} 必须是布尔值"))
}

fn string_field(payload: &Value, field: &str) -> Result<String, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("参数 {field} 必须是字符串"))
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn now_string() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn error_response(request: &PluginRequest, message: String) -> PluginResponse {
    PluginResponse {
        request_id: request.request_id.clone(),
        plugin_id: request.plugin_id.clone(),
        ok: false,
        result: Value::Null,
        error: Some(PluginError {
            code: ErrorCode::Internal,
            message,
        }),
    }
}
