mod app_launcher;
mod bookmarks;

use serde_json::{json, Value};
use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind, PluginResponse,
    PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION, MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};

const PLUGIN_ID: &str = "quick_access";

sideweave_plugin_sdk::export_plugin_dylib!(QuickAccessPlugin);

pub struct QuickAccessPlugin {
    manifest: PluginManifest,
    apps: app_launcher::AppLauncherPlugin,
    bookmarks: bookmarks::BookmarksPlugin,
}

impl Default for QuickAccessPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl QuickAccessPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "快捷入口".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "统一管理本地应用与网页书签的快速访问入口".into(),
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
                    "system.open_external".into(),
                ],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Vue),
                    asset_root: "ui".into(),
                    initial_width: Some(860),
                    initial_height: Some(720),
                    ..Default::default()
                }),
                hover_preview: Some(sideweave_plugin_api::PluginHoverPreview {
                    width: Some(320),
                    height: Some(300),
                    data_method: Some("get_preview".into()),
                    entry: None,
                }),
                auto_start: false,
                minimum_host_version: None,
            },
            apps: app_launcher::AppLauncherPlugin::new(),
            bookmarks: bookmarks::BookmarksPlugin::new(),
        }
    }

    fn preview(&mut self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        let apps = call_child(&mut self.apps, request, context, "get_preview")?;
        let bookmarks = call_child(&mut self.bookmarks, request, context, "get_preview")?;
        let mut items = Vec::new();
        append_preview_items(&mut items, &apps, "apps");
        append_preview_items(&mut items, &bookmarks, "bookmarks");
        let total = items.len();
        Ok(json!({
            "title": "快捷入口",
            "subtitle": "本地应用与网页书签",
            "status": if total > 0 { "online" } else { "info" },
            "statusText": format!("{total} 个快捷入口"),
            "items": items,
            "actions": [{ "id": "open", "label": "管理快捷入口", "primary": false }]
        }))
    }
}

impl Plugin for QuickAccessPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if !matches!(request.kind, PluginRequestKind::Call) {
            return success_response(&request, Value::Null);
        }

        if request.method.as_deref() == Some("get_preview") {
            return match self.preview(&request, context) {
                Ok(value) => success_response(&request, value),
                Err(message) => error_response(&request, message),
            };
        }

        let Some((section, method)) = request
            .method
            .as_deref()
            .and_then(|method| method.split_once('.'))
        else {
            return error_response(&request, "方法必须包含模块前缀".into());
        };
        let method = method.to_string();
        let Some(plugin) = (match section {
            "apps" => Some(&mut self.apps as &mut dyn Plugin),
            "bookmarks" => Some(&mut self.bookmarks as &mut dyn Plugin),
            _ => None,
        }) else {
            return error_response(&request, format!("不支持的快捷入口模块: {section}"));
        };
        let mut routed = request;
        routed.method = Some(method);
        plugin.handle(routed, context)
    }
}

fn call_child(
    plugin: &mut dyn Plugin,
    request: &PluginRequest,
    context: &PluginContext,
    method: &str,
) -> Result<Value, String> {
    let mut routed = request.clone();
    routed.method = Some(method.to_string());
    let response = plugin.handle(routed, context);
    if response.ok {
        Ok(response.result)
    } else {
        Err(response.error.map(|error| error.message).unwrap_or_else(|| "插件模块调用失败".into()))
    }
}

fn append_preview_items(output: &mut Vec<Value>, preview: &Value, section: &str) {
    let Some(items) = preview.get("items").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let mut item = item.clone();
        let Some(object) = item.as_object_mut() else {
            continue;
        };
        if let Some(method) = object.get("method").and_then(Value::as_str).map(str::to_string) {
            object.insert("method".into(), Value::String(format!("{section}.{method}")));
        }
        output.push(item);
    }
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
