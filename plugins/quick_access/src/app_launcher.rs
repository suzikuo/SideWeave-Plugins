use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind, PluginResponse,
    PluginRuntime, PluginTrust, API_VERSION, MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

const PLUGIN_ID: &str = "app_launcher";
const STATE_KEY: &str = "quick_access_apps";
const ICON_CACHE_CAPACITY: usize = 128;


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub app_type: String,
    pub exe_path: String,
    pub arguments: String,
    pub use_ahk: bool,
    pub ahk_path: String,
    pub window_x: i32,
    pub window_y: i32,
    pub window_width: i32,
    pub window_height: i32,
    pub always_on_top: bool,
    #[serde(default)]
    pub in_preview: bool,
}

impl Default for AppEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            app_type: "application".into(),
            exe_path: String::new(),
            arguments: String::new(),
            use_ahk: false,
            ahk_path: String::new(),
            window_x: 200,
            window_y: 200,
            window_width: 160,
            window_height: 400,
            always_on_top: true,
            in_preview: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEntryInput {
    pub name: String,
    pub app_type: String,
    pub exe_path: String,
    pub arguments: String,
    pub use_ahk: bool,
    pub ahk_path: String,
    pub window_x: i32,
    pub window_y: i32,
    pub window_width: i32,
    pub window_height: i32,
    pub always_on_top: bool,
    #[serde(default)]
    pub in_preview: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct LauncherState {
    apps: Vec<AppEntry>,
}

pub struct AppLauncherPlugin {
    manifest: PluginManifest,
    icon_cache: Mutex<IconCache>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct IconCacheKey {
    path: String,
    file_len: Option<u64>,
    modified_ns: Option<u128>,
}

#[derive(Default)]
struct IconCache {
    values: HashMap<IconCacheKey, String>,
    recent: VecDeque<IconCacheKey>,
}

impl IconCache {
    fn get(&mut self, key: &IconCacheKey) -> Option<String> {
        let value = self.values.get(key)?.clone();
        if let Some(index) = self.recent.iter().position(|item| item == key) {
            self.recent.remove(index);
        }
        self.recent.push_back(key.clone());
        Some(value)
    }

    fn insert(&mut self, key: IconCacheKey, value: String) {
        if self.values.contains_key(&key) {
            self.recent.retain(|item| item != &key);
        } else if self.values.len() >= ICON_CACHE_CAPACITY {
            if let Some(oldest) = self.recent.pop_front() {
                self.values.remove(&oldest);
            }
        }
        self.recent.push_back(key.clone());
        self.values.insert(key, value);
    }
}

impl Default for AppLauncherPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl AppLauncherPlugin {
    pub fn new() -> Self {
        Self {
            icon_cache: Mutex::new(IconCache::default()),
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "App Launcher".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "应用启动器：快速启动本地应用与 Chrome 应用窗口".into(),
                author: "SideWeave".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec![
                    "storage".into(),
                    "filesystem.read".into(),
                    "process.spawn".into(),
                    "system.file_picker".into(),
                    "system.icon".into(),
                ],
                ui: Some(sideweave_plugin_api::PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(sideweave_plugin_api::PluginUiKind::Html),
                    asset_root: "ui".into(),
                    initial_width: Some(920),
                    initial_height: Some(680),
                    ..Default::default()
                }),
                hover_preview: Some(sideweave_plugin_api::PluginHoverPreview {
                    width: Some(280),
                    height: Some(250),
                    data_method: Some("get_preview".into()),
                    entry: None,
                }),
                auto_start: false,
                minimum_host_version: None,
            },
        }
    }

    fn call(&self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        match request.method.as_deref() {
            Some("list") => Ok(json!(load(context).apps)),
            Some("add") => {
                let input: AppEntryInput = field(&request.payload, "entry")?;
                validate(&input)?;
                let mut state = load(context);
                let entry = entry_from_input(uuid::Uuid::new_v4().to_string(), input);
                state.apps.push(entry.clone());
                save(context, &state)?;
                Ok(json!(entry))
            }
            Some("update") => {
                let id = string_field(&request.payload, "id")?;
                let input: AppEntryInput = field(&request.payload, "entry")?;
                validate(&input)?;
                let mut state = load(context);
                let entry = state
                    .apps
                    .iter_mut()
                    .find(|entry| entry.id == id)
                    .ok_or_else(|| "应用不存在".to_string())?;
                *entry = entry_from_input(id, input);
                let updated = entry.clone();
                save(context, &state)?;
                Ok(json!(updated))
            }
            Some("remove") => {
                let id = string_field(&request.payload, "id")?;
                let mut state = load(context);
                let before = state.apps.len();
                state.apps.retain(|entry| entry.id != id);
                if state.apps.len() == before {
                    return Err("应用不存在".into());
                }
                save(context, &state)?;
                Ok(Value::Null)
            }
            Some("launch") => {
                let id = string_field(&request.payload, "id")?;
                let entry = load(context)
                    .apps
                    .into_iter()
                    .find(|entry| entry.id == id)
                    .ok_or_else(|| "应用不存在".to_string())?;
                launch(context, &entry)?;
                Ok(Value::Null)
            }
            Some("chrome_preset") => Ok(json!(chrome_preset())),
            Some("pick_executable") => {
                let extensions = vec!["exe".to_string()];
                Ok(json!(context
                    .system()
                    .pick_file(&extensions)
                    .map_err(|error| error.message)?))
            }
            Some("icon") => {
                let path = string_field(&request.payload, "path")?;
                Ok(json!(self.extract_icon_cached(context, &path)?))
            }
            Some("toggle_preview") => {
                let id = string_field(&request.payload, "id")?;
                let mut state = load(context);
                let entry = state
                    .apps
                    .iter_mut()
                    .find(|entry| entry.id == id)
                    .ok_or_else(|| "应用不存在".to_string())?;
                entry.in_preview = !entry.in_preview;
                let in_preview = entry.in_preview;
                save(context, &state)?;
                Ok(json!({ "id": id, "in_preview": in_preview }))
            }
            Some("get_preview") => {
                let state = load(context);
                let preview_apps: Vec<_> = state.apps.iter().filter(|a| a.in_preview).collect();
                let total_apps = state.apps.len();
                let selected_count = preview_apps.len();

                let items: Vec<Value> = preview_apps
                    .into_iter()
                    .map(|app| {
                        let icon = self
                            .extract_icon_cached(context, &app.exe_path)
                            .ok()
                            .flatten();
                        json!({
                            "id": app.id,
                            "title": app.name,
                            "subtitle": if app.app_type == "chrome" { "Chrome Web App" } else { "本地应用" },
                            "icon": icon.map(|b64| if b64.starts_with("data:") { b64 } else { format!("data:image/png;base64,{b64}") }),
                            "method": "launch",
                            "payload": { "id": app.id }
                        })
                    })
                    .collect();

                Ok(json!({
                    "title": "应用启动器",
                    "subtitle": format!("已选 {selected_count}/{total_apps} 个应用"),
                    "status": if selected_count > 0 { "online" } else { "info" },
                    "statusText": format!("{selected_count} 个快捷方式"),
                    "items": items,
                    "actions": [
                        { "id": "open", "label": "管理应用", "primary": false }
                    ]
                }))
            }
            Some(method) => Err(format!("不支持的应用启动器方法: {method}")),
            None => Err("缺少插件方法".into()),
        }
    }

    fn extract_icon_cached(
        &self,
        context: &PluginContext,
        path: &str,
    ) -> Result<Option<String>, String> {
        let key = icon_cache_key(path);
        if let Some(icon) = self
            .icon_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
        {
            return Ok(Some(icon));
        }

        let icon = context
            .system()
            .extract_icon(path)
            .map_err(|error| error.message)?;
        if let Some(value) = icon.as_ref() {
            self.icon_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(key, value.clone());
        }
        Ok(icon)
    }
}

