mod config_manager;
mod file_share;
mod models;

use models::{FileShareConfig, FileShareView};
use serde_json::{json, Value};
use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind, PluginResponse,
    PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION, MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};

const PLUGIN_ID: &str = "file_share";

sideweave_plugin_sdk::export_plugin_dylib!(FileSharePlugin);

pub struct FileSharePlugin {
    manifest: PluginManifest,
}

impl Default for FileSharePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSharePlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "本地文件服务".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "独立管理本机与局域网文件浏览、上传、账号认证和媒体预览。".into(),
                author: "SideWeave".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec![
                    "storage".into(),
                    "filesystem.read".into(),
                    "filesystem.write".into(),
                    "system.file_picker".into(),
                    "system.open_external".into(),
                    "network.listen".into(),
                    "clipboard".into(),
                ],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Vue),
                    asset_root: "ui".into(),
                    initial_width: Some(900),
                    initial_height: Some(760),
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
                let shares = config_manager::load_shares()?;
                let running = shares.iter().filter(|share| file_share::is_running(&share.id)).count();
                Ok(json!({
                    "title": "本地文件服务",
                    "status": if running > 0 { "online" } else { "offline" },
                    "statusText": format!("{running}/{} 个实例运行", shares.len()),
                    "subtitle": "本机与局域网文件服务",
                    "metrics": [{ "label": "文件服务", "value": format!("{running}/{} 运行", shares.len()) }],
                    "actions": []
                }))
            }
            Some("get_file_shares") => {
                let shares = config_manager::load_shares()?;
                let views = shares
                    .iter()
                    .map(file_share_view)
                    .collect::<Vec<_>>();
                Ok(json!({
                    "file_shares": views,
                    "users": file_share::list_file_share_users().unwrap_or_default()
                }))
            }
            Some("pick_directory") => {
                let kind = request.payload.get("kind").and_then(Value::as_str).unwrap_or("view");
                let title = match kind {
                    "view" => "选择查看目录",
                    "receive" => "选择收件目录",
                    _ => return Err("未知的目录选择类型".into()),
                };
                Ok(json!(pick_directory(title)?))
            }
            Some("suggest_port") => Ok(json!(suggest_port()?)),
            Some("save_file_share") => save_file_share(&request.payload),
            Some("delete_file_share") => delete_file_share(&request.payload),
            Some("start_file_share") => start_file_share(&request.payload),
            Some("stop_file_share") => stop_file_share(&request.payload),
            Some("rotate_file_share_token") => rotate_file_share_token(&request.payload),
            Some("clear_access_logs") => {
                let id = string_field(&request.payload, "id")?;
                if !config_manager::load_shares()?.iter().any(|share| share.id == id) {
                    return Err("Instance not found".into());
                }
                file_share::clear_access_logs(&id);
                Ok(Value::Null)
            }
            Some("add_user") => {
                let username = string_field(&request.payload, "username")?;
                let password = string_field(&request.payload, "password")?;
                file_share::add_file_share_user(&username, &password)?;
                Ok(Value::Null)
            }
            Some("set_user_password") => {
                let username = string_field(&request.payload, "username")?;
                let password = string_field(&request.payload, "password")?;
                file_share::set_file_share_user_password(&username, &password)?;
                Ok(Value::Null)
            }
            Some("delete_user") => {
                let username = string_field(&request.payload, "username")?;
                file_share::delete_file_share_user(&username)?;
                Ok(Value::Null)
            }
            Some("open_url") => {
                let url = string_field(&request.payload, "url")?;
                context.system().open_external(&url).map_err(|error| error.message)?;
                Ok(Value::Null)
            }
            Some("copy_text") => {
                let text = string_field(&request.payload, "text")?;
                context.clipboard().write_text(&text).map_err(|error| error.message)?;
                Ok(Value::Null)
            }
            Some(method) => Err(format!("未知的方法: {method}")),
            None => Err("缺少方法名称".into()),
        }
    }
}

impl Plugin for FileSharePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if matches!(request.kind, PluginRequestKind::Activate) {
            let result = stop_all_and_disable();
            return response(&request, result);
        }
        if matches!(request.kind, PluginRequestKind::Deactivate | PluginRequestKind::Shutdown) {
            let _ = stop_all_and_disable();
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
                error: Some(PluginError { code: ErrorCode::Internal, message }),
            },
        }
    }
}

fn response(request: &PluginRequest, result: Result<(), String>) -> PluginResponse {
    match result {
        Ok(()) => success_response(request, Value::Null),
        Err(message) => PluginResponse {
            request_id: request.request_id.clone(),
            plugin_id: request.plugin_id.clone(),
            ok: false,
            result: Value::Null,
            error: Some(PluginError { code: ErrorCode::Internal, message }),
        },
    }
}

