use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind,
    PluginResponse, PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION,
    MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{
    success_response, HostServices, Plugin, PluginContext, WindowEvent, WindowEventHandler,
    WindowSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

const PLUGIN_ID: &str = "notes";
const STATE_KEY: &str = "state";

sideweave_plugin_sdk::export_plugin_dylib!(NotesPlugin);
const NOTE_W: f64 = 260.0;
const NOTE_H: f64 = 200.0;
const TODO_W: f64 = 280.0;
const TODO_H: f64 = 340.0;
const NOTE_SKINS: &[&str] = &["classic", "lined", "grid", "minimal", "terminal", "neon"];
const NOTE_COLORS: &[&str] = &["#f9e2af", "#a6e3a1", "#89b4fa", "#f5c2e7", "#f38ba8", "#94e2d5"];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TodoItem {
    pub id: String,
    pub text: String,
    pub done: bool,
    pub done_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Note {
    pub id: String,
    pub kind: String,
    pub skin: String,
    pub text: String,
    pub items: Vec<TodoItem>,
    pub color: String,
    pub opacity: f64,
    pub pos_x: Option<i32>,
    pub pos_y: Option<i32>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub visible: bool,
    pub deleted: bool,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct NotesState {
    notes: Vec<Note>,
}

pub struct NotesPlugin {
    manifest: PluginManifest,
}

impl Default for NotesPlugin {
    fn default() -> Self { Self::new() }
}

impl NotesPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "便签".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "桌面便签：文本便签与任务清单小窗".into(),
                author: "SideWeave".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec![
                    "storage".into(),
                    "window.create".into(),
                    "window.manage".into(),
                    "event.publish".into(),
                ],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Html),
                    asset_root: "ui".into(),
                    initial_width: Some(720),
                    initial_height: Some(680),
                    ..Default::default()
                }),
                hover_preview: None,
                auto_start: true,
                minimum_host_version: None,
            },
        }
    }

    fn call(&self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        match request.method.as_deref() {
            Some("list") => Ok(json!(load(context).notes)),
            Some("create") => create(context, string_field(&request.payload, "kind")?),
            Some("delete") => delete(context, string_field(&request.payload, "id")?),
            Some("restore") => restore(context, string_field(&request.payload, "id")?),
            Some("purge") => purge(context, string_field(&request.payload, "id")?),
            Some("clear_history") => clear_history(context),
            Some("toggle_visible") => toggle_visible(context, string_field(&request.payload, "id")?),
            Some("update_text") => update_text(
                context,
                string_field(&request.payload, "id")?,
                string_field(&request.payload, "text")?,
            ),
            Some("set_style") => set_style(
                context,
                string_field(&request.payload, "id")?,
                string_field(&request.payload, "color")?,
                request.payload.get("opacity").and_then(Value::as_f64).unwrap_or(0.95),
            ),
            Some("set_skin") => set_skin(
                context,
                string_field(&request.payload, "id")?,
                string_field(&request.payload, "skin")?,
            ),
            Some("get") => {
                let id = string_field(&request.payload, "id")?;
                load(context)
                    .notes
                    .into_iter()
                    .find(|note| note.id == id)
                    .map(|note| json!(note))
                    .ok_or_else(|| "便签不存在".into())
            }
            Some("add_item") => add_item(
                context,
                string_field(&request.payload, "id")?,
                string_field(&request.payload, "text")?,
            ),
            Some("set_item_done") => set_item_done(
                context,
                string_field(&request.payload, "id")?,
                string_field(&request.payload, "item_id")?,
                request.payload.get("done").and_then(Value::as_bool).unwrap_or(false),
            ),
            Some("edit_item") => edit_item(
                context,
                string_field(&request.payload, "id")?,
                string_field(&request.payload, "item_id")?,
                string_field(&request.payload, "text")?,
            ),
            Some("remove_item") => remove_item(
                context,
                string_field(&request.payload, "id")?,
                string_field(&request.payload, "item_id")?,
            ),
            Some(method) => Err(format!("不支持的便签方法: {method}")),
            None => Err("缺少插件方法".into()),
        }
    }
}

impl Plugin for NotesPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if matches!(request.kind, PluginRequestKind::Activate) {
            return match restore_visible(context) {
                Ok(()) => success_response(&request, Value::Null),
                Err(message) => PluginResponse {
                    request_id: request.request_id,
                    plugin_id: request.plugin_id,
                    ok: false,
                    result: Value::Null,
                    error: Some(PluginError {
                        code: ErrorCode::Internal,
                        message,
                    }),
                },
            };
        }
        if !matches!(request.kind, PluginRequestKind::Call) {
            return success_response(&request, Value::Null);
        }
        match self.call(&request, context) {
            Ok(result) => success_response(&request, result),
            Err(message) => PluginResponse {
                request_id: request.request_id,
                plugin_id: request.plugin_id,
                ok: false,
                result: Value::Null,
                error: Some(PluginError {
                    code: ErrorCode::Internal,
                    message,
                }),
            },
        }
    }
}

