mod cloudflare_tunnel;
mod port_forward;
mod ssh;

use serde_json::{json, Value};
use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind, PluginResponse,
    PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION, MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};

const PLUGIN_ID: &str = "dev_connection";

sideweave_plugin_sdk::export_plugin_dylib!(DevConnectionPlugin);

pub struct DevConnectionPlugin {
    manifest: PluginManifest,
    ssh: ssh::SshPlugin,
    ports: port_forward::PortForwardPlugin,
    tunnel: cloudflare_tunnel::CloudflareTunnelPlugin,
}

impl Default for DevConnectionPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl DevConnectionPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "开发连接台".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "集中管理 SSH 会话、本地端口转发与 Cloudflare Tunnel".into(),
                author: "SideWeave".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec![
                    "storage".into(),
                    "filesystem.read".into(),
                    "filesystem.write".into(),
                    "filesystem.external_read".into(),
                    "system.file_picker".into(),
                    "system.open_external".into(),
                    "process.spawn".into(),
                    "network.connect".into(),
                    "network.listen".into(),
                    "clipboard".into(),
                ],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Vue),
                    asset_root: "ui".into(),
                    initial_width: Some(900),
                    initial_height: Some(780),
                    ..Default::default()
                }),
                hover_preview: Some(sideweave_plugin_api::PluginHoverPreview {
                    width: Some(320),
                    height: Some(300),
                    data_method: Some("get_preview".into()),
                    entry: None,
                }),
                auto_start: true,
                minimum_host_version: None,
            },
            ssh: ssh::SshPlugin::new(),
            ports: port_forward::PortForwardPlugin::new(),
            tunnel: cloudflare_tunnel::CloudflareTunnelPlugin::new(),
        }
    }

    fn preview(&mut self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        let ssh_preview = call_child(&mut self.ssh, request, context, "get_preview").ok();
        let tunnel_preview = call_child(&mut self.tunnel, request, context, "get_preview").ok();
        let tunnel_preview_services = call_child(
            &mut self.tunnel,
            request,
            context,
            "get_preview_services",
        )
            .ok()
            .and_then(|value| value.as_array().cloned());
        let port_rules = call_child(&mut self.ports, request, context, "list")
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();

        let mut items = Vec::new();
        if let Some(preview) = ssh_preview.as_ref() {
            append_preview_items(&mut items, preview, "ssh");
        }

        for rule in &port_rules {
            if !rule.get("in_preview").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            let Some(id) = rule.get("id").and_then(Value::as_str) else {
                continue;
            };
            let name = rule.get("name").and_then(Value::as_str).unwrap_or("端口转发");
            let listen_port = rule.get("listen_port").and_then(Value::as_u64).unwrap_or(0);
            let target_host = rule.get("target_host").and_then(Value::as_str).unwrap_or("localhost");
            let target_port = rule.get("target_port").and_then(Value::as_u64).unwrap_or(0);
            let running = rule.get("running").and_then(Value::as_bool).unwrap_or(false);
            items.push(json!({
                "id": format!("ports:{id}"),
                "title": name,
                "subtitle": format!("0.0.0.0:{listen_port} → {target_host}:{target_port}"),
                "badge": if running { "运行中" } else { "已停止" },
                "method": "ports.set_running",
                "payload": { "id": id, "running": !running }
            }));
        }

        if let Some(services) = tunnel_preview_services.as_ref() {
            for service in services {
                if !service.get("in_preview").and_then(Value::as_bool).unwrap_or(false) {
                    continue;
                }
                let Some(id) = service.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let name = service.get("name").and_then(Value::as_str).unwrap_or("Tunnel 服务");
                let url = service.get("full_url").and_then(Value::as_str).unwrap_or_default();
                let enabled = service.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                items.push(json!({
                    "id": format!("tunnel:{id}"),
                    "title": name,
                    "subtitle": url,
                    "badge": if enabled { "\u{5df2}\u{542f}\u{7528}" } else { "\u{5df2}\u{6682}\u{505c}" },
                    "method": "tunnel.open_url",
                    "payload": { "url": url }
                }));
            }
        }

        let mut state_parts = Vec::new();
        let mut status = "info";
        if let Some(preview) = tunnel_preview.as_ref() {
            if let Some(value) = preview.get("status").and_then(Value::as_str) {
                status = value;
            }
            if let Some(value) = preview.get("stateText").and_then(Value::as_str) {
                state_parts.push(value.to_string());
            }
        }
        let state_text = (!state_parts.is_empty()).then(|| state_parts.join(" · "));

        let has_items = !items.is_empty();
        let mut result = json!({
            "title": "开发连接台",
            "subtitle": "SSH、端口转发与 Cloudflare Tunnel",
            "status": status,
            "items": items,
        });
        if let Some(state_text) = state_text {
            result["stateText"] = Value::String(state_text);
        }
        if !has_items && result.get("stateText").is_none() {
            return Ok(Value::Null);
        }
        Ok(result)
    }
}

impl Plugin for DevConnectionPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if !matches!(request.kind, PluginRequestKind::Call) {
            let _ = self.ssh.handle(request.clone(), context);
            let _ = self.ports.handle(request.clone(), context);
            let _ = self.tunnel.handle(request.clone(), context);
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
            "ssh" => Some(&mut self.ssh as &mut dyn Plugin),
            "ports" => Some(&mut self.ports as &mut dyn Plugin),
            "tunnel" => Some(&mut self.tunnel as &mut dyn Plugin),
            _ => None,
        }) else {
            return error_response(&request, format!("不支持的开发连接模块: {section}"));
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
        if let Some(id) = object.get("id").and_then(Value::as_str).map(str::to_string) {
            object.insert("id".into(), Value::String(format!("{section}:{id}")));
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
