use agile_tiles_plugin_sdk::{
    PluginError, PluginManifest, PluginRequest, PluginResponse, PluginRuntime, PluginTrust,
    PluginUi, PluginUiKind, API_VERSION, MANIFEST_VERSION,
};
use agile_tiles_plugin_sdk::{success_response, Plugin, PluginContext};
use serde_json::json;

pub struct DemoPlugin {
    manifest: PluginManifest,
}

impl DemoPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: agile_tiles_plugin_sdk::PluginId::new("demo").expect("static plugin id"),
                name: "Demo Plugin".into(),
                icon: "ui/icon.svg".into(),
                version: "0.1.0".into(),
                description: "A local SideWeave plugin.".into(),
                author: "Your Name".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec![],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Html),
                    asset_root: "ui".into(),
                    initial_width: Some(580),
                    initial_height: Some(760),
                    ..Default::default()
                }),
                hover_preview: None,
                auto_start: false,
                minimum_host_version: None,
            },
        }
    }
}

agile_tiles_plugin_sdk::export_plugin_dylib!(DemoPlugin);

impl Default for DemoPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for DemoPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, _context: &PluginContext) -> PluginResponse {
        match request.method.as_deref() {
            Some("hello") => success_response(&request, json!({ "message": "Hello from plugin" })),
            Some(method) => PluginResponse {
                request_id: request.request_id,
                plugin_id: request.plugin_id,
                ok: false,
                result: serde_json::Value::Null,
                error: Some(PluginError {
                    code: agile_tiles_plugin_sdk::ErrorCode::NotFound,
                    message: format!("unknown method: {method}"),
                }),
            },
            None => PluginResponse {
                request_id: request.request_id,
                plugin_id: request.plugin_id,
                ok: false,
                result: serde_json::Value::Null,
                error: Some(PluginError {
                    code: agile_tiles_plugin_sdk::ErrorCode::InvalidRequest,
                    message: "missing method".into(),
                }),
            },
        }
    }
}