fn icon_cache_key(path: &str) -> IconCacheKey {
    let normalized_path = fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let path = normalized_path.to_string_lossy().into_owned();
    #[cfg(windows)]
    let path = path.to_lowercase();
    let metadata = fs::metadata(path.as_str()).ok();
    let file_len = metadata.as_ref().map(fs::Metadata::len);
    let modified_ns = metadata
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos());
    IconCacheKey {
        path,
        file_len,
        modified_ns,
    }
}

impl Plugin for AppLauncherPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if !matches!(request.kind, PluginRequestKind::Call) {
            return success_response(&request, Value::Null);
        }
        match self.call(&request, context) {
            Ok(result) => success_response(&request, result),
            Err(message) => error_response(&request, message),
        }
    }
}

fn load(context: &PluginContext) -> LauncherState {
    let current = context
        .storage()
        .get(context.plugin_id(), STATE_KEY)
        .ok()
        .flatten();
    let stored = current.or_else(|| {
        let legacy_id = sideweave_plugin_api::PluginId::new(PLUGIN_ID).ok()?;
        let value = context.storage().get(&legacy_id, "state").ok().flatten()?;
        let _ = context.storage().set(context.plugin_id(), STATE_KEY, &value);
        Some(value)
    });
    stored
        .and_then(|value| serde_json::from_str(&value).ok())
        .map(normalize_state)
        .unwrap_or_default()
}