fn load(context: &PluginContext) -> NotesState {
    let mut state = context
        .storage()
        .get(context.plugin_id(), STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<NotesState>(&value).ok())
        .unwrap_or_default();
    for note in &mut state.notes {
        if !NOTE_SKINS.contains(&note.skin.as_str()) {
            note.skin = "classic".into();
        }
    }
    state
}

fn save(context: &PluginContext, state: &NotesState) -> Result<(), String> {
    let value = serde_json::to_string(state).map_err(|error| error.to_string())?;
    context
        .storage()
        .set(context.plugin_id(), STATE_KEY, &value)
        .map_err(|error| error.message)
}

fn publish_changed(context: &PluginContext) {
    let _ = context.events().publish(sideweave_plugin_api::PluginEvent {
        plugin_id: context.plugin_id().clone(),
        topic: "notes.changed".into(),
        payload: Value::Null,
    });
}

fn create(context: &PluginContext, kind: String) -> Result<Value, String> {
    let kind = if kind == "todo" { "todo" } else { "text" }.to_string();
    let note = Note {
        id: uuid::Uuid::new_v4().to_string(),
        kind,
        skin: "classic".into(),
        text: String::new(),
        items: Vec::new(),
        color: NOTE_COLORS[0].into(),
        opacity: 0.95,
        pos_x: None,
        pos_y: None,
        width: None,
        height: None,
        visible: true,
        deleted: false,
        deleted_at: None,
    };
    let mut state = load(context);
    state.notes.push(note.clone());
    save(context, &state)?;
    create_window(context, &note)?;
    publish_changed(context);
    Ok(json!(note))
}

fn delete(context: &PluginContext, id: String) -> Result<Value, String> {
    let (purged, note_exists) = {
        let mut state = load(context);
        let Some(note) = state.notes.iter_mut().find(|note| note.id == id) else {
            return Err("便签不存在".into());
        };
        let blank = if note.kind == "todo" {
            note.items.is_empty()
        } else {
            note.text.trim().is_empty()
        };
        if blank {
            state.notes.retain(|note| note.id != id);
        } else {
            note.deleted = true;
            note.deleted_at = Some(now_ms());
            note.visible = false;
        }
        save(context, &state)?;
        (blank, true)
    };
    let _ = context.windows().close(&window_id(&id));
    publish_changed(context);
    Ok(json!(purged && note_exists))
}

fn restore(context: &PluginContext, id: String) -> Result<Value, String> {
    let note = {
        let mut state = load(context);
        let Some(note) = state.notes.iter_mut().find(|note| note.id == id) else {
            return Err("便签不存在".into());
        };
        if !note.deleted {
            return Err("便签不在历史中".into());
        }
        note.deleted = false;
        note.deleted_at = None;
        note.visible = true;
        let snapshot = note.clone();
        save(context, &state)?;
        snapshot
    };
    create_window(context, &note)?;
    publish_changed(context);
    Ok(Value::Null)
}

fn purge(context: &PluginContext, id: String) -> Result<Value, String> {
    let mut state = load(context);
    let before = state.notes.len();
    state.notes.retain(|note| !(note.id == id && note.deleted));
    if before == state.notes.len() {
        return Err("便签不存在或不在历史中".into());
    }
    save(context, &state)?;
    let _ = context.windows().close(&window_id(&id));
    publish_changed(context);
    Ok(Value::Null)
}

fn clear_history(context: &PluginContext) -> Result<Value, String> {
    let mut state = load(context);
    state.notes.retain(|note| !note.deleted);
    save(context, &state)?;
    publish_changed(context);
    Ok(Value::Null)
}

fn toggle_visible(context: &PluginContext, id: String) -> Result<Value, String> {
    let note = {
        let mut state = load(context);
        let Some(note) = state.notes.iter_mut().find(|note| note.id == id) else {
            return Err("便签不存在".into());
        };
        if note.deleted {
            return Err("便签已在历史中，请先恢复".into());
        }
        note.visible = !note.visible;
        let snapshot = note.clone();
        save(context, &state)?;
        snapshot
    };
    if note.visible {
        create_window(context, &note)?;
    } else {
        context.windows().close(&window_id(&id)).map_err(|e| e.message)?;
    }
    Ok(Value::Null)
}

fn update_text(context: &PluginContext, id: String, text: String) -> Result<Value, String> {
    mutate(context, &id, |note| note.text = text)?;
    Ok(Value::Null)
}

fn set_style(context: &PluginContext, id: String, color: String, opacity: f64) -> Result<Value, String> {
    let color = clamp_color(&color);
    let opacity = if opacity.is_finite() { opacity.clamp(0.3, 1.0) } else { 0.95 };
    mutate(context, &id, |note| {
        note.color = color.clone();
        note.opacity = opacity;
    })?;
    context
        .windows()
        .emit(&window_id(&id), "note://style", json!({ "id": id, "color": color, "opacity": opacity }))
        .map_err(|e| e.message)?;
    Ok(Value::Null)
}

