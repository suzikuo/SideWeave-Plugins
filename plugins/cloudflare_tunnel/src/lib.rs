mod checker;
mod config_manager;
mod file_share;
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
                    "network.listen".into(),
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
                    return Ok(json!({
                        "title": "Cloudflare Tunnel",
                        "status": "offline",
                        "statusText": "未配置",
                        "subtitle": "未检测到本地 Tunnel",
                        "metrics": [],
                        "actions": []
                    }));
                }
                let tunnel = config_manager::load_tunnel()?;
                let (state, conns, _err_msg) = process_manager::get_tunnel_status();
                let (status, status_text) = match state {
                    models::TunnelState::Running => ("online", format!("已连接 ({conns} 链路)")),
                    models::TunnelState::Starting => ("busy", "正在启动...".to_string()),
                    models::TunnelState::Stopped => ("offline", "已停止".to_string()),
                    models::TunnelState::Error => ("error", "运行异常".to_string()),
                };
                let is_active =
                    state == models::TunnelState::Running || state == models::TunnelState::Starting;
                let active_services = tunnel.services.iter().filter(|s| s.enabled).count();
                let total_services = tunnel.services.len();

                let action = if is_active {
                    json!({
                        "id": "stop",
                        "label": "停止",
                        "method": "stop",
                        "danger": true
                    })
                } else {
                    json!({
                        "id": "start",
                        "label": "启动",
                        "method": "start",
                        "primary": true
                    })
                };

                Ok(json!({
                    "title": "Cloudflare Tunnel",
                    "status": status,
                    "statusText": status_text,
                    "subtitle": tunnel.domain,
                    "metrics": [
                        { "label": "服务状态", "value": format!("{active_services}/{total_services} 启用") },
                        { "label": "Tunnel 名称", "value": tunnel.name }
                    ],
                    "actions": [action]
                }))
            }

            Some("get_tunnel") => {
                if !config_manager::has_tunnel() {
                    return Ok(Value::Null);
                }
                let tunnel = config_manager::load_tunnel()?;
                let (status, conns, err_msg) = process_manager::get_tunnel_status();

                let mut service_views = Vec::new();
                for s in &tunnel.services {
                    let port = config_manager::extract_port(&s.service);
                    let full_host = config_manager::full_hostname(&s.hostname, &tunnel.domain);
                    let is_file_share_route = s.enabled
                        && tunnel.file_share.as_ref().is_some_and(|share| {
                            share.enabled
                                && share.port == port
                                && !share.hostname.trim().is_empty()
                                && config_manager::full_hostname(&share.hostname, &tunnel.domain)
                                    .eq_ignore_ascii_case(&full_host)
                        });
                    let is_online = if is_file_share_route {
                        file_share::is_running()
                    } else {
                        checker::is_local_port_listening(port)
                    };
                    service_views.push(ServiceView {
                        id: s.id.clone(),
                        name: s.name.clone(),
                        hostname: s.hostname.clone(),
                        full_url: format!("https://{full_host}"),
                        service: s.service.clone(),
                        port,
                        local_online: is_online,
                        enabled: s.enabled,
                    });
                }

                let file_share_view = tunnel.file_share.as_ref().map(|share| {
                    let local_running = file_share::is_running();
                    let lan_running = file_share::lan_is_running();
                    let cloudflare_route_enabled = local_running
                        && file_share_route_is_enabled(&tunnel, &share.hostname, share.port);
                    let lan_host = file_share::lan_host()
                        .or_else(file_share::detect_lan_ipv4)
                        .unwrap_or_default();
                    FileShareView {
                        enabled: share.enabled,
                        directory: share.directory.clone(),
                        local_url: if local_running && !share.token.is_empty() {
                            format!("http://127.0.0.1:{}/{}/", share.port, share.token)
                        } else {
                            String::new()
                        },
                        local_running,
                        port: share.port,
                        lan_enabled: share.lan_enabled,
                        lan_running,
                        lan_host: lan_host.clone(),
                        lan_share_url: if share.lan_enabled
                            && lan_running
                            && !share.token.is_empty()
                            && !lan_host.is_empty()
                        {
                            format!("http://{lan_host}:{}/{}/", share.port, share.token)
                        } else {
                            String::new()
                        },
                        cloudflare_share_url: if cloudflare_route_enabled && !share.token.is_empty()
                        {
                            format!("https://{}/{}/", share.hostname, share.token)
                        } else {
                            String::new()
                        },
                        upload_enabled: share.upload_enabled,
                        upload_directory: share.upload_directory.clone(),
                        max_upload_mb: share.max_upload_mb,
                        max_total_upload_mb: share.max_total_upload_mb,
                        allowed_ips: share.allowed_ips.clone(),
                        access_logs: file_share::get_access_logs(100),
                    }
                });
                let mut tunnel_view = tunnel.clone();
                if let Some(share) = tunnel_view.file_share.as_mut() {
                    // The token is a bearer secret. The UI only receives the complete share URL.
                    share.token.clear();
                }
                let view = TunnelDetailView {
                    tunnel: tunnel_view,
                    status,
                    conns,
                    services: service_views,
                    file_share: file_share_view,
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
                    file_share: existing_tunnel.and_then(|tunnel| tunnel.file_share),
                };
                config_manager::save_tunnel(&record)?;

                Ok(Value::Null)
            }

            Some("pick_file_share_directory") => {
                let kind = request
                    .payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("share");
                let title = match kind {
                    "share" => "选择共享文件夹",
                    "upload" => "选择上传文件夹",
                    _ => return Err("未知的文件夹选择类型".into()),
                };
                Ok(json!(pick_file_share_directory(title)?))
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

            Some("save_file_share") => {
                let mut tunnel = config_manager::load_tunnel()?;
                let was_running = file_share::is_running();
                let previous_tunnel = tunnel.clone();
                let previous_share = tunnel.file_share.clone();
                let previous_config = fs::read(config_manager::config_yaml_path()).ok();
                let mut share = previous_share.clone().unwrap_or_default();
                share.enabled = was_running;
                if let Some(port) = request.payload.get("port").and_then(Value::as_u64) {
                    if !(1024..=65535).contains(&port) {
                        return Err("本机文件服务端口必须在 1024 到 65535 之间".into());
                    }
                    share.port = port as u16;
                }
                share.lan_enabled = request
                    .payload
                    .get("lan_enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(share.lan_enabled);
                share.directory = request
                    .payload
                    .get("directory")
                    .and_then(Value::as_str)
                    .unwrap_or(&share.directory)
                    .trim()
                    .to_string();
                share.hostname = request
                    .payload
                    .get("hostname")
                    .and_then(Value::as_str)
                    .unwrap_or(&share.hostname)
                    .trim()
                    .trim_matches('.')
                    .to_ascii_lowercase();
                if !share.hostname.is_empty() {
                    normalize_file_share_hostname(&share.hostname, &tunnel.domain)?;
                }

                if let Some(value) = request.payload.get("allowed_ips").and_then(Value::as_str) {
                    share.allowed_ips = value
                        .split(|character| character == '\n' || character == ',')
                        .map(str::trim)
                        .filter(|entry| !entry.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                file_share::validate_allowed_ips(&share.allowed_ips)?;

                share.upload_enabled = request
                    .payload
                    .get("upload_enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(share.upload_enabled);
                share.upload_directory = request
                    .payload
                    .get("upload_directory")
                    .and_then(Value::as_str)
                    .unwrap_or(&share.upload_directory)
                    .trim()
                    .to_string();
                if let Some(max_upload_mb) =
                    request.payload.get("max_upload_mb").and_then(Value::as_u64)
                {
                    if !(1..=1024).contains(&max_upload_mb) {
                        return Err("单个上传文件大小限制必须在 1 到 1024 MB 之间".into());
                    }
                    share.max_upload_mb = max_upload_mb as u32;
                }
                if let Some(max_total_upload_mb) = request
                    .payload
                    .get("max_total_upload_mb")
                    .and_then(Value::as_u64)
                {
                    if !(1..=8192).contains(&max_total_upload_mb) {
                        return Err("上传目录总量上限必须在 1 到 8192 MB 之间".into());
                    }
                    share.max_total_upload_mb = max_total_upload_mb as u32;
                }

                let rotate_token = request
                    .payload
                    .get("rotate_token")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if rotate_token
                    || share.token.len() != 32
                    || !share.token.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    share.token = uuid::Uuid::new_v4().simple().to_string();
                }
                if was_running {
                    if share.directory.is_empty() {
                        return Err("请先填写要共享的文件夹路径".into());
                    }
                    let root = fs::canonicalize(&share.directory)
                        .map_err(|error| format!("共享目录无法访问：{error}"))?;
                    if !root.is_dir() {
                        return Err("共享路径不是一个目录".into());
                    }
                    share.directory = root.to_string_lossy().into_owned();
                }
                if let Err(error) = file_share::start(&mut share) {
                    if was_running {
                        if let Some(mut previous) = previous_share.clone() {
                            let _ = file_share::start(&mut previous);
                        }
                    }
                    return Err(error);
                }
                tunnel.file_share = Some(share);
                let save_result = (|| {
                    if was_running {
                        if let Some(previous) = previous_share.as_ref() {
                            if !same_tunnel_hostname(
                                &previous.hostname,
                                &tunnel.file_share.as_ref().unwrap().hostname,
                                &tunnel.domain,
                            ) {
                                set_file_share_route(
                                    &mut tunnel,
                                    &previous.hostname,
                                    previous.port,
                                    false,
                                )?;
                            }
                        }
                        let (hostname, port) = tunnel
                            .file_share
                            .as_ref()
                            .map(|share| (share.hostname.clone(), share.port))
                            .ok_or_else(|| "文件服务配置不存在".to_string())?;
                        if !hostname.is_empty() {
                            set_file_share_route(&mut tunnel, &hostname, port, true)?;
                        }
                        config_manager::save_tunnel(&tunnel)
                    } else {
                        config_manager::save_tunnel_record_only(&tunnel)
                    }
                })();
                if let Err(error) = save_result {
                    if was_running {
                        if let Some(mut previous) = previous_share {
                            let _ = file_share::start(&mut previous);
                        } else {
                            file_share::stop();
                        }
                    }
                    let _ = config_manager::save_tunnel(&previous_tunnel);
                    return Err(error);
                }
                if was_running
                    && (tunnel.services != previous_tunnel.services
                        || active_tunnel_config_changed(previous_config))
                    && process_manager::is_cloudflared_running_globally()
                {
                    process_manager::restart_tunnel()?;
                }
                Ok(Value::Null)
            }

            Some("start_file_share") => {
                start_file_share()?;
                Ok(Value::Null)
            }

            Some("stop_file_share") => {
                stop_file_share()?;
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
                file_share::stop();
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
        if matches!(request.kind, PluginRequestKind::Activate) && config_manager::has_tunnel() {
            let result = config_manager::load_tunnel().and_then(|mut tunnel| {
                file_share::stop();
                let previous_share_state = tunnel.file_share.as_mut().and_then(|share| {
                    if share.enabled {
                        share.enabled = false;
                        Some((share.hostname.clone(), share.port))
                    } else {
                        None
                    }
                });
                if let Some((hostname, port)) = previous_share_state {
                    set_file_share_route(&mut tunnel, &hostname, port, false)?;
                    config_manager::save_tunnel(&tunnel)?;
                }
                Ok(())
            });
            if let Err(message) = result {
                return PluginResponse {
                    request_id: request.request_id,
                    plugin_id: request.plugin_id,
                    ok: false,
                    result: Value::Null,
                    error: Some(PluginError {
                        code: ErrorCode::Internal,
                        message,
                    }),
                };
            }
            return success_response(&request, Value::Null);
        }

        if matches!(
            request.kind,
            PluginRequestKind::Deactivate | PluginRequestKind::Shutdown
        ) {
            let _ = process_manager::stop_tunnel();
            file_share::stop();
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

fn pick_file_share_directory(title: &str) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let title = title.replace('\'', "''");
        let script = format!(
            r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = '{title}'
$dialog.ShowNewFolderButton = $true
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    Write-Output $dialog.SelectedPath
}}
"#
        );
        let output = Command::new("powershell")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(script)
            .output()
            .map_err(|error| format!("启动文件夹选择器失败：{error}"))?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if message.is_empty() {
                "文件夹选择器启动失败".into()
            } else {
                message
            });
        }
        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((!selected.is_empty()).then_some(selected))
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        Err("当前系统暂不支持文件夹选择器".into())
    }
}