fn file_share_view(share: &FileShareConfig) -> FileShareView {
    let local_running = file_share::is_running(&share.id);
    let lan_running = file_share::lan_is_running(&share.id);
    let lan_host = file_share::lan_host(&share.id)
        .or_else(file_share::detect_lan_ipv4)
        .unwrap_or_default();
    let suffix = if share.require_login { String::new() } else { format!("{}/", share.token) };
    let cloudflare_url = if share.tunnel_hostname.is_empty() {
        String::new()
    } else if share.require_login {
        format!("https://{}/", share.tunnel_hostname)
    } else {
        format!("https://{}/{}/", share.tunnel_hostname, share.token)
    };
    FileShareView {
        id: share.id.clone(),
        name: share.name.clone(),
        enabled: share.enabled,
        view_directory: share.view_directory.clone(),
        receive_directory: share.receive_directory.clone(),
        token: if share.require_login { String::new() } else { share.token.clone() },
        local_url: format!("http://127.0.0.1:{}/{suffix}", share.port),
        local_running,
        port: share.port,
        tunnel_hostname: share.tunnel_hostname.clone(),
        cloudflare_url,
        lan_enabled: share.lan_enabled,
        lan_running,
        lan_host: lan_host.clone(),
        lan_share_url: if share.lan_enabled && !lan_host.is_empty() { format!("http://{lan_host}:{}/{suffix}", share.port) } else { String::new() },
        allow_view: share.allow_view,
        allow_upload: share.allow_upload,
        require_login: share.require_login,
        media_preview_limit_kbps: share.media_preview_limit_kbps,
        max_upload_mb: share.max_upload_mb,
        max_total_upload_mb: share.max_total_upload_mb,
        allowed_ips: share.allowed_ips.clone(),
        access_logs: file_share::get_access_logs(&share.id, 100),
    }
}

fn save_file_share(payload: &Value) -> Result<Value, String> {
    let raw = payload.get("config").ok_or_else(|| "Missing config".to_string())?;
    let mut share: FileShareConfig = serde_json::from_value(raw.clone()).map_err(|error| error.to_string())?;
    let mut shares = config_manager::load_shares()?;
    if share.id.trim().is_empty() {
        share.id = uuid::Uuid::new_v4().to_string();
    }
    if share.name.trim().is_empty() {
        return Err("Instance name is required".into());
    }
    share.name = share.name.trim().to_string();
    if !(1024..=65535).contains(&share.port) {
        return Err("Port must be between 1024 and 65535".into());
    }
    if !(1..=1024).contains(&share.max_upload_mb) {
        return Err("Per-file upload limit must be between 1 and 1024 MB".into());
    }
    if !(1..=8192).contains(&share.max_total_upload_mb) {
        return Err("Receive directory limit must be between 1 and 8192 MB".into());
    }
    if !(64..=8192).contains(&share.media_preview_limit_kbps) {
        return Err("Media preview speed must be between 64 and 8192 KB/s".into());
    }
    share.tunnel_hostname = normalize_cloudflare_hostname(&share.tunnel_hostname)?;
    let old = shares.iter().find(|item| item.id == share.id).cloned();
    let was_running = file_share::is_running(&share.id);
    share.enabled = was_running;
    if let Some(previous) = &old {
        share.token = previous.token.clone();
    }
    if share.token.len() != 32 || !share.token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        share.token = uuid::Uuid::new_v4().simple().to_string();
    }
    file_share::validate_allowed_ips(&share.allowed_ips)?;
    if !share.allow_view && !share.allow_upload {
        return Err("At least one of view or upload must be allowed".into());
    }
    if share.allow_view && share.view_directory.trim().is_empty() {
        return Err("A view directory is required when viewing is enabled".into());
    }
    if share.allow_upload && share.receive_directory.trim().is_empty() && !share.allow_view {
        return Err("A receive directory is required when viewing is disabled".into());
    }
    if shares.iter().any(|item| item.id != share.id && item.port == share.port) {
        return Err("File service ports must be unique".into());
    }
    if !share.tunnel_hostname.is_empty()
        && shares.iter().any(|item| {
            item.id != share.id
                && item.tunnel_hostname.eq_ignore_ascii_case(&share.tunnel_hostname)
        })
    {
        return Err("Cloudflare hostnames must be unique between file service instances".into());
    }
    if was_running {
        if let Err(error) = file_share::start(&mut share) {
            if let Some(mut previous) = old.clone() {
                let _ = file_share::start(&mut previous);
            }
            return Err(error);
        }
    }
    if let Some(index) = shares.iter().position(|item| item.id == share.id) {
        shares[index] = share.clone();
    } else {
        shares.push(share.clone());
    }
    if let Err(error) = config_manager::save_shares(&shares) {
        if was_running {
            file_share::stop_id(&share.id);
            if let Some(mut previous) = old {
                let _ = file_share::start(&mut previous);
            }
        }
        return Err(error);
    }
    Ok(json!({ "id": share.id }))
}

