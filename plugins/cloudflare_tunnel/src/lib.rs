mod checker;
mod config_manager;
mod models;
mod process_manager;

use chrono::Local;
use models::*;
use serde::Serialize;
use serde_json::{json, Value};
use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind, PluginResponse,
    PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION, MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};
use std::fs;
use std::path::PathBuf;

const PLUGIN_ID: &str = "cloudflare_tunnel";

sideweave_plugin_sdk::export_plugin_dylib!(CloudflareTunnelPlugin);

pub struct CloudflareTunnelPlugin {
    manifest: PluginManifest,
}

impl Default for CloudflareTunnelPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudflareTunnelPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "Cloudflare Tunnel".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "SideWeave 本地 Cloudflare Tunnel 快速管理器".into(),
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
                    "process.spawn".into(),
                    "system.open_external".into(),
                    "network.connect".into(),
                    "clipboard".into(),
                ],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Vue),
                    asset_root: "ui".into(),
                    initial_width: Some(840),
                    initial_height: Some(780),
                    ..Default::default()
                }),
                hover_preview: Some(sideweave_plugin_api::PluginHoverPreview {
                    width: Some(280),
                    height: Some(180),
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
            Some("get_preview") => {
                if !config_manager::has_tunnel() {
                    return Ok(
                        json!({"title":"Cloudflare Tunnel","status":"offline","statusText":"未配置","subtitle":"未检测到本地 Tunnel","metrics":[],"actions":[]}),
                    );
                }
                let tunnel = config_manager::load_tunnel()?;
                let (state, conns, _) = process_manager::get_tunnel_status();
                let (status, status_text) = match state {
                    models::TunnelState::Running => ("online", format!("已连接 ({conns} 链路)")),
                    models::TunnelState::Starting => ("busy", "正在启动…".to_string()),
                    models::TunnelState::Stopped => ("offline", "已停止".to_string()),
                    models::TunnelState::Error => ("error", "运行异常".to_string()),
                };
                let active = matches!(
                    state,
                    models::TunnelState::Running | models::TunnelState::Starting
                );
                let visible_services = tunnel.services.len();
                let active_services = tunnel.services.iter().filter(|service| service.enabled).count();
                let tunnel_action = if active {
                    json!({"id":"stop","label":"停止","method":"stop","danger":true})
                } else {
                    json!({"id":"start","label":"启动","method":"start","primary":true})
                };
                Ok(json!({
                    "title":"Cloudflare Tunnel", "status":status, "statusText":status_text,
                    "subtitle":tunnel.domain,
                    "metrics":[
                        {"label":"服务路由","value":format!("{}/{} 启用",active_services,visible_services)},
                        {"label":"Tunnel 名称","value":tunnel.name}
                    ],
                    "actions":[tunnel_action]
                }))
            }

            Some("get_tunnel") => {
                if !config_manager::has_tunnel() {
                    return Ok(Value::Null);
                }
                let tunnel = config_manager::load_tunnel()?;
                let (status, conns, err_msg) = process_manager::get_tunnel_status();
                let mut service_views = Vec::new();
                for service in &tunnel.services {
                    let port = config_manager::extract_port(&service.service);
                    let full_host =
                        config_manager::full_hostname(&service.hostname, &tunnel.domain);
                    let online = checker::is_local_port_listening(port);
                    service_views.push(ServiceView {
                        id: service.id.clone(),
                        name: service.name.clone(),
                        hostname: service.hostname.clone(),
                        full_url: format!("https://{full_host}"),
                        service: service.service.clone(),
                        port,
                        local_online: online,
                        enabled: service.enabled,
                        disable_chunked_encoding: service.disable_chunked_encoding,
                    });
                }
                let view = TunnelDetailView {
                    tunnel,
                    status,
                    conns,
                    services: service_views,
                    error_message: err_msg,
                    config_path: config_manager::config_yaml_path()
                        .to_string_lossy()
                        .into_owned(),
                    credentials_path: config_manager::credentials_path()
                        .to_string_lossy()
                        .into_owned(),
                };
                serde_json::to_value(view).map_err(|e| e.to_string())
            }

            Some("setup_tunnel") => {
                let name = string_field(&request.payload, "name")?.trim().to_string();
                let domain = string_field(&request.payload, "domain")?.trim().to_string();
                let tunnel_id = string_field(&request.payload, "tunnel_id")?
                    .trim()
                    .to_string();

                if name.is_empty() || domain.is_empty() || tunnel_id.is_empty() {
                    return Err("Tunnel 名称、根域名和 Tunnel ID 均不能为空".to_string());
                }

                let cred_content = if let Some(content) = request
                    .payload
                    .get("credentials_content")
                    .and_then(Value::as_str)
                {
                    content.to_string()
                } else if let Some(path) = request
                    .payload
                    .get("credentials_path")
                    .and_then(Value::as_str)
                {
                    fs::read_to_string(path).map_err(|e| format!("读取凭证文件失败: {e}"))?
                } else {
                    let auto_path = find_cloudflared_credential(&tunnel_id)?;
                    fs::read_to_string(auto_path).map_err(|e| format!("读取凭证失败: {e}"))?
                };

                serde_json::from_str::<Value>(&cred_content)
                    .map_err(|e| format!("凭证文件不是合法的 JSON: {e}"))?;

                let dir = config_manager::plugin_data_dir();
                fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

                let dest_cred = config_manager::credentials_path();
                fs::write(&dest_cred, cred_content)
                    .map_err(|e| format!("写入凭证文件失败: {e}"))?;

                let existing_tunnel = config_manager::load_tunnel().ok();
                let existing_services = existing_tunnel
                    .as_ref()
                    .map(|tunnel| tunnel.services.clone())
                    .unwrap_or_default();

                let record = TunnelRecord {
                    id: "local".to_string(),
                    name,
                    tunnel_id,
                    domain,
                    created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                    services: existing_services,
                };
                config_manager::save_tunnel(&record)?;

                Ok(Value::Null)
            }

            Some("scan_credentials") => {
                let list = scan_user_cloudflared_credentials();
                Ok(serde_json::to_value(list).unwrap_or(Value::Array(Vec::new())))
            }

            Some("save_services") => {
                let services_val = request
                    .payload
                    .get("services")
                    .ok_or_else(|| "缺少 services 字段".to_string())?;
                let raw_services: Vec<ServiceRule> =
                    serde_json::from_value(services_val.clone()).map_err(|e| e.to_string())?;

                let mut sanitized = Vec::new();
                for s in raw_services {
                    let (norm_service, _) = config_manager::normalize_service_address(&s.service)?;
                    let sub = s.hostname.trim().trim_matches('.').to_string();
                    if sub.is_empty() {
                        return Err("服务 Hostname 不能为空".to_string());
                    }
                    sanitized.push(ServiceRule {
                        id: if s.id.trim().is_empty() {
                            uuid::Uuid::new_v4().to_string()
                        } else {
                            s.id
                        },
                        name: if s.name.trim().is_empty() {
                            sub.clone()
                        } else {
                            s.name.trim().to_string()
                        },
                        hostname: sub,
                        service: norm_service,
                        enabled: s.enabled,
                        disable_chunked_encoding: s.disable_chunked_encoding,
                    });
                }

                let mut tunnel = config_manager::load_tunnel()?;
                tunnel.services = sanitized;
                config_manager::save_tunnel(&tunnel)?;

                if process_manager::is_cloudflared_running_globally() {
                    let _ = process_manager::restart_tunnel();
                }

                Ok(Value::Null)
            }

            Some("toggle_service") => {
                let service_id = string_field(&request.payload, "service_id")?;
                let enabled = request
                    .payload
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);

                let mut tunnel = config_manager::load_tunnel()?;
                if let Some(s) = tunnel.services.iter_mut().find(|s| s.id == service_id) {
                    s.enabled = enabled;
                }
                config_manager::save_tunnel(&tunnel)?;

                if process_manager::is_cloudflared_running_globally() {
                    let _ = process_manager::restart_tunnel();
                }

                Ok(Value::Null)
            }

            Some("start") => {
                process_manager::start_tunnel()?;
                Ok(Value::Null)
            }

            Some("stop") => {
                process_manager::stop_tunnel()?;
                Ok(Value::Null)
            }

            Some("restart") => {
                process_manager::restart_tunnel()?;
                Ok(Value::Null)
            }

            Some("get_logs") => {
                let limit = request
                    .payload
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize);
                let logs = process_manager::get_tunnel_logs(limit);
                Ok(serde_json::to_value(logs).unwrap_or(Value::Array(Vec::new())))
            }

            Some("clear_logs") => {
                process_manager::clear_tunnel_logs()?;
                Ok(Value::Null)
            }

            Some("reset_tunnel") => {
                let stop_result = process_manager::stop_tunnel();
                stop_result?;
                config_manager::delete_tunnel_data()?;
                Ok(Value::Null)
            }

            Some("open_url") => {
                let url = string_field(&request.payload, "url")?;
                context
                    .system()
                    .open_external(&url)
                    .map_err(|e| e.message)?;
                Ok(Value::Null)
            }

            Some("copy_text") => {
                let text = string_field(&request.payload, "text")?;
                context
                    .clipboard()
                    .write_text(&text)
                    .map_err(|e| e.message)?;
                Ok(Value::Null)
            }

            Some("check_port") => {
                let port = request
                    .payload
                    .get("port")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u16;
                let online = checker::is_local_port_listening(port);
                Ok(json!({ "port": port, "online": online }))
            }

            Some(method) => Err(format!("未知的方法: {method}")),
            None => Err("缺少方法名称".to_string()),
        }
    }
}