fn normalize_file_share_hostname(
    hostname: &str,
    tunnel_domain: &str,
) -> Result<(String, String), String> {
    let hostname = hostname.trim().trim_matches('.').to_ascii_lowercase();
    let tunnel_domain = tunnel_domain.trim().trim_matches('.').to_ascii_lowercase();
    if hostname.is_empty() || tunnel_domain.is_empty() {
        return Err("请填写当前 Tunnel 根域名下的完整转发域名".into());
    }

    let suffix = format!(".{tunnel_domain}");
    let subdomain = hostname
        .strip_suffix(&suffix)
        .ok_or_else(|| format!("转发域名必须位于 Tunnel 根域名 {tunnel_domain} 之下"))?
        .to_string();
    if hostname.len() > 253
        || hostname.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err("转发域名格式无效，请填写完整的 DNS 域名".into());
    }

    Ok((hostname, subdomain))
}

fn same_tunnel_hostname(first: &str, second: &str, tunnel_domain: &str) -> bool {
    match (
        normalize_file_share_hostname(first, tunnel_domain),
        normalize_file_share_hostname(second, tunnel_domain),
    ) {
        (Ok((first, _)), Ok((second, _))) => first == second,
        _ => first.trim().eq_ignore_ascii_case(second.trim()),
    }
}

