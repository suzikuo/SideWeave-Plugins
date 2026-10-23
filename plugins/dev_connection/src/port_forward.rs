mod listener_process;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind, PluginResponse,
    PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION, MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{success_response, Plugin, PluginContext};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PLUGIN_ID: &str = "port_forward";
const STATE_KEY: &str = "dev_connection_port_forward";
const LISTEN_HOST: &str = "0.0.0.0";
const MAX_CONNECTION_LOGS: usize = 200;
const CONNECTION_LOG_RETENTION: Duration = Duration::from_secs(15 * 60);
const LOG_PRUNE_INTERVAL: Duration = Duration::from_secs(30);
const TARGET_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_INSPECTED_REQUEST_HEADER: usize = 16 * 1024;


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GwRule {
    pub id: String,
    pub name: String,
    pub listen_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub enabled: bool,
    pub in_preview: bool,
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
    pub in_preview: bool,
}

struct Entry {
    stop: Arc<AtomicBool>,
    conns: Arc<AtomicU64>,
}

static RUNTIME: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
static CONNECTION_LOGS: OnceLock<Mutex<VecDeque<Arc<ConnectionLog>>>> = OnceLock::new();
static LOG_PRUNER: OnceLock<Mutex<Option<LogPruner>>> = OnceLock::new();
static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

struct RequestDetails {
    protocol: String,
    method: Option<String>,
    host: Option<String>,
    path: Option<String>,
}

impl Default for RequestDetails {
    fn default() -> Self {
        Self {
            protocol: "TCP".into(),
            method: None,
            host: None,
            path: None,
        }
    }
}

struct ConnectionStatus {
    label: String,
    error: Option<String>,
}

struct ConnectionLog {
    id: u64,
    started_at_ms: u64,
    started: Instant,
    rule_name: String,
    source_addr: String,
    listen_addr: String,
    target_endpoint: String,
    actual_target_addr: Mutex<Option<String>>,
    details: Mutex<RequestDetails>,
    status: Mutex<ConnectionStatus>,
    bytes_from_client: AtomicU64,
    bytes_from_target: AtomicU64,
    finished_at_ms: AtomicU64,
}

struct LogPruner {
    stop: Arc<(Mutex<bool>, Condvar)>,
    handle: JoinHandle<()>,
}

fn runtime() -> &'static Mutex<HashMap<String, Entry>> {
    RUNTIME.get_or_init(|| Mutex::new(HashMap::new()))
}

fn connection_logs() -> &'static Mutex<VecDeque<Arc<ConnectionLog>>> {
    CONNECTION_LOGS.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn log_pruner() -> &'static Mutex<Option<LogPruner>> {
    LOG_PRUNER.get_or_init(|| Mutex::new(None))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn prune_expired_logs() {
    let cutoff = now_ms().saturating_sub(CONNECTION_LOG_RETENTION.as_millis() as u64);
    if let Ok(mut logs) = connection_logs().lock() {
        logs.retain(|item| item.started_at_ms >= cutoff);
        while logs.len() > MAX_CONNECTION_LOGS {
            logs.pop_front();
        }
    }
}

fn ensure_log_pruner() {
    let Ok(mut worker) = log_pruner().lock() else {
        return;
    };
    if worker.is_some() {
        return;
    }

    let stop = Arc::new((Mutex::new(false), Condvar::new()));
    let thread_stop = stop.clone();
    let Ok(handle) = thread::Builder::new()
        .name("port-forward-log-pruner".into())
        .spawn(move || loop {
            let (lock, wake) = &*thread_stop;
            let Ok(stopped) = lock.lock() else { break };
            let Ok((stopped, _)) = wake.wait_timeout(stopped, LOG_PRUNE_INTERVAL) else {
                break;
            };
            if *stopped {
                break;
            }
            drop(stopped);
            prune_expired_logs();
        })
    else {
        return;
    };
    *worker = Some(LogPruner { stop, handle });
}