fn set_skin(context: &PluginContext, id: String, skin: String) -> Result<Value, String> {
    let skin = if NOTE_SKINS.contains(&skin.trim()) { skin } else { "classic".into() };
    mutate(context, &id, |note| note.skin = skin.clone())?;
    context
        .windows()
        .emit(&window_id(&id), "note://skin", json!({ "id": id, "skin": skin }))
        .map_err(|e| e.message)?;
    publish_changed(context);
    Ok(Value::Null)
}

fn add_item(context: &PluginContext, id: String, text: String) -> Result<Value, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(Value::Null);
    }
    mutate(context, &id, |note| {
        note.items.push(TodoItem {
            id: uuid::Uuid::new_v4().to_string(),
            text,
            done: false,
            done_at: None,
        });
    })?;
    Ok(Value::Null)
}

fn set_item_done(context: &PluginContext, id: String, item_id: String, done: bool) -> Result<Value, String> {
    mutate(context, &id, |note| {
        if let Some(item) = note.items.iter_mut().find(|item| item.id == item_id) {
            item.done = done;
            item.done_at = if done { Some(now_ms()) } else { None };
        }
    })?;
    Ok(Value::Null)
}

fn edit_item(context: &PluginContext, id: String, item_id: String, text: String) -> Result<Value, String> {
    let text = text.trim().to_string();
    mutate(context, &id, |note| {
        if let Some(item) = note.items.iter_mut().find(|item| item.id == item_id) {
            if !text.is_empty() {
                item.text = text;
            }
        }
    })?;
    Ok(Value::Null)
}

fn remove_item(context: &PluginContext, id: String, item_id: String) -> Result<Value, String> {
    mutate(context, &id, |note| note.items.retain(|item| item.id != item_id))?;
    Ok(Value::Null)
}

fn mutate(context: &PluginContext, id: &str, f: impl FnOnce(&mut Note)) -> Result<(), String> {
    let mut state = load(context);
    let Some(note) = state.notes.iter_mut().find(|note| note.id == id) else {
        return Err("便签不存在".into());
    };
    if note.deleted {
        return Err("便签已在历史中".into());
    }
    f(note);
    save(context, &state)?;
    publish_changed(context);
    Ok(())
}

fn create_window(context: &PluginContext, note: &Note) -> Result<(), String> {
    let (default_width, default_height) = if note.kind == "todo" {
        (TODO_W, TODO_H)
    } else {
        (NOTE_W, NOTE_H)
    };
    let handler = NotesWindowHandler {
        services: context.services_arc(),
        plugin_id: context.plugin_id().clone(),
        note_id: note.id.clone(),
    };
    context
        .windows()
        .create(
            WindowSpec {
                id: window_id(&note.id),
                title: "便签".into(),
                url: format!("ui/note.html?note_id={}", note.id),
                width: note.width.unwrap_or(default_width),
                height: note.height.unwrap_or(default_height),
                position_px: note.pos_x.zip(note.pos_y),
                always_on_top: true,
                skip_taskbar: true,
                resizable: true,
                transparent: true,
                decorations: false,
                maximizable: false,
            },
            Some(Arc::new(handler)),
        )
        .map_err(|error| error.message)
}

fn restore_visible(context: &PluginContext) -> Result<(), String> {
    for note in load(context).notes.into_iter().filter(|note| note.visible && !note.deleted) {
        create_window(context, &note)?;
    }
    Ok(())
}

struct NotesWindowHandler {
    services: Arc<dyn HostServices>,
    plugin_id: sideweave_plugin_api::PluginId,
    note_id: String,
}

impl WindowEventHandler for NotesWindowHandler {
    fn on_event(&self, event: WindowEvent) {
        let Some(value) = self
            .services
            .storage()
            .get(&self.plugin_id, STATE_KEY)
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str::<NotesState>(&value).ok())
        else {
            return;
        };
        let mut state = value;
        if let Some(note) = state.notes.iter_mut().find(|note| note.id == self.note_id) {
            match event {
                WindowEvent::Moved { x, y } => {
                    note.pos_x = Some(x);
                    note.pos_y = Some(y);
                }
                WindowEvent::Resized { width, height } => {
                    note.width = Some(width.max(140.0));
                    note.height = Some(height.max(80.0));
                }
                WindowEvent::Closed => note.visible = false,
                WindowEvent::NativeHandleAvailable { .. }
                | WindowEvent::VisibilityChanged { .. } => return,
            }
            if let Ok(serialized) = serde_json::to_string(&state) {
                let _ = self.services.storage().set(&self.plugin_id, STATE_KEY, &serialized);
            }
        }
    }
}

fn string_field(payload: &Value, name: &str) -> Result<String, String> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("缺少字段: {name}"))
}

fn window_id(id: &str) -> String {
    format!("note-{id}")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn clamp_color(value: &str) -> String {
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if hex.len() == 6 && hex.chars().all(|character| character.is_ascii_hexdigit()) {
        format!("#{}", hex.to_ascii_lowercase())
    } else {
        NOTE_COLORS[0].into()
    }
}
