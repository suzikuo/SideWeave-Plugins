use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind,
    PluginResponse, PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION,
    MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PLUGIN_ID: &str = "ssh_manager";

sideweave_plugin_sdk::export_plugin_dylib!(SshPlugin);
const STATE_KEY: &str = "state";
const KEYS_KEY: &str = "keys";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const OPENSSH_KEEPALIVE_OPTIONS: [&str; 6] = [
    "-o",
    "ServerAliveInterval=30",
    "-o",
    "ServerAliveCountMax=6",
    "-o",
    "TCPKeepAlive=yes",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SshProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
    pub key_path: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SshProfileInput {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
    pub key_path: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SshKeyInfo {
    pub name: String,
    pub path: String,
    pub imported_at: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct ProfilesFile {
    profiles: Vec<SshProfile>,
}

fn load(context: &PluginContext) -> ProfilesFile {
    context
        .storage()
        .get(context.plugin_id(), STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn save(context: &PluginContext, file: &ProfilesFile) -> Result<(), String> {
    let value = serde_json::to_string(file).map_err(|error| error.to_string())?;
    context
        .storage()
        .set(context.plugin_id(), STATE_KEY, &value)
        .map_err(|error| error.message)
}

fn load_keys(context: &PluginContext) -> Vec<SshKeyInfo> {
    context
        .storage()
        .get(context.plugin_id(), KEYS_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn save_keys(context: &PluginContext, keys: &[SshKeyInfo]) -> Result<(), String> {
    let value = serde_json::to_string(keys).map_err(|error| error.to_string())?;
    context
        .storage()
        .set(context.plugin_id(), KEYS_KEY, &value)
        .map_err(|error| error.message)
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    tags.iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn validate(profile: &SshProfileInput) -> Result<(), String> {
    if profile.name.trim().is_empty() { return Err("名称不能为空".into()); }
    if profile.host.trim().is_empty() { return Err("主机不能为空".into()); }
    if profile.user.trim().is_empty() { return Err("用户不能为空".into()); }
    if profile.port == 0 { return Err("端口需在 1-65535 之间".into()); }
    if profile.auth != "password" && profile.auth != "key" {
        return Err("认证方式仅支持 password / key".into());
    }
    Ok(())
}

pub fn ssh_args(profile: &SshProfile) -> Vec<String> {
    let mut args = vec!["ssh".into(), "-p".into(), profile.port.to_string()];
    args.extend(OPENSSH_KEEPALIVE_OPTIONS.iter().map(|value| (*value).into()));
    if profile.auth == "key" && !profile.key_path.trim().is_empty() {
        args.extend(["-o".into(), "IdentitiesOnly=yes".into()]);
        args.push("-i".into());
        args.push(profile.key_path.trim().into());
    }
    args.push(format!("{}@{}", profile.user.trim(), profile.host.trim()));
    args
}

fn find_profile(context: &PluginContext, id: &str) -> Result<SshProfile, String> {
    load(context).profiles.into_iter().find(|profile| profile.id == id)
        .ok_or_else(|| "会话不存在".into())
}

fn find_wt() -> Option<PathBuf> {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let path = PathBuf::from(local).join("Microsoft").join("WindowsApps").join("wt.exe");
        if path.is_file() { return Some(path); }
    }
    which("wt.exe")
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|directory| directory.join(name)).find(|candidate| candidate.is_file())
}

fn resolve_key_path(context: &PluginContext, key_path: &str) -> Option<PathBuf> {
    let key = key_path.trim();
    if key.is_empty() { return None; }
    let raw = PathBuf::from(key);
    if raw.is_absolute() { return raw.is_file().then_some(raw); }
    if key.contains(['/', '\\']) || key == "." || key == ".." { return None; }
    let managed = context.files().private_path(context.plugin_id(), &format!("keys/{key}")).ok().map(PathBuf::from)?;
    managed.is_file().then_some(managed)
}

fn resolve_profile_key_path(context: &PluginContext, key_path: &str) -> Result<PathBuf, String> {
    let key = key_path.trim();
    if key.is_empty() {
        return Err("此会话使用密钥认证，请选择或填写私钥文件".into());
    }
    let raw = PathBuf::from(key);
    if raw.is_absolute() {
        return raw.is_file()
            .then_some(raw)
            .ok_or_else(|| format!("私钥文件不存在：{key}"));
    }
    if key.contains(['/', '\\']) || key == "." || key == ".." {
        return Err("托管密钥名称无效".into());
    }
    let relative = format!("keys/{key}");
    let path = context
        .files()
        .private_path(context.plugin_id(), &relative)
        .map_err(|error| error.message)?;
    let bytes = context
        .files()
        .read_private(context.plugin_id(), &relative)
        .map_err(|error| format!("读取托管密钥失败：{}（路径：{path}）", error.message))?
        .ok_or_else(|| format!("托管密钥不存在：{key}，请重新导入"))?;
    if bytes.is_empty() {
        return Err(format!("托管密钥为空：{key}，请重新导入"));
    }
    Ok(PathBuf::from(path))
}

fn open_terminal(context: &PluginContext, profile: &SshProfile) -> Result<(), String> {
    let mut resolved = profile.clone();
    if profile.auth == "key" {
        resolved.key_path = resolve_profile_key_path(context, &profile.key_path)?
            .to_string_lossy()
            .into_owned();
    }
    let args = ssh_args(&resolved);
    if let Some(wt) = find_wt() {
        let mut terminal_args = vec!["new-tab".into(), "--title".into(), profile.name.clone()];
        terminal_args.extend(args);
        return context.system().spawn_with_cwd(&wt.to_string_lossy(), &terminal_args, None).map_err(|error| error.message);
    }
    let mut cmd_args = vec!["/C".into(), "start".into(), "".into(), "cmd".into(), "/K".into()];
    cmd_args.extend(args);
    context.system().spawn_with_cwd("cmd", &cmd_args, None).map_err(|error| error.message)
}

fn test_connect(profile: &SshProfile) -> Result<u128, String> {
    let address = format!("{}:{}", profile.host.trim(), profile.port);
    let target = address.to_socket_addrs().map_err(|error| format!("无法解析 {address}：{error}"))?.next().ok_or_else(|| format!("无法解析 {address}"))?;
    let started = Instant::now();
    match TcpStream::connect_timeout(&target, CONNECT_TIMEOUT) {
        Ok(_) => Ok(started.elapsed().as_millis()),
        Err(error) => {
            let reason = match error.kind() { std::io::ErrorKind::TimedOut => "超时", std::io::ErrorKind::ConnectionRefused => "连接被拒绝", _ => "连接失败" };
            Err(format!("无法连接 {address}：{reason}"))
        }
    }
}

fn import_key(context: &PluginContext) -> Result<Option<SshKeyInfo>, String> {
    let Some(source) = context.system().pick_file(&["pem".into(), "ppk".into(), "ppkx".into(), "key".into(), "pub".into(), "rsa".into(), "*".into()]).map_err(|error| error.message)? else { return Ok(None); };
    let bytes = context.files().read_external(&source).map_err(|error| format!("读取密钥失败: {}", error.message))?;
    if bytes.is_empty() { return Err("密钥文件为空".into()); }
    let source_name = PathBuf::from(&source).file_name().map(|name| name.to_string_lossy().into_owned()).ok_or_else(|| "路径无文件名".to_string())?;
    validate_private_key(&bytes, &source_name)?;
    let mut name = source_name;
    let mut keys = load_keys(context);
    if let Some(existing) = keys.iter().find(|key| key.name == name) {
        let existing_path = format!("keys/{}", existing.name);
        if context.files().read_private(context.plugin_id(), &existing_path).ok().flatten().as_deref() == Some(bytes.as_slice()) { return Ok(Some(existing.clone())); }
        let digest = Sha256::digest(&bytes);
        let hash: String = digest.iter().take(4).map(|byte| format!("{byte:02x}")).collect();
        let path = PathBuf::from(&name);
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        name = match path.extension().map(|value| value.to_string_lossy()) { Some(extension) => format!("{stem}_{hash}.{extension}"), None => format!("{stem}_{hash}") };
    }
    context.files().write_private(context.plugin_id(), &format!("keys/{name}"), &bytes).map_err(|error| format!("写入托管密钥失败: {}", error.message))?;
    let info = SshKeyInfo {
        name: name.clone(),
        path: context.files().private_path(context.plugin_id(), &format!("keys/{name}")).unwrap_or_else(|_| format!("keys/{name}")),
        imported_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_secs().to_string()).unwrap_or_default(),
        size: bytes.len() as u64,
    };
    keys.retain(|key| key.name != info.name);
    keys.push(info.clone());
    save_keys(context, &keys)?;
    Ok(Some(info))
}

fn validate_private_key(bytes: &[u8], name: &str) -> Result<(), String> {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with("ssh-") || text.contains(" PUBLIC KEY-----") {
        return Err(format!("{name} 是公钥文件，请选择私钥文件"));
    }
    if trimmed.starts_with("PuTTY-User-Key-File-") {
        return Err(format!("暂不支持 {name} 的 PPK 格式，请先转换为 OpenSSH PEM 私钥"));
    }
    if !text.contains("PRIVATE KEY") {
        return Err(format!("无法识别 {name}，请选择 OpenSSH PEM 私钥"));
    }
    Ok(())
}

fn delete_key(context: &PluginContext, name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." { return Err("非法的密钥名".into()); }
    context.files().remove_private(context.plugin_id(), &format!("keys/{name}")).map_err(|error| format!("删除失败: {}", error.message))?;
    let mut keys = load_keys(context);
    let before = keys.len();
    keys.retain(|key| key.name != name);
    if before == keys.len() { return Err("密钥不存在".into()); }
    save_keys(context, &keys)
}

fn save_profile(context: &PluginContext, input: SshProfileInput) -> Result<SshProfile, String> {
    validate(&input)?;
    let mut file = load(context);
    let profile = SshProfile { id: if input.id.trim().is_empty() { uuid::Uuid::new_v4().to_string() } else { input.id.clone() }, name: input.name.trim().into(), host: input.host.trim().into(), port: input.port, user: input.user.trim().into(), auth: input.auth, key_path: input.key_path.trim().into(), tags: clean_tags(&input.tags) };
    if let Some(target) = file.profiles.iter_mut().find(|item| item.id == profile.id) { *target = profile.clone(); } else { file.profiles.push(profile.clone()); }
    save(context, &file)?;
    Ok(profile)
}

pub struct SshPlugin { manifest: PluginManifest }

impl Default for SshPlugin {
    fn default() -> Self { Self::new() }
}

const SERVER_ICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 24 24' fill='none' stroke='%2300b4d8' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Crect width='20' height='8' x='2' y='2' rx='2' ry='2'%3E%3C/rect%3E%3Crect width='20' height='8' x='2' y='14' rx='2' ry='2'%3E%3C/rect%3E%3Cline x1='6' x2='6.01' y1='6' y2='6'%3E%3C/line%3E%3Cline x1='6' x2='6.01' y1='18' y2='18'%3E%3C/line%3E%3C/svg%3E";

impl SshPlugin {
    pub fn new() -> Self {
        Self { manifest: PluginManifest { manifest_version: MANIFEST_VERSION, api_version: API_VERSION, id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"), name: "SSH".into(), icon: "ui/icon.svg".into(), version: env!("CARGO_PKG_VERSION").into(), description: "SSH profile management, key storage, terminal launch, and connectivity checks".into(), author: "SideWeave".into(), runtime: PluginRuntime::DynamicLibrary, backend: Some("plugin.dll".into()), trust: PluginTrust::TrustedLocal, auto_start: false, permissions: vec!["storage".into(), "filesystem.read".into(), "filesystem.write".into(), "filesystem.external_read".into(), "system.file_picker".into(), "network.connect".into(), "process.spawn".into()], ui: Some(PluginUi { entry: "ui/index.html".into(), kind: Some(PluginUiKind::Html), asset_root: "ui".into(), initial_width: Some(760), initial_height: Some(760), ..Default::default() }), hover_preview: Some(sideweave_plugin_api::PluginHoverPreview { width: Some(300), height: Some(280), data_method: Some("get_preview".into()), entry: None }), minimum_host_version: None } }
    }

    fn get_preview(&self, context: &PluginContext) -> Result<Value, String> {
        let file = load(context);
        let total = file.profiles.len();

        if total == 0 {
            return Ok(json!({
                "title": "SSH 会话",
                "subtitle": "暂未添加远程服务器",
                "status": "info",
                "statusText": "无会话",
                "items": [],
                "actions": [
                    { "id": "open", "label": "添加服务器", "primary": true }
                ]
            }));
        }

        let items: Vec<Value> = file.profiles
            .iter()
            .take(8)
            .map(|profile| {
                json!({
                    "id": profile.id,
                    "title": profile.name,
                    "tooltip": format!("{}: {}@{}:{}", profile.name, profile.user, profile.host, profile.port),
                    "icon": SERVER_ICON,
                    "method": "open_terminal",
                    "payload": { "id": profile.id }
                })
            })
            .collect();

        Ok(json!({
            "title": "SSH 会话",
            "subtitle": format!("已配置 {total} 台远程机器，点击直接连接"),
            "status": "online",
            "statusText": format!("{total} 台服务器"),
            "items": items,
            "actions": [
                { "id": "open", "label": "管理会话", "primary": false }
            ]
        }))
    }

    fn call(&self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        match request.method.as_deref() {
            Some("list") => Ok(serde_json::to_value(load(context).profiles).map_err(|error| error.to_string())?),
            Some("save") => Ok(serde_json::to_value(save_profile(context, parse(&request.payload, "profile")?)?).map_err(|error| error.to_string())?),
            Some("delete") => { let id = string_field(&request.payload, "id")?; let mut file = load(context); let before = file.profiles.len(); file.profiles.retain(|profile| profile.id != id); if before == file.profiles.len() { return Err("会话不存在".into()); } save(context, &file)?; Ok(Value::Null) }
            Some("open_terminal") => { let profile = find_profile(context, &string_field(&request.payload, "id")?)?; open_terminal(context, &profile)?; Ok(Value::Null) }
            Some("test") => { let profile = find_profile(context, &string_field(&request.payload, "id")?)?; Ok(json!(test_connect(&profile)?)) }
            Some("import_key") => Ok(serde_json::to_value(import_key(context)?).map_err(|error| error.to_string())?),
            Some("list_keys") => Ok(serde_json::to_value(load_keys(context)).map_err(|error| error.to_string())?),
            Some("delete_key") => { delete_key(context, &string_field(&request.payload, "name")?)?; Ok(Value::Null) }
            Some("resolve_key") => { let key = string_field(&request.payload, "key_path")?; Ok(resolve_key_path(context, &key).map(|path| Value::String(path.to_string_lossy().into_owned())).unwrap_or(Value::Null)) }
            Some("get_preview") => self.get_preview(context),
            Some(method) => Err(format!("不支持的 SSH 方法: {method}")),
            None => Err("缺少插件方法".into()),
        }
    }
}

impl Plugin for SshPlugin {
    fn manifest(&self) -> &PluginManifest { &self.manifest }
    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if !matches!(request.kind, PluginRequestKind::Call) { return success_response(&request, Value::Null); }
        match self.call(&request, context) {
            Ok(result) => success_response(&request, result),
            Err(message) => PluginResponse { request_id: request.request_id, plugin_id: request.plugin_id, ok: false, result: Value::Null, error: Some(PluginError { code: ErrorCode::Internal, message }) },
        }
    }
}

fn parse<T: for<'de> Deserialize<'de>>(payload: &Value, key: &str) -> Result<T, String> {
    payload.get(key).cloned().ok_or_else(|| format!("缺少字段: {key}")).and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
}

fn string_field(payload: &Value, key: &str) -> Result<String, String> {
    payload.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("缺少字段: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> SshProfile { SshProfile { id: "1".into(), name: "web".into(), host: "1.2.3.4".into(), port: 2222, user: "root".into(), auth: "password".into(), key_path: String::new(), tags: vec!["生产".into()] } }
    #[test] fn ssh_args_password_profile_includes_keepalive() { assert_eq!(ssh_args(&profile()), vec!["ssh", "-p", "2222", "-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=6", "-o", "TCPKeepAlive=yes", "root@1.2.3.4"]); }
    #[test] fn ssh_args_key_profile_includes_identity_and_keepalive_options() { let mut value = profile(); value.auth = "key".into(); value.key_path = r"C:\my keys\id_ed25519".into(); assert_eq!(ssh_args(&value), vec!["ssh", "-p", "2222", "-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=6", "-o", "TCPKeepAlive=yes", "-o", "IdentitiesOnly=yes", "-i", r"C:\my keys\id_ed25519", "root@1.2.3.4"]); }
    #[test] fn clean_tags_trims_and_drops_empty() { assert_eq!(clean_tags(&[" 生产 ".into(), "".into(), "  ".into(), "测试".into()]), vec!["生产", "测试"]); }
    #[test] fn validate_private_key_accepts_openssh_and_pem_headers() { assert!(validate_private_key(b"-----BEGIN OPENSSH PRIVATE KEY-----", "id_ed25519").is_ok()); assert!(validate_private_key(b"-----BEGIN RSA PRIVATE KEY-----", "id_rsa").is_ok()); }
    #[test] fn validate_private_key_rejects_public_and_ppk_files() { assert!(validate_private_key(b"ssh-ed25519 AAAA", "id_ed25519.pub").is_err()); assert!(validate_private_key(b"PuTTY-User-Key-File-2: ssh-rsa", "id_rsa.ppk").is_err()); }
    #[test] fn ssh_manifest_includes_hover_preview() {
        let plugin = SshPlugin::new();
        let hover = plugin.manifest().hover_preview.as_ref().expect("hover preview should be configured");
        assert_eq!(hover.data_method.as_deref(), Some("get_preview"));
        assert_eq!(hover.width, Some(300));
        assert_eq!(hover.height, Some(280));
    }
}