fn stop_log_pruner_and_clear() {
    let worker = log_pruner()
        .lock()
        .ok()
        .and_then(|mut worker| worker.take());
    if let Some(worker) = worker {
        let (lock, wake) = &*worker.stop;
        if let Ok(mut stopped) = lock.lock() {
            *stopped = true;
            wake.notify_all();
        }
        let _ = worker.handle.join();
    }
    clear_connection_logs();
}

fn clear_connection_logs() {
    if let Ok(mut logs) = connection_logs().lock() {
        logs.clear();
    }
}

fn endpoint(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn new_connection_log(
    rule_name: &str,
    source_addr: String,
    listen_addr: String,
    target_host: &str,
    target_port: u16,
) -> Arc<ConnectionLog> {
    ensure_log_pruner();
    prune_expired_logs();

    let item = Arc::new(ConnectionLog {
        id: NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed),
        started_at_ms: now_ms(),
        started: Instant::now(),
        rule_name: rule_name.chars().take(128).collect(),
        source_addr,
        listen_addr,
        target_endpoint: endpoint(target_host, target_port),
        actual_target_addr: Mutex::new(None),
        details: Mutex::new(RequestDetails::default()),
        status: Mutex::new(ConnectionStatus {
            label: "连接目标中".into(),
            error: None,
        }),
        bytes_from_client: AtomicU64::new(0),
        bytes_from_target: AtomicU64::new(0),
        finished_at_ms: AtomicU64::new(0),
    });

    if let Ok(mut logs) = connection_logs().lock() {
        logs.push_back(item.clone());
        while logs.len() > MAX_CONNECTION_LOGS {
            logs.pop_front();
        }
    }
    item
}

fn update_log_status(item: &ConnectionLog, label: &str, error: Option<String>, finished: bool) {
    if let Ok(mut status) = item.status.lock() {
        status.label = label.into();
        if error.is_some() {
            status.error = error.map(|value| value.chars().take(512).collect());
        }
    }
    if finished {
        item.finished_at_ms.store(now_ms(), Ordering::Relaxed);
    }
}

fn connection_log_value(item: &ConnectionLog) -> Value {
    let details = item
        .details
        .lock()
        .map(|value| RequestDetails {
            protocol: value.protocol.clone(),
            method: value.method.clone(),
            host: value.host.clone(),
            path: value.path.clone(),
        })
        .unwrap_or_default();
    let status = item
        .status
        .lock()
        .map(|value| (value.label.clone(), value.error.clone()))
        .unwrap_or_else(|_| ("状态未知".into(), None));
    let finished_at_ms = item.finished_at_ms.load(Ordering::Relaxed);
    let elapsed_ms = if finished_at_ms == 0 {
        item.started.elapsed().as_millis() as u64
    } else {
        finished_at_ms.saturating_sub(item.started_at_ms)
    };
    let request_url = details.path.as_ref().map(|path| {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.clone()
        } else if details.method.as_deref() == Some("CONNECT") {
            format!("https://{path}")
        } else if let Some(host) = details.host.as_deref() {
            format!("http://{host}{path}")
        } else {
            path.clone()
        }
    });
    json!({
        "id": item.id,
        "started_at_ms": item.started_at_ms,
        "elapsed_ms": elapsed_ms,
        "finished": finished_at_ms != 0,
        "rule_name": item.rule_name,
        "source_addr": item.source_addr,
        "listen_addr": item.listen_addr,
        "target_endpoint": item.target_endpoint,
        "actual_target_addr": item.actual_target_addr.lock().ok().and_then(|value| value.clone()),
        "protocol": details.protocol,
        "method": details.method,
        "host": details.host,
        "path": details.path,
        "request_url": request_url,
        "bytes_from_client": item.bytes_from_client.load(Ordering::Relaxed),
        "bytes_from_target": item.bytes_from_target.load(Ordering::Relaxed),
        "status": status.0,
        "error": status.1,
    })
}