impl Plugin for CloudflareTunnelPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if matches!(
            request.kind,
            PluginRequestKind::Deactivate | PluginRequestKind::Shutdown
        ) {
            let _ = process_manager::stop_tunnel();
            return success_response(&request, Value::Null);
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

fn string_field(payload: &Value, key: &str) -> Result<String, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("缺少字段: {key}"))
}

fn find_cloudflared_credential(tunnel_id: &str) -> Result<PathBuf, String> {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(profile)
            .join(".cloudflared")
            .join(format!("{tunnel_id}.json"));
        if p.exists() {
            return Ok(p);
        }
    }
    Err(format!(
        "未在 ~/.cloudflared/ 找到凭证文件 {tunnel_id}.json"
    ))
}

#[derive(Serialize)]
struct CredentialScanItem {
    tunnel_id: String,
    path: String,
}

fn scan_user_cloudflared_credentials() -> Vec<CredentialScanItem> {
    let mut items = Vec::new();
    let Some(profile) = std::env::var_os("USERPROFILE") else {
        return items;
    };
    let cf_dir = PathBuf::from(profile).join(".cloudflared");
    let Ok(entries) = fs::read_dir(cf_dir) else {
        return items;
    };

    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().map(|e| e == "json").unwrap_or(false) {
            let stem = p.file_stem().unwrap_or_default().to_string_lossy();
            if stem.len() == 36 && uuid::Uuid::parse_str(&stem).is_ok() {
                items.push(CredentialScanItem {
                    tunnel_id: stem.to_string(),
                    path: p.to_string_lossy().into_owned(),
                });
            }
        }
    }
    items
}