fn save(context: &PluginContext, state: &LauncherState) -> Result<(), String> {
    let value = serde_json::to_string(state).map_err(|error| error.to_string())?;
    context
        .storage()
        .set(context.plugin_id(), STATE_KEY, &value)
        .map_err(|error| error.message)
}

fn normalize_state(value: Value) -> LauncherState {
    let mut state: LauncherState = serde_json::from_value(value).unwrap_or_default();
    for entry in &mut state.apps {
        if entry.id.trim().is_empty() {
            entry.id = uuid::Uuid::new_v4().to_string();
        }
    }
    state
}

fn entry_from_input(id: String, input: AppEntryInput) -> AppEntry {
    AppEntry {
        id,
        name: input.name,
        app_type: input.app_type,
        exe_path: input.exe_path,
        arguments: input.arguments,
        use_ahk: input.use_ahk,
        ahk_path: input.ahk_path,
        window_x: input.window_x,
        window_y: input.window_y,
        window_width: input.window_width,
        window_height: input.window_height,
        always_on_top: input.always_on_top,
        in_preview: input.in_preview,
    }
}

fn validate(input: &AppEntryInput) -> Result<(), String> {
    if input.name.trim().is_empty() {
        return Err("Name 不能为空".into());
    }
    if input.exe_path.trim().is_empty() {
        return Err("Exe Path 不能为空".into());
    }
    if input.app_type != "application" && input.app_type != "chrome" {
        return Err("app_type 仅支持 application / chrome".into());
    }
    Ok(())
}

fn launch(context: &PluginContext, entry: &AppEntry) -> Result<(), String> {
    let exe = PathBuf::from(expand_env(&entry.exe_path).trim().trim_matches('"'));
    if !exe.is_file() {
        return Err(format!("找不到可执行文件: {}", entry.exe_path));
    }
    let args = parse_windows_args(&expand_env(&entry.arguments));
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    let args = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    if entry.app_type == "chrome" && entry.use_ahk {
        let ahk = find_ahk(&entry.ahk_path).ok_or("未找到 AutoHotkey，请安装或在编辑框填写路径")?;
        let script = write_ahk_script(entry, &ahk)?;
        context
            .system()
            .spawn(
                &ahk.to_string_lossy(),
                &[script.to_string_lossy().into_owned()],
            )
            .map_err(|error| format!("启动 AHK 失败: {}", error.message))?;
    } else {
        let cwd = exe.parent().map(|path| path.to_string_lossy().into_owned());
        context
            .system()
            .spawn_with_cwd(&exe.to_string_lossy(), &args, cwd.as_deref())
            .map_err(|error| format!("启动失败: {}", error.message))?;
    }
    Ok(())
}

