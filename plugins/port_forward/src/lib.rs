mod listener_process;

use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind,
    PluginResponse, PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION,
    MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

const PLUGIN_ID: &str = "port_forward";
const STATE_KEY: &str = "state";
const LISTEN_HOST: &str = "0.0.0.0";

sideweave_plugin_sdk::export_plugin_dylib!(PortForwardPlugin);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GwRule {
    pub id: String,
    pub name: String,
    pub listen_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GwRuleInput {
    pub id: String,
    pub name: String,
    pub listen_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub enabled: bool,
}

struct Entry {
    stop: Arc<AtomicBool>,
    conns: Arc<AtomicU64>,
}

static RUNTIME: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();

fn runtime() -> &'static Mutex<HashMap<String, Entry>> {
    RUNTIME.get_or_init(|| Mutex::new(HashMap::new()))
}

fn load_rules(context: &PluginContext) -> Vec<GwRule> {
    context
        .storage()
        .get(context.plugin_id(), STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<Value>(&value).ok())
        .and_then(|value| value.get("rules").cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn save_rules(context: &PluginContext, rules: &[GwRule]) -> Result<(), String> {
    let value = serde_json::to_string(&json!({ "rules": rules })).map_err(|e| e.to_string())?;
    context
        .storage()
        .set(context.plugin_id(), STATE_KEY, &value)
        .map_err(|error| error.message)
}

fn rule_value(rule: &GwRule) -> Value {
    let (running, conns) = runtime()
        .lock()
        .ok()
        .and_then(|state| {
            state.get(&rule.id).map(|entry| {
                (
                    !entry.stop.load(Ordering::SeqCst),
                    entry.conns.load(Ordering::SeqCst),
                )
            })
        })
        .unwrap_or((false, 0));
    json!({
        "id": rule.id,
        "name": rule.name,
        "listen_port": rule.listen_port,
        "target_host": rule.target_host,
        "target_port": rule.target_port,
        "enabled": rule.enabled,
        "running": running,
        "conns": conns,
    })
}

fn handle_connection(conns: Arc<AtomicU64>, client: TcpStream, host: String, port: u16) {
    let Ok(target) = TcpStream::connect((host.as_str(), port)) else { return };
    let (Ok(client_read), Ok(target_read)) = (client.try_clone(), target.try_clone()) else { return };
    conns.fetch_add(1, Ordering::SeqCst);
    let done = Arc::new(AtomicU64::new(0));
    let left = conns.clone();
    let done_left = done.clone();
    thread::spawn(move || {
        let _ = io::copy(&mut &client_read, &mut &target_read);
        if done_left.fetch_add(1, Ordering::SeqCst) == 1 { left.fetch_sub(1, Ordering::SeqCst); }
    });
    thread::spawn(move || {
        let _ = io::copy(&mut &target, &mut &client);
        if done.fetch_add(1, Ordering::SeqCst) == 1 { conns.fetch_sub(1, Ordering::SeqCst); }
    });
}

fn accept_loop(listener: TcpListener, stop: Arc<AtomicBool>, conns: Arc<AtomicU64>, host: String, port: u16) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((client, _)) => {
                let conns = conns.clone();
                let host = host.clone();
                thread::spawn(move || handle_connection(conns, client, host, port));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(50)),
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn stop_rule(id: &str) {
    if let Ok(state) = runtime().lock() {
        if let Some(entry) = state.get(id) { entry.stop.store(true, Ordering::SeqCst); }
    }
}

fn wait_stopped(id: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let gone = runtime().lock().map(|state| !state.contains_key(id)).unwrap_or(true);
        if gone { return; }
        thread::sleep(Duration::from_millis(20));
    }
}

fn bind_listener(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind((LISTEN_HOST, port))
}

fn start_rule(rule: &GwRule) -> Result<(), String> {
    stop_rule(&rule.id);
    wait_stopped(&rule.id);
    let listener = bind_listener(rule.listen_port)
        .map_err(|error| format!("监听端口失败: {error}"))?;
    listener.set_nonblocking(true).map_err(|error| error.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let conns = Arc::new(AtomicU64::new(0));
    runtime().lock().map_err(|e| e.to_string())?.insert(rule.id.clone(), Entry { stop: stop.clone(), conns: conns.clone() });
    let id = rule.id.clone();
    let host = rule.target_host.clone();
    let port = rule.target_port;
    thread::Builder::new()
        .name(format!("port-forward-{id}"))
        .spawn(move || {
            accept_loop(listener, stop, conns, host, port);
            if let Ok(mut state) = runtime().lock() { state.remove(&id); }
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn stop_all() {
    let ids = runtime().lock().map(|state| state.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    for id in &ids { stop_rule(id); }
    for id in ids { wait_stopped(&id); }
}

pub struct PortForwardPlugin {
    manifest: PluginManifest,
}

impl Default for PortForwardPlugin {
    fn default() -> Self { Self::new() }
}

impl PortForwardPlugin {
    pub fn new() -> Self {
        Self { manifest: PluginManifest {
            manifest_version: MANIFEST_VERSION,
            api_version: API_VERSION,
            id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
            name: "端口工具".into(),
            icon: "ui/icon.svg".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "本地 TCP 转发规则、端口占用与进程管理".into(),
            author: "SideWeave".into(),
            runtime: PluginRuntime::DynamicLibrary,
            backend: Some("plugin.dll".into()),
            trust: PluginTrust::TrustedLocal,
            permissions: vec![
                "storage".into(),
                "network.listen".into(),
                "network.connect".into(),
            ],
            ui: Some(PluginUi { entry: "ui/index.html".into(), kind: Some(PluginUiKind::Html), asset_root: "ui".into(), initial_width: Some(720), initial_height: Some(620), ..Default::default() }),
            hover_preview: None,
            auto_start: true,
            minimum_host_version: None,
        }}
    }

    fn call(&self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        match request.method.as_deref() {
            Some("list") => Ok(Value::Array(load_rules(context).iter().map(rule_value).collect())),
            Some("listeners") => {
                let port = request.payload.get("port").map(|value| {
                    value.as_u64()
                        .and_then(|value| u16::try_from(value).ok())
                        .filter(|port| *port > 0)
                        .ok_or_else(|| "端口号必须是 1-65535 的整数".to_string())
                }).transpose()?;
                serde_json::to_value(listener_process::list_listeners(port)?)
                    .map_err(|error| error.to_string())
            }
            Some("stop_listener") => {
                let request: listener_process::StopListenerRequest =
                    parse(&request.payload, "listener")?;
                listener_process::stop_listener(request)?;
                Ok(json!({ "stopped": true }))
            }
            Some("save") => {
                let input: GwRuleInput = parse(&request.payload, "rule")?;
                if input.name.trim().is_empty() || input.listen_port == 0 || input.target_host.trim().is_empty() || input.target_port == 0 { return Err("规则名称、监听端口、目标主机和目标端口不能为空".into()); }
                let mut rules = load_rules(context);
                let rule = GwRule { id: if input.id.trim().is_empty() { uuid::Uuid::new_v4().to_string() } else { input.id }, name: input.name.trim().into(), listen_port: input.listen_port, target_host: input.target_host.trim().into(), target_port: input.target_port, enabled: input.enabled };
                if let Some(existing) = rules.iter_mut().find(|item| item.id == rule.id) { *existing = rule.clone(); } else { rules.push(rule.clone()); }
                let running = runtime().lock().map(|state| state.contains_key(&rule.id)).unwrap_or(false);
                save_rules(context, &rules)?;
                if running { start_rule(&rule)?; }
                Ok(rule_value(&rule))
            }
            Some("delete") => {
                let id = string_field(&request.payload, "id")?;
                stop_rule(&id);
                wait_stopped(&id);
                let mut rules = load_rules(context);
                let before = rules.len();
                rules.retain(|rule| rule.id != id);
                if before == rules.len() { return Err("规则不存在".into()); }
                save_rules(context, &rules)?;
                Ok(Value::Null)
            }
            Some("set_running") => {
                let id = string_field(&request.payload, "id")?;
                let running = request.payload.get("running").and_then(Value::as_bool).unwrap_or(false);
                let rule = load_rules(context).into_iter().find(|rule| rule.id == id).ok_or_else(|| "规则不存在".to_string())?;
                if running { start_rule(&rule)?; } else { stop_rule(&id); wait_stopped(&id); }
                Ok(rule_value(&rule))
            }
            Some(method) => Err(format!("不支持的端口转发方法: {method}")),
            None => Err("缺少插件方法".into()),
        }
    }
}

impl Plugin for PortForwardPlugin {
    fn manifest(&self) -> &PluginManifest { &self.manifest }
    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if matches!(request.kind, PluginRequestKind::Activate) {
            for rule in load_rules(context).into_iter().filter(|rule| rule.enabled) { let _ = start_rule(&rule); }
            return success_response(&request, Value::Null);
        }
        if matches!(request.kind, PluginRequestKind::Deactivate | PluginRequestKind::Shutdown) { stop_all(); return success_response(&request, Value::Null); }
        if !matches!(request.kind, PluginRequestKind::Call) { return success_response(&request, Value::Null); }
        match self.call(&request, context) {
            Ok(result) => success_response(&request, result),
            Err(message) => PluginResponse { request_id: request.request_id, plugin_id: request.plugin_id, ok: false, result: Value::Null, error: Some(PluginError { code: ErrorCode::Internal, message }) },
        }
    }
}

fn parse<T: for<'de> Deserialize<'de>>(payload: &Value, key: &str) -> Result<T, String> {
    payload.get(key).cloned().ok_or_else(|| format!("缺少字段: {key}"))
        .and_then(|value| serde_json::from_value(value).map_err(|e| e.to_string()))
}

fn string_field(payload: &Value, key: &str) -> Result<String, String> {
    payload.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("缺少字段: {key}"))
}

#[cfg(test)]
mod tests {
    use super::{bind_listener, LISTEN_HOST};

    #[test]
    fn listener_binds_all_ipv4_interfaces() {
        let listener = bind_listener(0).expect("listener should bind an ephemeral port");
        let address = listener.local_addr().expect("listener should expose its address");

        assert_eq!(LISTEN_HOST, "0.0.0.0");
        assert!(address.ip().is_unspecified());
        assert!(address.ip().is_ipv4());
    }
}