fn file_share_route_is_enabled(tunnel: &TunnelRecord, hostname: &str, port: u16) -> bool {
    let Ok((full_hostname, _)) = normalize_file_share_hostname(hostname, &tunnel.domain) else {
        return false;
    };
    tunnel.services.iter().any(|service| {
        service.enabled
            && config_manager::full_hostname(&service.hostname, &tunnel.domain)
                .trim_matches('.')
                .eq_ignore_ascii_case(&full_hostname)
            && config_manager::extract_port(&service.service) == port
    })
}

fn set_file_share_route(
    tunnel: &mut TunnelRecord,
    hostname: &str,
    port: u16,
    enabled: bool,
) -> Result<(), String> {
    if !enabled && hostname.trim().is_empty() {
        return Ok(());
    }
    let (full_hostname, subdomain) = normalize_file_share_hostname(hostname, &tunnel.domain)?;
    let matching_indices: Vec<usize> = tunnel
        .services
        .iter()
        .enumerate()
        .filter_map(|(index, service)| {
            let candidate = service
                .hostname
                .trim()
                .trim_matches('.')
                .to_ascii_lowercase();
            let domain = tunnel.domain.trim().trim_matches('.').to_ascii_lowercase();
            let suffix = format!(".{domain}");
            let candidate_full = if candidate.ends_with(&suffix) {
                candidate
            } else {
                format!("{candidate}.{domain}")
            };
            (candidate_full == full_hostname).then_some(index)
        })
        .collect();

    if !enabled {
        for index in matching_indices {
            tunnel.services[index].enabled = false;
        }
        return Ok(());
    }

    let target = format!("http://127.0.0.1:{port}");
    if let Some(&primary_index) = matching_indices.first() {
        for index in matching_indices {
            let service = &mut tunnel.services[index];
            if index == primary_index {
                service.hostname = subdomain.clone();
                service.service = target.clone();
                service.enabled = true;
                if service.name.trim().is_empty() {
                    service.name = "文件服务".into();
                }
            } else {
                service.enabled = false;
            }
        }
    } else {
        tunnel.services.push(ServiceRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: "文件服务".into(),
            hostname: subdomain,
            service: target,
            enabled: true,
        });
    }
    Ok(())
}