fn list_connection_logs() -> Value {
    prune_expired_logs();
    let values = connection_logs()
        .lock()
        .map(|logs| {
            logs.iter()
                .rev()
                .map(|item| connection_log_value(item))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Value::Array(values)
}

fn connect_target(host: &str, port: u16) -> io::Result<TcpStream> {
    let addresses = (host, port).to_socket_addrs()?.collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "目标主机未解析出地址",
        ));
    }
    let deadline = Instant::now() + TARGET_CONNECT_TIMEOUT;
    let mut last_error = None;
    for address in addresses {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "连接目标超时")))
}

struct RequestInspector {
    prefix: Vec<u8>,
    done: bool,
}

impl RequestInspector {
    fn new() -> Self {
        Self {
            prefix: Vec::new(),
            done: false,
        }
    }

    fn inspect(&mut self, chunk: &[u8], log: &ConnectionLog) {
        if self.done {
            return;
        }
        let remaining = MAX_INSPECTED_REQUEST_HEADER.saturating_sub(self.prefix.len());
        self.prefix
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);

        if self.prefix.first() == Some(&0x16) {
            if let Ok(mut details) = log.details.lock() {
                details.protocol = "TLS".into();
                if let Some(host) = parse_tls_server_name(&self.prefix) {
                    details.host = Some(host);
                }
            }
            if tls_record_is_complete(&self.prefix)
                || self.prefix.len() >= MAX_INSPECTED_REQUEST_HEADER
            {
                self.done = true;
                self.prefix.clear();
            }
            return;
        }

        let header_end = self
            .prefix
            .windows(4)
            .position(|window| window == b"\r\n\r\n");
        let inspected_end = header_end
            .map(|index| index + 4)
            .unwrap_or(self.prefix.len());
        let text = String::from_utf8_lossy(&self.prefix[..inspected_end]);
        let mut lines = text.split("\r\n");
        if let Some(request_line) = lines.next() {
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or("");
            let path = parts.next().unwrap_or("");
            let version = parts.next().unwrap_or("");
            if !method.is_empty() && version.starts_with("HTTP/") {
                if let Ok(mut details) = log.details.lock() {
                    details.protocol = "HTTP".into();
                    details.method = Some(method.chars().take(32).collect());
                    details.path = Some(path.chars().take(2048).collect());
                    for line in lines {
                        if let Some((name, value)) = line.split_once(':') {
                            if name.eq_ignore_ascii_case("host") {
                                details.host = Some(value.trim().chars().take(512).collect());
                                break;
                            }
                        }
                    }
                }
                if header_end.is_some() || self.prefix.len() >= MAX_INSPECTED_REQUEST_HEADER {
                    self.done = true;
                    self.prefix.clear();
                }
                return;
            }
        }

        if self.prefix.len() >= MAX_INSPECTED_REQUEST_HEADER
            || self
                .prefix
                .iter()
                .any(|byte| *byte == 0 || !byte.is_ascii())
        {
            self.done = true;
            self.prefix.clear();
        }
    }
}