fn write_ahk_script(entry: &AppEntry, ahk: &Path) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("sideweave_app-launcher");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let script = dir.join(format!("{}.ahk", entry.id));
    let exe = entry.exe_path.replace('"', r#"\"#);
    let args = entry.arguments.replace('"', r#"\"#);
    let is_v2 = ahk.to_string_lossy().contains(r"\v2\");
    let body = if is_v2 {
        format!(
            "#SingleInstance Force\nRun \"{}\" {}\nWinWaitActive ahk_exe chrome.exe, , 10\nif (WinExist(\"ahk_exe chrome.exe\")) {{\n  WinMove {}, {}, {}, {}, \"ahk_exe chrome.exe\"\n{}\n}}\n",
            exe,
            args,
            entry.window_x,
            entry.window_y,
            entry.window_width,
            entry.window_height,
            if entry.always_on_top {
                "  WinSetAlwaysOnTop 1, \"ahk_exe chrome.exe\""
            } else {
                ""
            }
        )
    } else {
        format!(
            "#SingleInstance Force\nRun, \"{}\" {}\nWinWaitActive, ahk_exe chrome.exe, , 10\nWinMove, ahk_exe chrome.exe, , {}, {}, {}, {}\n",
            exe, args, entry.window_x, entry.window_y, entry.window_width, entry.window_height,
        ) + if entry.always_on_top {
            "WinSet, AlwaysOnTop, On, ahk_exe chrome.exe\n"
        } else {
            ""
        }
    };
    std::fs::write(&script, body).map_err(|error| error.to_string())?;
    Ok(script)
}

fn find_ahk(configured: &str) -> Option<PathBuf> {
    if !configured.trim().is_empty() {
        let path = PathBuf::from(configured);
        if path.exists() {
            return Some(path);
        }
    }
    let mut candidates = Vec::new();
    for base in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
        if let Some(dir) = std::env::var_os(base) {
            let base = PathBuf::from(dir);
            candidates.push(base.join("AutoHotkey").join("AutoHotkey.exe"));
            candidates.push(base.join("AutoHotkey").join("v2").join("AutoHotkey64.exe"));
            candidates.push(base.join("AutoHotkey").join("v2").join("AutoHotkey.exe"));
        }
    }
    for name in ["AutoHotkey64.exe", "AutoHotkey32.exe", "AutoHotkey.exe"] {
        if let Some(path) = which(name) {
            candidates.push(path);
        }
    }
    candidates.into_iter().find(|path| path.exists())
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

fn find_chrome() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .map(|base| {
            base.join("Google")
                .join("Chrome")
                .join("Application")
                .join("chrome.exe")
        })
        .find(|path| path.exists())
}

fn chrome_preset() -> Option<AppEntryInput> {
    let chrome = find_chrome()?;
    let user_data = std::env::temp_dir().join("linuxdo-app");
    Some(AppEntryInput {
        name: "linux.do".into(),
        app_type: "chrome".into(),
        exe_path: chrome.to_string_lossy().into_owned(),
        arguments: format!(
            r#"--window-size=300,600 --app=https://linux.do/ --user-data-dir="{}""#,
            user_data.to_string_lossy()
        ),
        use_ahk: true,
        ahk_path: String::new(),
        window_x: 200,
        window_y: 200,
        window_width: 160,
        window_height: 400,
        always_on_top: true,
        in_preview: false,
    })
}

fn parse_windows_args(value: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut has_token = false;
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            ' ' | '\t' if !quoted => {
                if has_token {
                    output.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            '"' => quoted = !quoted,
            '\\' => {
                let mut count = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    count += 1;
                }
                if chars.peek() == Some(&'"') {
                    for _ in 0..count / 2 {
                        current.push('\\');
                    }
                    if count % 2 == 1 {
                        chars.next();
                        current.push('"');
                    }
                } else {
                    for _ in 0..count {
                        current.push('\\');
                    }
                }
            }
            _ => current.push(character),
        }
        if !matches!(character, ' ' | '\t') || quoted {
            has_token = true;
        }
    }
    if has_token || !current.is_empty() {
        output.push(current);
    }
    output
}

fn expand_env(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        output.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        match rest.find('%') {
            Some(end) => {
                let name = &rest[..end];
                output.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
                rest = &rest[end + 1..];
            }
            None => {
                output.push('%');
                break;
            }
        }
    }
    output.push_str(rest);
    output
}

fn field<T: for<'de> Deserialize<'de>>(payload: &Value, name: &str) -> Result<T, String> {
    payload
        .get(name)
        .cloned()
        .ok_or_else(|| format!("缺少参数: {name}"))
        .and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
}

fn string_field(payload: &Value, name: &str) -> Result<String, String> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("参数 {name} 必须是字符串"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_key(index: usize) -> IconCacheKey {
        IconCacheKey {
            path: format!("C:\\Apps\\app-{index}.exe"),
            file_len: Some(index as u64),
            modified_ns: Some(index as u128),
        }
    }

    #[test]
    fn icon_cache_hits_and_invalidates_when_metadata_changes() {
        let mut cache = IconCache::default();
        let original = IconCacheKey {
            path: "C:\\Apps\\target.exe".into(),
            file_len: Some(100),
            modified_ns: Some(500),
        };
        let changed = IconCacheKey {
            file_len: Some(101),
            modified_ns: Some(501),
            ..original.clone()
        };
        cache.insert(original.clone(), "icon-data".into());

        assert_eq!(cache.get(&original).as_deref(), Some("icon-data"));
        assert!(cache.get(&changed).is_none());
    }

    #[test]
    fn icon_cache_evicts_least_recently_used_items_at_capacity() {
        let mut cache = IconCache::default();
        for index in 0..ICON_CACHE_CAPACITY {
            cache.insert(cache_key(index), format!("icon-{index}"));
        }
        assert!(cache.get(&cache_key(0)).is_some());
        cache.insert(cache_key(ICON_CACHE_CAPACITY), "new-icon".into());

        assert_eq!(cache.values.len(), ICON_CACHE_CAPACITY);
        assert!(cache.get(&cache_key(0)).is_some());
        assert!(cache.get(&cache_key(1)).is_none());
        assert!(cache.get(&cache_key(ICON_CACHE_CAPACITY)).is_some());
    }
}