fn delete_file_share(payload: &Value) -> Result<Value, String> {
    let id = string_field(payload, "id")?;
    let mut shares = config_manager::load_shares()?;
    let previous = shares.clone();
    let was_running = file_share::is_running(&id);
    if !shares.iter().any(|share| share.id == id) {
        return Err("Instance not found".into());
    }
    file_share::stop_id(&id);
    shares.retain(|share| share.id != id);
    if let Err(error) = config_manager::save_shares(&shares) {
        if was_running {
            if let Some(mut old) = previous.iter().find(|share| share.id == id).cloned() {
                let _ = file_share::start(&mut old);
            }
        }
        return Err(error);
    }
    file_share::delete_access_logs(&id);
    Ok(Value::Null)
}

fn start_file_share(payload: &Value) -> Result<Value, String> {
    let id = string_field(payload, "id")?;
    let mut shares = config_manager::load_shares()?;
    let previous = shares.clone();
    let share = shares.iter_mut().find(|item| item.id == id).ok_or_else(|| "Instance not found".to_string())?;
    share.enabled = true;
    if let Err(error) = file_share::start(share) {
        return Err(error);
    }
    if let Err(error) = config_manager::save_shares(&shares) {
        file_share::stop_id(&id);
        if let Some(mut old) = previous.into_iter().find(|share| share.id == id) {
            if old.enabled { let _ = file_share::start(&mut old); }
        }
        return Err(error);
    }
    Ok(Value::Null)
}

fn stop_file_share(payload: &Value) -> Result<Value, String> {
    let id = string_field(payload, "id")?;
    let mut shares = config_manager::load_shares()?;
    let previous = shares.clone();
    let was_running = file_share::is_running(&id);
    let share = shares.iter_mut().find(|item| item.id == id).ok_or_else(|| "Instance not found".to_string())?;
    share.enabled = false;
    file_share::stop_id(&id);
    if let Err(error) = config_manager::save_shares(&shares) {
        if was_running {
            if let Some(mut old) = previous.into_iter().find(|share| share.id == id) { let _ = file_share::start(&mut old); }
        }
        return Err(error);
    }
    Ok(Value::Null)
}

fn rotate_file_share_token(payload: &Value) -> Result<Value, String> {
    let id = string_field(payload, "id")?;
    let mut shares = config_manager::load_shares()?;
    let previous = shares.clone();
    let was_running = file_share::is_running(&id);
    let share = shares.iter_mut().find(|item| item.id == id).ok_or_else(|| "Instance not found".to_string())?;
    share.token = uuid::Uuid::new_v4().simple().to_string();
    if was_running {
        if let Err(error) = file_share::start(share) {
            if let Some(mut old) = previous.iter().find(|item| item.id == id).cloned() {
                let _ = file_share::start(&mut old);
            }
            return Err(error);
        }
    }
    if let Err(error) = config_manager::save_shares(&shares) {
        file_share::stop_id(&id);
        if let Some(mut old) = previous.into_iter().find(|share| share.id == id) {
            if was_running { let _ = file_share::start(&mut old); }
        }
        return Err(error);
    }
    let token = shares.iter().find(|share| share.id == id).map(|share| share.token.clone()).unwrap_or_default();
    Ok(json!({ "id": id, "token": token }))
}

fn suggest_port() -> Result<u16, String> {
    let shares = config_manager::load_shares()?;
    for port in 12026u16..=12126u16 {
        if !shares.iter().any(|share| share.port == port)
            && std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
        {
            return Ok(port);
        }
    }
    Err("No available file service port was found".into())
}

fn stop_all_and_disable() -> Result<(), String> {
    file_share::stop();
    let mut shares = config_manager::load_shares()?;
    let mut changed = false;
    for share in &mut shares {
        if share.enabled {
            share.enabled = false;
            changed = true;
        }
    }
    if changed { config_manager::save_shares(&shares)?; }
    Ok(())
}

fn string_field(payload: &Value, key: &str) -> Result<String, String> {
    payload.get(key).and_then(Value::as_str).map(str::to_string)
        .ok_or_else(|| format!("Missing field: {key}"))
}

fn normalize_cloudflare_hostname(value: &str) -> Result<String, String> {
    let hostname = value.trim().trim_matches('.').to_ascii_lowercase();
    if hostname.is_empty() {
        return Ok(String::new());
    }
    if hostname.len() > 253
        || hostname.split('.').count() < 2
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
        return Err("Cloudflare 域名格式无效，请填写完整 DNS 域名，例如 files.example.com".into());
    }
    Ok(hostname)
}

fn pick_directory(title: &str) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let title = title.replace('\'', "''");
        let script = format!(r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = '{title}'
$dialog.ShowNewFolderButton = $true
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    Write-Output $dialog.SelectedPath
}}
"#);
        let output = Command::new("powershell")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(script)
            .output()
            .map_err(|error| format!("Unable to start folder picker: {error}"))?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if message.is_empty() { "Folder picker failed".into() } else { message });
        }
        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((!selected.is_empty()).then_some(selected))
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        Err("The folder picker is not supported on this platform".into())
    }
}