fn tls_record_is_complete(bytes: &[u8]) -> bool {
    if bytes.len() < 5 || bytes[0] != 0x16 {
        return false;
    }
    let length = u16::from_be_bytes([bytes[3], bytes[4]]) as usize;
    bytes.len() >= 5 + length
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<usize> {
    Some(u16::from_be_bytes([*bytes.get(offset)?, *bytes.get(offset + 1)?]) as usize)
}

fn parse_tls_server_name(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 9 || bytes[0] != 0x16 || bytes[5] != 0x01 {
        return None;
    }
    let record_end = 5usize.checked_add(read_u16(bytes, 3)?)?;
    if record_end > bytes.len() {
        return None;
    }
    let handshake_end = 9usize.checked_add(
        ((bytes[6] as usize) << 16) | ((bytes[7] as usize) << 8) | bytes[8] as usize,
    )?;
    if handshake_end > record_end {
        return None;
    }

    let mut offset = 9usize.checked_add(2 + 32)?;
    let session_id_len = *bytes.get(offset)? as usize;
    offset = offset.checked_add(1 + session_id_len)?;
    let cipher_suites_len = read_u16(bytes, offset)?;
    offset = offset.checked_add(2 + cipher_suites_len)?;
    let compression_len = *bytes.get(offset)? as usize;
    offset = offset.checked_add(1 + compression_len)?;
    let extensions_len = read_u16(bytes, offset)?;
    offset = offset.checked_add(2)?;
    let extensions_end = offset.checked_add(extensions_len)?.min(handshake_end);

    while offset.checked_add(4)? <= extensions_end {
        let extension_type = read_u16(bytes, offset)?;
        let extension_len = read_u16(bytes, offset + 2)?;
        let data_start = offset + 4;
        let data_end = data_start.checked_add(extension_len)?;
        if data_end > extensions_end {
            return None;
        }
        if extension_type == 0 && extension_len >= 5 {
            let mut name_offset = data_start.checked_add(2)?;
            let names_end = data_start
                .checked_add(read_u16(bytes, data_start)?)?
                .min(data_end);
            while name_offset.checked_add(3)? <= names_end {
                let name_type = *bytes.get(name_offset)?;
                let name_len = read_u16(bytes, name_offset + 1)?;
                let name_start = name_offset + 3;
                let name_end = name_start.checked_add(name_len)?;
                if name_end > names_end {
                    return None;
                }
                if name_type == 0 {
                    let name = String::from_utf8_lossy(bytes.get(name_start..name_end)?)
                        .trim()
                        .to_string();
                    return (!name.is_empty()).then(|| name.chars().take(512).collect());
                }
                name_offset = name_end;
            }
        }
        offset = data_end;
    }
    None
}

fn relay_stream(
    mut source: TcpStream,
    mut destination: TcpStream,
    from_client: bool,
    log: Arc<ConnectionLog>,
) -> io::Result<()> {
    let result = (|| {
        let mut buffer = [0u8; 16 * 1024];
        let mut inspector = RequestInspector::new();
        loop {
            let count = match source.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            destination.write_all(&buffer[..count])?;
            if from_client {
                log.bytes_from_client
                    .fetch_add(count as u64, Ordering::Relaxed);
                inspector.inspect(&buffer[..count], &log);
            } else {
                log.bytes_from_target
                    .fetch_add(count as u64, Ordering::Relaxed);
            }
        }
        destination.shutdown(Shutdown::Write)
    })();

    if result.is_err() {
        let _ = source.shutdown(Shutdown::Both);
        let _ = destination.shutdown(Shutdown::Both);
    }
    result
}

fn complete_direction(
    result: io::Result<()>,
    from_client: bool,
    completed: &AtomicU64,
    conns: &AtomicU64,
    log: &ConnectionLog,
) {
    if let Err(error) = result {
        let direction = if from_client {
            "客户端到目标"
        } else {
            "目标到客户端"
        };
        update_log_status(
            log,
            "转发异常",
            Some(format!("{direction}: {error}")),
            false,
        );
    }
    if completed.fetch_add(1, Ordering::AcqRel) == 1 {
        conns.fetch_sub(1, Ordering::Relaxed);
        let failed = log
            .status
            .lock()
            .map(|status| status.error.is_some())
            .unwrap_or(true);
        update_log_status(log, if failed { "转发异常" } else { "已结束" }, None, true);
    }
}

fn handle_connection(
    conns: Arc<AtomicU64>,
    client: TcpStream,
    rule_name: String,
    listen_port: u16,
    host: String,
    port: u16,
) {
    let source_addr = client
        .peer_addr()
        .map(|address| address.to_string())
        .unwrap_or_else(|_| "未知来源".into());
    let listen_addr = client
        .local_addr()
        .map(|address| address.to_string())
        .unwrap_or_else(|_| format!("{LISTEN_HOST}:{listen_port}"));
    let log = new_connection_log(&rule_name, source_addr, listen_addr, &host, port);
    let target = match connect_target(&host, port) {
        Ok(stream) => stream,
        Err(error) => {
            update_log_status(&log, "连接失败", Some(error.to_string()), true);
            return;
        }
    };
    if let Ok(address) = target.peer_addr() {
        if let Ok(mut actual) = log.actual_target_addr.lock() {
            *actual = Some(address.to_string());
        }
    }

    let clones = (
        client.try_clone(),
        target.try_clone(),
        target.try_clone(),
        client.try_clone(),
    );
    let (Ok(client_to_target), Ok(target_write), Ok(target_to_client), Ok(client_write)) = clones
    else {
        update_log_status(
            &log,
            "转发启动失败",
            Some("无法创建 TCP 转发流".into()),
            true,
        );
        return;
    };

    conns.fetch_add(1, Ordering::Relaxed);
    update_log_status(&log, "转发中", None, false);
    let completed = Arc::new(AtomicU64::new(0));

    let log_out = log.clone();
    let conns_out = conns.clone();
    let completed_out = completed.clone();
    thread::spawn(move || {
        let result = relay_stream(client_to_target, target_write, true, log_out.clone());
        complete_direction(result, true, &completed_out, &conns_out, &log_out);
    });

    thread::spawn(move || {
        let result = relay_stream(target_to_client, client_write, false, log.clone());
        complete_direction(result, false, &completed, &conns, &log);
    });
}

fn accept_loop(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    conns: Arc<AtomicU64>,
    rule_name: String,
    listen_port: u16,
    host: String,
    port: u16,
) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((client, _)) => {
                let conns = conns.clone();
                let rule_name = rule_name.clone();
                let host = host.clone();
                thread::spawn(move || {
                    handle_connection(conns, client, rule_name, listen_port, host, port)
                });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn load_rules(context: &PluginContext) -> Vec<GwRule> {
    let current = context
        .storage()
        .get(context.plugin_id(), STATE_KEY)
        .ok()
        .flatten();
    let stored = current.or_else(|| {
        let legacy_id = sideweave_plugin_api::PluginId::new(PLUGIN_ID).ok()?;
        let value = context.storage().get(&legacy_id, "state").ok().flatten()?;
        let _ = context.storage().set(context.plugin_id(), STATE_KEY, &value);
        Some(value)
    });
    stored
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
        "in_preview": rule.in_preview,
        "running": running,
        "conns": conns,
    })
}