fn start_file_share() -> Result<(), String> {
    let mut tunnel = config_manager::load_tunnel()?;
    let previous_tunnel = tunnel.clone();
    let previous_config = fs::read(config_manager::config_yaml_path()).ok();
    let previous_share = tunnel.file_share.clone();
    let mut share = previous_share.clone().unwrap_or_default();
    share.enabled = true;
    if !share.hostname.trim().is_empty() {
        normalize_file_share_hostname(&share.hostname, &tunnel.domain)?;
    }

    if let Err(error) = file_share::start(&mut share) {
        if let Some(mut previous) = previous_share {
            if previous.enabled {
                let _ = file_share::start(&mut previous);
            }
        }
        return Err(error);
    }
    let hostname = share.hostname.clone();
    let port = share.port;
    tunnel.file_share = Some(share);
    if !hostname.is_empty() {
        if let Err(error) = set_file_share_route(&mut tunnel, &hostname, port, true) {
            if let Some(mut previous) = previous_share {
                if previous.enabled {
                    let _ = file_share::start(&mut previous);
                } else {
                    file_share::stop();
                }
            } else {
                file_share::stop();
            }
            return Err(error);
        }
    }
    if let Err(error) = config_manager::save_tunnel(&tunnel) {
        if let Some(mut previous) = previous_share {
            if previous.enabled {
                let _ = file_share::start(&mut previous);
            } else {
                file_share::stop();
            }
        } else {
            file_share::stop();
        }
        let _ = config_manager::save_tunnel(&previous_tunnel);
        return Err(error);
    }
    if (tunnel.services != previous_tunnel.services
        || active_tunnel_config_changed(previous_config))
        && process_manager::is_cloudflared_running_globally()
    {
        process_manager::restart_tunnel()?;
    }
    Ok(())
}

fn stop_file_share() -> Result<(), String> {
    let mut tunnel = config_manager::load_tunnel()?;
    let previous_services = tunnel.services.clone();
    let previous_config = fs::read(config_manager::config_yaml_path()).ok();
    file_share::stop();
    let route = if let Some(share) = tunnel.file_share.as_mut() {
        share.enabled = false;
        Some((share.hostname.clone(), share.port))
    } else {
        None
    };
    if let Some((hostname, port)) = route {
        if let Err(error) = set_file_share_route(&mut tunnel, &hostname, port, false) {
            let _ = config_manager::save_tunnel_record_only(&tunnel);
            return Err(error);
        }
        config_manager::save_tunnel(&tunnel)?;
    }
    if (tunnel.services != previous_services || active_tunnel_config_changed(previous_config))
        && process_manager::is_cloudflared_running_globally()
    {
        process_manager::restart_tunnel()?;
    }
    Ok(())
}

fn active_tunnel_config_changed(previous_config: Option<Vec<u8>>) -> bool {
    fs::read(config_manager::config_yaml_path()).ok() != previous_config
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
