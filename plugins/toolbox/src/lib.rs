use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind,
    PluginResponse, PluginRuntime, PluginTrust, API_VERSION, MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};
use serde_json::{json, Value};
use std::path::Path;

sideweave_plugin_sdk::export_plugin_dylib!(ToolboxPlugin);

pub struct ToolboxPlugin {
    manifest: PluginManifest,
}

impl Default for ToolboxPlugin {
    fn default() -> Self { Self::new() }
}

impl ToolboxPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new("toolbox").expect("static plugin id"),
                name: "工具箱".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "开发小工具：JSON/Base64/时间戳/UUID/正则".into(),
                author: "SideWeave".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec!["filesystem.read".into(), "system.file_picker".into()],
                ui: Some(sideweave_plugin_api::PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(sideweave_plugin_api::PluginUiKind::Html),
                    asset_root: "ui".into(),
                    initial_width: Some(760),
                    initial_height: Some(620),
                    ..Default::default()
                }),
                hover_preview: None,
                auto_start: false,
                minimum_host_version: None,
            },
        }
    }
}

impl Plugin for ToolboxPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if !matches!(&request.kind, PluginRequestKind::Call) {
            return success_response(&request, Value::Null);
        }
        let result = match request.method.as_deref() {
            Some("decode_qr") => decode_qr(context),
            Some(method) => Err(format!("不支持的工具箱方法: {method}")),
            None => Err("缺少插件方法".into()),
        };
        match result {
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

fn decode_qr(context: &PluginContext) -> Result<Value, String> {
    let extensions = ["png", "jpg", "jpeg", "webp", "bmp", "gif"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let Some(path) = context
        .system()
        .pick_file(&extensions)
        .map_err(|error| error.message)?
    else {
        return Ok(json!([]));
    };
    Ok(json!(decode_qr_file(Path::new(&path))?))
}

fn decode_qr_file(path: &Path) -> Result<Vec<String>, String> {
    let image = image::open(path).map_err(|error| format!("图片读取失败: {error}"))?;
    let mut prepared = rqrr::PreparedImage::prepare(image.to_luma8());
    let grids = prepared.detect_grids();
    if grids.is_empty() {
        return Err("未在图片中识别到二维码".into());
    }
    grids
        .into_iter()
        .map(|grid| {
            grid.decode()
                .map(|(_, content)| content)
                .map_err(|error| format!("二维码解码失败: {error}"))
        })
        .collect()
}
