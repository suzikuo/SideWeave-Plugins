use sideweave_plugin_api::{
    PluginManifest, PluginRequest, PluginRequestKind, PluginResponse, PluginRuntime,
    PluginTrust, PluginUi, PluginUiKind, API_VERSION, MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const PLUGIN_ID: &str = "bookmarks";
const STATE_KEY: &str = "state";

sideweave_plugin_sdk::export_plugin_dylib!(BookmarksPlugin);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Bookmark {
    pub id: String,
    pub name: String,
    pub url: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub in_preview: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BookmarkInput {
    pub name: String,
    pub url: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub in_preview: bool,
}

pub struct BookmarksPlugin {
    manifest: PluginManifest,
}

impl Default for BookmarksPlugin {
    fn default() -> Self { Self::new() }
}

impl BookmarksPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "书签".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "书签：标签化收藏、极速检索、一键直达".into(),
                author: "SideWeave".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec!["storage".into(), "system.open_external".into()],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Html),
                    asset_root: "ui".into(),
                    initial_width: Some(580),
                    initial_height: Some(760),
                    ..Default::default()
                }),
                hover_preview: Some(sideweave_plugin_api::PluginHoverPreview {
                    width: Some(280),
                    height: Some(260),
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
            Some("list") => Ok(json!(load(context))),
            Some("add") => {
                let input: BookmarkInput = parse_payload(request, "input")?;
                validate_input(&input)?;
                let bookmark = Bookmark {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: input.name.trim().to_string(),
                    url: input.url.trim().to_string(),
                    tags: clean_tags(&input.tags),
                    pinned: input.pinned,
                    in_preview: input.in_preview,
                    created_at: now_secs(),
                };
                let mut all = load(context);
                all.push(bookmark.clone());
                save(context, &all)?;
                Ok(json!(bookmark))
            }
            Some("update") => {
                let id = string_field(&request.payload, "id")?;
                let input: BookmarkInput = parse_payload(request, "input")?;
                validate_input(&input)?;
                let mut all = load(context);
                let target = all
                    .iter_mut()
                    .find(|bookmark| bookmark.id == id)
                    .ok_or_else(|| format!("书签不存在：{id}"))?;
                target.name = input.name.trim().to_string();
                target.url = input.url.trim().to_string();
                target.tags = clean_tags(&input.tags);
                target.pinned = input.pinned;
                target.in_preview = input.in_preview;
                let updated = target.clone();
                save(context, &all)?;
                Ok(json!(updated))
            }
            Some("remove") => {
                let id = string_field(&request.payload, "id")?;
                let mut all = load(context);
                let before = all.len();
                all.retain(|bookmark| bookmark.id != id);
                if all.len() == before {
                    return Err(format!("书签不存在：{id}"));
                }
                save(context, &all)?;
                Ok(Value::Null)
            }
            Some("open") | Some("launch") => {
                let id = string_field(&request.payload, "id")?;
                let bookmark = load(context)
                    .into_iter()
                    .find(|bookmark| bookmark.id == id)
                    .ok_or_else(|| format!("书签不存在：{id}"))?;
                validate_url(&bookmark.url)?;
                context
                    .system()
                    .open_external(bookmark.url.trim())
                    .map_err(|error| error.message)?;
                Ok(Value::Null)
            }
            Some("toggle_preview") => {
                let id = string_field(&request.payload, "id")?;
                let mut all = load(context);
                let target = all
                    .iter_mut()
                    .find(|bookmark| bookmark.id == id)
                    .ok_or_else(|| format!("书签不存在：{id}"))?;
                target.in_preview = !target.in_preview;
                let in_preview = target.in_preview;
                save(context, &all)?;
                Ok(json!({ "id": id, "in_preview": in_preview }))
            }
            Some("get_preview") => {
                let all = load(context);
                let preview_bookmarks: Vec<_> = all.iter().filter(|b| b.in_preview).collect();
                let total_count = all.len();
                let selected_count = preview_bookmarks.len();

                let items: Vec<Value> = preview_bookmarks
                    .into_iter()
                    .map(|bm| {
                        json!({
                            "id": bm.id,
                            "title": bm.name,
                            "tooltip": bm.url,
                            "icon": bookmark_avatar_svg(&bm.name),
                            "method": "launch",
                            "payload": { "id": bm.id }
                        })
                    })
                    .collect();

                Ok(json!({
                    "title": "书签",
                    "subtitle": format!("已选 {selected_count}/{total_count} 个书签"),
                    "status": if selected_count > 0 { "online" } else { "info" },
                    "statusText": if selected_count > 0 {
                        format!("{selected_count} 个快捷书签")
                    } else {
                        "未设置预览书签".to_string()
                    },
                    "items": items,
                    "actions": [
                        { "id": "open", "label": "管理书签", "primary": false }
                    ]
                }))
            }
            Some("get_view") => {
                let state = load_state(context);
                Ok(json!(state.view))
            }
            Some("set_view") => {
                let view = string_field(&request.payload, "view")?;
                let view = match view.as_str() {
                    "compact" | "table" => view,
                    _ => "card".into(),
                };
                let mut state = load_state(context);
                state.view = view;
                save_state(context, &state)?;
                Ok(Value::Null)
            }
            Some(method) => Err(format!("不支持的书签方法: {method}")),
            None => Err("缺少插件方法".into()),
        }
    }
}