fn stop_rule(id: &str) {
    if let Ok(state) = runtime().lock() {
        if let Some(entry) = state.get(id) {
            entry.stop.store(true, Ordering::SeqCst);
        }
    }
}

fn wait_stopped(id: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let gone = runtime()
            .lock()
            .map(|state| !state.contains_key(id))
            .unwrap_or(true);
        if gone {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn bind_listener(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind((LISTEN_HOST, port))
}

fn start_rule(rule: &GwRule) -> Result<(), String> {
    stop_rule(&rule.id);
    wait_stopped(&rule.id);
    let listener =
        bind_listener(rule.listen_port).map_err(|error| format!("监听端口失败: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let conns = Arc::new(AtomicU64::new(0));
    runtime().lock().map_err(|e| e.to_string())?.insert(
        rule.id.clone(),
        Entry {
            stop: stop.clone(),
            conns: conns.clone(),
        },
    );
    let id = rule.id.clone();
    let name = rule.name.clone();
    let listen_port = rule.listen_port;
    let host = rule.target_host.clone();
    let port = rule.target_port;
    thread::Builder::new()
        .name(format!("port-forward-{id}"))
        .spawn(move || {
            accept_loop(listener, stop, conns, name, listen_port, host, port);
            if let Ok(mut state) = runtime().lock() {
                state.remove(&id);
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn stop_all() {
    let ids = runtime()
        .lock()
        .map(|state| state.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for id in &ids {
        stop_rule(id);
    }
    for id in ids {
        wait_stopped(&id);
    }
}

pub struct PortForwardPlugin {
    manifest: PluginManifest,
}

impl Default for PortForwardPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl PortForwardPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
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
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Html),
                    asset_root: "ui".into(),
                    initial_width: Some(720),
                    initial_height: Some(620),
                    ..Default::default()
                }),
                hover_preview: None,
                auto_start: true,
                minimum_host_version: None,
            },
        }
    }

    fn call(&self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        match request.method.as_deref() {
            Some("list") => Ok(Value::Array(
                load_rules(context).iter().map(rule_value).collect(),
            )),
            Some("connection_logs") => Ok(list_connection_logs()),
            Some("clear_connection_logs") => {
                clear_connection_logs();
                Ok(json!({ "cleared": true }))
            }
            Some("listeners") => {
                let port = request
                    .payload
                    .get("port")
                    .map(|value| {
                        value
                            .as_u64()
                            .and_then(|value| u16::try_from(value).ok())
                            .filter(|port| *port > 0)
                            .ok_or_else(|| "端口号必须是 1-65535 的整数".to_string())
                    })
                    .transpose()?;
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
                if input.name.trim().is_empty()
                    || input.listen_port == 0
                    || input.target_host.trim().is_empty()
                    || input.target_port == 0
                {
                    return Err("规则名称、监听端口、目标主机和目标端口不能为空".into());
                }
                let mut rules = load_rules(context);
                let rule = GwRule {
                    id: if input.id.trim().is_empty() {
                        uuid::Uuid::new_v4().to_string()
                    } else {
                        input.id
                    },
                    name: input.name.trim().into(),
                    listen_port: input.listen_port,
                    target_host: input.target_host.trim().into(),
                    target_port: input.target_port,
                    enabled: input.enabled,
                    in_preview: input.in_preview,
                };
                if let Some(existing) = rules.iter_mut().find(|item| item.id == rule.id) {
                    *existing = rule.clone();
                } else {
                    rules.push(rule.clone());
                }
                let running = runtime()
                    .lock()
                    .map(|state| state.contains_key(&rule.id))
                    .unwrap_or(false);
                save_rules(context, &rules)?;
                if running {
                    start_rule(&rule)?;
                }
                Ok(rule_value(&rule))
            }
            Some("delete") => {
                let id = string_field(&request.payload, "id")?;
                stop_rule(&id);
                wait_stopped(&id);
                let mut rules = load_rules(context);
                let before = rules.len();
                rules.retain(|rule| rule.id != id);
                if before == rules.len() {
                    return Err("规则不存在".into());
                }
                save_rules(context, &rules)?;
                Ok(Value::Null)
            }
            Some("set_running") => {
                let id = string_field(&request.payload, "id")?;
                let running = request
                    .payload
                    .get("running")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let rule = load_rules(context)
                    .into_iter()
                    .find(|rule| rule.id == id)
                    .ok_or_else(|| "规则不存在".to_string())?;
                if running {
                    start_rule(&rule)?;
                } else {
                    stop_rule(&id);
                    wait_stopped(&id);
                }
                Ok(rule_value(&rule))
            }
            Some(method) => Err(format!("不支持的端口转发方法: {method}")),
            None => Err("缺少插件方法".into()),
        }
    }
}

impl Plugin for PortForwardPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        if matches!(request.kind, PluginRequestKind::Activate) {
            for rule in load_rules(context).into_iter().filter(|rule| rule.enabled) {
                let _ = start_rule(&rule);
            }
            return success_response(&request, Value::Null);
        }
        if matches!(
            request.kind,
            PluginRequestKind::Deactivate | PluginRequestKind::Shutdown
        ) {
            stop_all();
            stop_log_pruner_and_clear();
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

fn parse<T: for<'de> Deserialize<'de>>(payload: &Value, key: &str) -> Result<T, String> {
    payload
        .get(key)
        .cloned()
        .ok_or_else(|| format!("缺少字段: {key}"))
        .and_then(|value| serde_json::from_value(value).map_err(|e| e.to_string()))
}

fn string_field(payload: &Value, key: &str) -> Result<String, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("缺少字段: {key}"))
}

#[cfg(test)]
mod tests {
    use super::{bind_listener, LISTEN_HOST};

    #[test]
    fn listener_binds_all_ipv4_interfaces() {
        let listener = bind_listener(0).expect("listener should bind an ephemeral port");
        let address = listener
            .local_addr()
            .expect("listener should expose its address");

        assert_eq!(LISTEN_HOST, "0.0.0.0");
        assert!(address.ip().is_unspecified());
        assert!(address.ip().is_ipv4());
    }
}