impl Plugin for BookmarksPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
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
                error: Some(sideweave_plugin_api::PluginError {
                    code: sideweave_plugin_api::ErrorCode::Internal,
                    message,
                }),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct BookmarkState {
    bookmarks: Vec<Bookmark>,
    view: String,
}

fn load_state(context: &PluginContext) -> BookmarkState {
    context
        .storage()
        .get(context.plugin_id(), STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .map(|mut state: BookmarkState| {
            if state.view.is_empty() {
                state.view = "card".into();
            }
            state
        })
        .unwrap_or_else(|| BookmarkState {
            bookmarks: Vec::new(),
            view: "card".into(),
        })
}

fn save_state(context: &PluginContext, state: &BookmarkState) -> Result<(), String> {
    let value = serde_json::to_string(state).map_err(|error| error.to_string())?;
    context
        .storage()
        .set(context.plugin_id(), STATE_KEY, &value)
        .map_err(|error| error.message)
}

fn load(context: &PluginContext) -> Vec<Bookmark> {
    load_state(context).bookmarks
}

fn save(context: &PluginContext, bookmarks: &[Bookmark]) -> Result<(), String> {
    let mut state = load_state(context);
    state.bookmarks = bookmarks.to_vec();
    save_state(context, &state)
}

fn parse_payload<T: for<'de> Deserialize<'de>>(
    request: &PluginRequest,
    field: &str,
) -> Result<T, String> {
    request
        .payload
        .get(field)
        .cloned()
        .ok_or_else(|| format!("缺少参数: {field}"))
        .and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
}

fn string_field(payload: &Value, field: &str) -> Result<String, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("参数 {field} 必须是字符串"))
}

fn validate_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("网址必须以 http:// 或 https:// 开头".into());
    }
    if url.contains('"') {
        return Err("网址不能包含引号字符".into());
    }
    Ok(())
}

fn validate_input(bookmark: &BookmarkInput) -> Result<(), String> {
    if bookmark.name.trim().is_empty() {
        return Err("名称不能为空".into());
    }
    validate_url(bookmark.url.trim())
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut output = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if !tag.is_empty() && !output.iter().any(|item| item == tag) {
            output.push(tag.to_string());
        }
    }
    output
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[allow(dead_code)]
fn extract_domain(url: &str) -> String {
    let without_proto = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let domain = without_proto.split('/').next().unwrap_or(without_proto);
    domain.to_string()
}

fn bookmark_avatar_svg(name: &str) -> String {
    let char = name.chars().next().unwrap_or('?').to_uppercase();
    let mut h: i32 = 0;
    for b in name.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*b as i32);
    }
    const COLORS: [&str; 8] = [
        "%231f5f6b", "%2341568b", "%235a4a86", "%237a4f74",
        "%2381563f", "%236e6a35", "%233f6e52", "%235d5d64",
    ];
    let color = COLORS[(h.abs() as usize) % COLORS.len()];
    format!(
        "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24'%3E%3Crect width='24' height='24' rx='6' fill='{}'/%3E%3Ctext x='50%25' y='55%25' dominant-baseline='middle' text-anchor='middle' fill='%23ffffff' font-size='12' font-family='sans-serif' font-weight='600'%3E{}%3C/text%3E%3C/svg%3E",
        color, char
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_hover_preview() {
        let plugin = BookmarksPlugin::new();
        let hover = plugin.manifest().hover_preview.as_ref().expect("hover preview should be configured");
        assert_eq!(hover.data_method.as_deref(), Some("get_preview"));
        assert_eq!(hover.width, Some(280));
        assert_eq!(hover.height, Some(260));
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(extract_domain("https://github.com/suzikuo/sidebar-tauri"), "github.com");
        assert_eq!(extract_domain("http://localhost:5173/"), "localhost:5173");
        assert_eq!(extract_domain("https://sub.example.org/path/to/page?q=1"), "sub.example.org");
    }

    #[test]
    fn test_bookmark_avatar_svg() {
        let svg = bookmark_avatar_svg("GitHub");
        assert!(svg.starts_with("data:image/svg+xml,"));
        assert!(svg.contains("%3EG%3C"));
    }

    #[test]
    fn test_bookmark_serialization_default_in_preview() {
        let json_data = r#"{"id":"1","name":"Test","url":"https://example.com","tags":[],"pinned":false,"created_at":0}"#;
        let bookmark: Bookmark = serde_json::from_str(json_data).expect("should deserialize");
        assert!(!bookmark.in_preview);
    }
}
