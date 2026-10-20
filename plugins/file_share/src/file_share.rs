use crate::models::FileShareConfig;
use chrono::Local;
use ring::digest::{digest, SHA256};
use ring::pbkdf2::{self, PBKDF2_HMAC_SHA256};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

trait HttpStream: Read + Write {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
}

impl HttpStream for TcpStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_write_timeout(self, timeout)
    }
}

impl HttpStream for rustls::StreamOwned<rustls::ServerConnection, TcpStream> {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.get_ref().set_read_timeout(timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.get_ref().set_write_timeout(timeout)
    }
}

enum ClientStream {
    Plain(TcpStream),
    Tls(rustls::StreamOwned<rustls::ServerConnection, TcpStream>),
}

impl Read for ClientStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for ClientStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

impl HttpStream for ClientStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.set_read_timeout(timeout),
            Self::Tls(stream) => stream.get_ref().set_read_timeout(timeout),
        }
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.set_write_timeout(timeout),
            Self::Tls(stream) => stream.get_ref().set_write_timeout(timeout),
        }
    }
}

impl Drop for ClientStream {
    fn drop(&mut self) {
        if let Self::Tls(stream) = self {
            stream.conn.send_close_notify();
            let _ = stream.flush();
        }
    }
}

const MAX_ACTIVE_REQUESTS: usize = 32;
const MAX_ACCESS_LOG_ENTRIES: usize = 100;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const PASSWORD_HASH_ITERATIONS: u32 = 600_000;
const PASSWORD_HASH_SALT_BYTES: usize = 16;
const PASSWORD_HASH_BYTES: usize = 32;
const MAX_TEXT_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MEDIA_PREVIEW_RANGE_BYTES: u64 = 8 * 1024 * 1024;
const CLIENT_READ_TIMEOUT: Duration = Duration::from_secs(5);
const UPLOAD_READ_TIMEOUT: Duration = Duration::from_secs(300);
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

struct RunningServer {
    lan_host: Option<String>,
    lan_enabled: bool,
    stop: Arc<AtomicBool>,
    clients: Arc<Mutex<Vec<(usize, TcpStream)>>>,
    thread: JoinHandle<()>,
    sessions: Arc<Mutex<HashMap<String, LoginSession>>>,
}

#[derive(Clone)]
struct LoginSession {
    expires: Instant,
    auth_epoch: u64,
}

struct CsrfChallenge {
    expires: Instant,
    client_ip: IpAddr,
    user_agent: String,
}

struct UploadQuota {
    used_bytes: Mutex<u64>,
    maximum_bytes: u64,
}

struct MediaRateState {
    tokens: f64,
    updated_at: Instant,
}

struct MediaRateLimiter {
    bytes_per_second: u64,
    state: Mutex<MediaRateState>,
}

impl MediaRateLimiter {
    const BURST_BYTES: f64 = 32.0 * 1024.0;

    fn new(bytes_per_second: u64) -> Self {
        Self {
            bytes_per_second: bytes_per_second.max(1),
            state: Mutex::new(MediaRateState {
                tokens: Self::BURST_BYTES,
                updated_at: Instant::now(),
            }),
        }
    }

    fn wait_for(&self, bytes: usize, stop: &AtomicBool) -> bool {
        let amount = bytes as f64;
        loop {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            let wait_seconds = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let now = Instant::now();
                let elapsed = now.duration_since(state.updated_at).as_secs_f64();
                state.tokens = (state.tokens + elapsed * self.bytes_per_second as f64)
                    .min(Self::BURST_BYTES);
                state.updated_at = now;
                if state.tokens >= amount {
                    state.tokens -= amount;
                    return true;
                }
                (amount - state.tokens) / self.bytes_per_second as f64
            };
            thread::sleep(Duration::from_secs_f64(wait_seconds.min(0.05)));
        }
    }
}

impl UploadQuota {
    fn reserve(&self, amount: u64) -> bool {
        let mut used = self
            .used_bytes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(next) = used.checked_add(amount) else {
            return false;
        };
        if next > self.maximum_bytes {
            return false;
        }
        *used = next;
        true
    }

    fn release(&self, amount: u64) {
        let mut used = self
            .used_bytes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *used = used.saturating_sub(amount);
    }
}

static SERVER: OnceLock<Mutex<HashMap<String, RunningServer>>> = OnceLock::new();
static LIFECYCLE: OnceLock<Mutex<()>> = OnceLock::new();
static AUTH_EPOCH: AtomicU64 = AtomicU64::new(1);
static LOGIN_FAILURES: OnceLock<Mutex<HashMap<String, (u32, Instant)>>> = OnceLock::new();
static CSRF_TOKENS: OnceLock<Mutex<HashMap<(String, String), CsrfChallenge>>> = OnceLock::new();
static ACCESS_LOGS: OnceLock<Mutex<HashMap<String, Arc<Mutex<VecDeque<FileShareAccessLog>>>>>> =
    OnceLock::new();
#[cfg(test)]
static TEST_ACCOUNT_STORE_PATH: OnceLock<PathBuf> = OnceLock::new();

thread_local! {
    static ACTIVE_ACCESS_LOG: RefCell<Option<AccessLogDraft>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileShareAccessLog {
    pub request_id: String,
    pub timestamp: String,
    pub client_ip: String,
    pub method: String,
    pub path: String,
    pub protocol: String,
    pub host: String,
    pub status_code: u16,
    pub status_text: String,
    pub request_content_length: Option<u64>,
    pub content_type: String,
    pub user_agent: String,
    pub referer: String,
    pub response_content_type: String,
    pub error: String,
    pub expected_bytes: Option<u64>,
    pub received_bytes: u64,
    pub sent_bytes: u64,
    pub duration_ms: u64,
}

struct AccessLogDraft {
    entry: FileShareAccessLog,
    started: Instant,
    should_record: bool,
    logs: Arc<Mutex<VecDeque<FileShareAccessLog>>>,
}

struct AccessLogScope;

impl AccessLogScope {
    fn begin(peer_addr: SocketAddr, logs: Arc<Mutex<VecDeque<FileShareAccessLog>>>) -> Self {
        let entry = FileShareAccessLog {
            request_id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            client_ip: peer_addr.ip().to_string(),
            method: String::new(),
            path: String::new(),
            protocol: String::new(),
            host: String::new(),
            status_code: 0,
            status_text: String::new(),
            request_content_length: None,
            content_type: String::new(),
            user_agent: String::new(),
            referer: String::new(),
            response_content_type: String::new(),
            error: String::new(),
            expected_bytes: None,
            received_bytes: 0,
            sent_bytes: 0,
            duration_ms: 0,
        };
        ACTIVE_ACCESS_LOG.with(|active| {
            *active.borrow_mut() = Some(AccessLogDraft {
                entry,
                started: Instant::now(),
                should_record: false,
                logs,
            });
        });
        Self
    }
}

impl Drop for AccessLogScope {
    fn drop(&mut self) {
        let draft = ACTIVE_ACCESS_LOG.with(|active| active.borrow_mut().take());
        if let Some(mut draft) = draft.filter(|draft| draft.should_record) {
            draft.entry.duration_ms = draft
                .started
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64;
            let mut logs = draft
                .logs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while logs.len() >= MAX_ACCESS_LOG_ENTRIES {
                logs.pop_front();
            }
            logs.push_back(draft.entry);
        }
    }
}

pub fn get_access_logs(id: &str, limit: usize) -> Vec<FileShareAccessLog> {
    let buffers = ACCESS_LOGS.get_or_init(|| Mutex::new(HashMap::new()));
    let buffers = buffers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(logs) = buffers.get(id) else {
        return Vec::new();
    };
    let logs = logs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    logs.iter()
        .rev()
        .take(limit.min(MAX_ACCESS_LOG_ENTRIES))
        .cloned()
        .collect()
}

fn access_log_buffer(id: &str) -> Arc<Mutex<VecDeque<FileShareAccessLog>>> {
    ACCESS_LOGS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())))
        .clone()
}

pub fn delete_access_logs(id: &str) {
    if let Some(buffers) = ACCESS_LOGS.get() {
        buffers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
    }
}

pub fn clear_access_logs(id: &str) {
    if let Some(buffers) = ACCESS_LOGS.get() {
        let buffers = buffers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(logs) = buffers.get(id) {
            logs.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
        }
    }
}

fn update_active_access_log(update: impl FnOnce(&mut AccessLogDraft)) {
    ACTIVE_ACCESS_LOG.with(|active| {
        if let Some(draft) = active.borrow_mut().as_mut() {
            update(draft);
        }
    });
}

fn set_access_request(request: &RequestHead, target: &str, token: &str, client_ip: IpAddr) {
    update_active_access_log(|draft| {
        draft.should_record = true;
        draft.entry.client_ip = client_ip.to_string();
        draft.entry.method = request.method.clone();
        draft.entry.path = redact_share_path(target, token);
        draft.entry.protocol = request.protocol.clone();
        draft.entry.host = sanitize_access_value(&request.host, token, 256);
        draft.entry.request_content_length = request.content_length.map(|length| length as u64);
        draft.entry.content_type = sanitize_access_value(&request.content_type, token, 256);
        draft.entry.user_agent = sanitize_access_value(&request.user_agent, token, 512);
        draft.entry.referer = sanitize_access_value(&request.referer, token, 1024);
    });
}

fn redact_share_path(target: &str, token: &str) -> String {
    let path = target.split('?').next().unwrap_or_default();
    let Some(path_without_slash) = path.strip_prefix('/') else {
        return sanitize_access_value(path, token, 2048);
    };
    let (first_segment, rest) = path_without_slash
        .split_once('/')
        .unwrap_or((path_without_slash, ""));
    if first_segment == token {
        format!("/{}", sanitize_access_value(rest, token, 2048))
    } else {
        sanitize_access_value(path, token, 2048)
    }
}

fn sanitize_access_value(value: &str, token: &str, max_chars: usize) -> String {
    let value: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect();
    if token.is_empty() {
        value
    } else {
        value.replace(token, "[redacted]")
    }
}

fn set_access_expected_bytes(bytes: Option<u64>) {
    update_active_access_log(|draft| draft.entry.expected_bytes = bytes);
}

fn set_access_status(status: &str) {
    let (code, status_text) = status.split_once(' ').unwrap_or((status, ""));
    let code = code.parse::<u16>().unwrap_or_default();
    update_active_access_log(|draft| {
        draft.should_record = true;
        draft.entry.status_code = code;
        draft.entry.status_text = status_text.to_string();
    });
}

fn set_access_response_content_type(content_type: &str) {
    update_active_access_log(|draft| draft.entry.response_content_type = content_type.to_string());
}

fn set_access_error(error: &str) {
    update_active_access_log(|draft| {
        draft.should_record = true;
        draft.entry.error = sanitize_access_value(error, "", 1024)
    });
}

fn add_access_received_bytes(bytes: usize) {
    update_active_access_log(|draft| {
        draft.entry.received_bytes = draft.entry.received_bytes.saturating_add(bytes as u64)
    });
}

fn current_access_received_bytes() -> u64 {
    ACTIVE_ACCESS_LOG.with(|active| {
        active
            .borrow()
            .as_ref()
            .map(|draft| draft.entry.received_bytes)
            .unwrap_or_default()
    })
}

fn add_access_sent_bytes(bytes: usize) {
    update_active_access_log(|draft| {
        draft.entry.sent_bytes = draft.entry.sent_bytes.saturating_add(bytes as u64)
    });
}

fn server_state() -> MutexGuard<'static, HashMap<String, RunningServer>> {
    SERVER
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn lifecycle_lock() -> MutexGuard<'static, ()> {
    LIFECYCLE
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Serialize, Deserialize, Default)]
struct FileShareAccounts {
    users: HashMap<String, String>,
}

fn account_store_path() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_ACCOUNT_STORE_PATH.get() {
        return path.clone();
    }
    crate::config_manager::plugin_data_dir().join("file_share_accounts.json")
}

fn load_accounts() -> Result<FileShareAccounts, String> {
    let path = account_store_path();
    if !path.exists() {
        return Ok(FileShareAccounts::default());
    }
    let bytes = fs::read(path).map_err(|e| format!("Unable to read or write account data: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Unable to read or write account data: {e}"))
}

fn save_accounts(accounts: &FileShareAccounts) -> Result<(), String> {
    let path = account_store_path();
    let parent = path
        .parent()
        .ok_or_else(|| "Account storage path has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("Unable to read or write account data: {e}"))?;
    let bytes = serde_json::to_vec_pretty(accounts).map_err(|e| e.to_string())?;
    fs::write(path, bytes).map_err(|e| format!("Unable to read or write account data: {e}"))
}

pub fn list_file_share_users() -> Result<Vec<String>, String> {
    let mut users = load_accounts()?.users.into_keys().collect::<Vec<_>>();
    users.sort_by_key(|username| username.to_lowercase());
    Ok(users)
}

pub fn add_file_share_user(username: &str, password: &str) -> Result<(), String> {
    let username = validate_username(username)?;
    validate_password(password)?;
    let mut accounts = load_accounts()?;
    if accounts.users.contains_key(&username) {
        return Err("Username already exists".into());
    }
    accounts.users.insert(username, hash_password(password)?);
    save_accounts(&accounts)?;
    Ok(())
}

pub fn set_file_share_user_password(username: &str, password: &str) -> Result<(), String> {
    let username = validate_username(username)?;
    validate_password(password)?;
    let mut accounts = load_accounts()?;
    let hash = accounts
        .users
        .get_mut(&username)
        .ok_or_else(|| "Account not found".to_string())?;
    *hash = hash_password(password)?;
    save_accounts(&accounts)?;
    AUTH_EPOCH.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

pub fn delete_file_share_user(username: &str) -> Result<(), String> {
    let mut accounts = load_accounts()?;
    if accounts.users.remove(username).is_none() {
        return Err("Account not found".into());
    }
    save_accounts(&accounts)?;
    AUTH_EPOCH.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

fn validate_username(username: &str) -> Result<String, String> {
    let username = username.trim();
    if !(3..=48).contains(&username.len())
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(
            "Username must be 3-48 characters using letters, digits, dot, underscore, or hyphen"
                .into(),
        );
    }
    Ok(username.to_string())
}

fn validate_password(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("Password cannot be empty".into());
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = uuid::Uuid::new_v4().into_bytes();
    let mut hash = [0_u8; PASSWORD_HASH_BYTES];
    pbkdf2::derive(
        PBKDF2_HMAC_SHA256,
        std::num::NonZeroU32::new(PASSWORD_HASH_ITERATIONS)
            .expect("password hash iteration count is non-zero"),
        &salt,
        password.as_bytes(),
        &mut hash,
    );
    Ok(format!(
        "pbkdf2-sha256${}${}${}",
        PASSWORD_HASH_ITERATIONS,
        hex_encode(&salt),
        hex_encode(&hash)
    ))
}

fn verify_user_password(username: &str, password: &str) -> Result<bool, String> {
    let Some(hash) = load_accounts()?.users.get(username).cloned() else {
        return Ok(false);
    };
    Ok(password_hash_matches(&hash, password))
}

fn password_hash_matches(hash: &str, password: &str) -> bool {
    let mut parts = hash.split('$');
    let expected_iterations = PASSWORD_HASH_ITERATIONS.to_string();
    if parts.next() != Some("pbkdf2-sha256") || parts.next() != Some(expected_iterations.as_str()) {
        return false;
    }
    let Some(salt) = parts.next().and_then(hex_decode) else {
        return false;
    };
    let Some(expected_hash) = parts.next().and_then(hex_decode) else {
        return false;
    };
    if parts.next().is_some()
        || salt.len() != PASSWORD_HASH_SALT_BYTES
        || expected_hash.len() != PASSWORD_HASH_BYTES
    {
        return false;
    }
    pbkdf2::verify(
        PBKDF2_HMAC_SHA256,
        std::num::NonZeroU32::new(PASSWORD_HASH_ITERATIONS)
            .expect("password hash iteration count is non-zero"),
        &salt,
        password.as_bytes(),
        &expected_hash,
    )
    .is_ok()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

fn session_cookie_name(instance_id: &str) -> String {
    let id_hash = digest(&SHA256, instance_id.as_bytes());
    format!("swfs_{}", hex_encode(&id_hash.as_ref()[..12]))
}

fn csrf_cookie_name(instance_id: &str) -> String {
    let id_hash = digest(&SHA256, instance_id.as_bytes());
    format!("swcsrf_{}", hex_encode(&id_hash.as_ref()[..12]))
}

fn issue_csrf_token(cookie_name: &str, client_ip: IpAddr, user_agent: &str) -> String {
    let token = uuid::Uuid::new_v4().simple().to_string();
    let now = Instant::now();
    let mut tokens = CSRF_TOKENS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    tokens.retain(|_, challenge| challenge.expires > now);
    if tokens.len() >= 8192 {
        if let Some(oldest) = tokens.keys().next().cloned() {
            tokens.remove(&oldest);
        }
    }
    tokens.insert(
        (cookie_name.to_string(), token.clone()),
        CsrfChallenge {
            expires: now + Duration::from_secs(12 * 60 * 60),
            client_ip,
            user_agent: user_agent.chars().take(512).collect(),
        },
    );
    token
}

fn csrf_token_matches(
    cookie: &str,
    cookie_name: &str,
    submitted: &str,
    client_ip: IpAddr,
    user_agent: &str,
) -> bool {
    if submitted.len() != 32 || !submitted.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    if let Some(expected) = session_token_from_cookie(cookie, cookie_name) {
        let mut difference = expected.len() ^ submitted.len();
        for (expected, submitted) in expected.bytes().zip(submitted.bytes()) {
            difference |= usize::from(expected ^ submitted);
        }
        if difference == 0 {
            return true;
        }
    }
    let now = Instant::now();
    let mut tokens = CSRF_TOKENS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    tokens.retain(|_, challenge| challenge.expires > now);
    tokens
        .get(&(cookie_name.to_string(), submitted.to_string()))
        .is_some_and(|challenge| {
            challenge.client_ip == client_ip
                && challenge.user_agent == user_agent.chars().take(512).collect::<String>()
        })
}

fn find_session(
    cookie: &str,
    cookie_name: &str,
    sessions: &Arc<Mutex<HashMap<String, LoginSession>>>,
) -> Option<LoginSession> {
    let token = session_token_from_cookie(cookie, cookie_name)?;
    let mut sessions = sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = Instant::now();
    sessions.retain(|_, session| {
        session.expires > now && session.auth_epoch == AUTH_EPOCH.load(Ordering::Relaxed)
    });
    sessions.get(token).cloned()
}

fn session_token_from_cookie<'a>(cookie: &'a str, cookie_name: &str) -> Option<&'a str> {
    cookie
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == cookie_name).then_some(value))
}

fn login_rate_limited(ip: &str) -> bool {
    LOGIN_FAILURES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(ip)
        .is_some_and(|(count, last)| *count >= 5 && last.elapsed() < Duration::from_secs(5 * 60))
}

fn record_login_failure(ip: &str) {
    let mut failures = LOGIN_FAILURES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = failures
        .entry(ip.to_string())
        .or_insert((0, Instant::now()));
    if entry.1.elapsed() > Duration::from_secs(15 * 60) {
        *entry = (0, Instant::now());
    }
    entry.0 = entry.0.saturating_add(1);
    entry.1 = Instant::now();
}

fn clear_login_failures(ip: &str) {
    if let Some(failures) = LOGIN_FAILURES.get() {
        failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(ip);
    }
}

fn same_origin_url(url: &str, host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    let Some((scheme, remainder)) = url.trim().split_once("://") else {
        return false;
    };
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return false;
    }
    let authority = remainder
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return false;
    }

    let default_port = if scheme.eq_ignore_ascii_case("https") {
        "443"
    } else {
        "80"
    };
    normalize_authority(authority, default_port)
        .zip(normalize_authority(host, default_port))
        .is_some_and(|(origin, request)| origin == request)
}

fn normalize_authority(authority: &str, default_port: &str) -> Option<String> {
    let (host, port) = if authority.starts_with('[') {
        let end = authority.find(']')?;
        let host = &authority[..=end];
        let remainder = &authority[end + 1..];
        let port = if remainder.is_empty() {
            None
        } else {
            Some(
                remainder
                    .strip_prefix(':')?
                    .parse::<u16>()
                    .ok()?
                    .to_string(),
            )
        };
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => {
                (host, Some(port.parse::<u16>().ok()?.to_string()))
            }
            Some(_) => return None,
            None => (authority, None),
        }
    };
    if host.is_empty()
        || host.bytes().any(|byte| byte.is_ascii_whitespace())
        || host.contains('/')
        || host.contains('\\')
    {
        return None;
    }
    let host = host.to_ascii_lowercase();
    match port {
        Some(port) if port != default_port => Some(format!("{host}:{port}")),
        _ => Some(host),
    }
}

fn same_origin_request(origin: &str, referer: &str, fetch_site: &str, host: &str) -> bool {
    if fetch_site.trim().eq_ignore_ascii_case("same-origin") {
        // The browser derives this header from the page and target origins.
        // Prefer it when a proxy has rewritten Host or removed Origin.
        return true;
    }
    (!origin.trim().is_empty() && same_origin_url(origin, host))
        || (!referer.trim().is_empty() && same_origin_url(referer, host))
}

fn parse_form(body: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(body)
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((url_decode(key)?, url_decode(value)?))
        })
        .collect()
}

fn url_decode(value: &str) -> Option<String> {
    let value = value.replace('+', " ");
    decode_segment(&value)
}

fn path_prefix(token: &str, require_login: bool) -> String {
    if require_login {
        String::new()
    } else {
        format!("/{token}")
    }
}

fn secure_cookie_attribute(secure_transport: bool) -> &'static str {
    if secure_transport {
        "; Secure"
    } else {
        ""
    }
}

fn send_login_page<S: HttpStream>(
    stream: S,
    status: &str,
    message: &str,
    csrf_cookie_name: &str,
    client_ip: IpAddr,
    user_agent: &str,
    head_only: bool,
    secure_transport: bool,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut body = String::from("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"theme-color\" content=\"#0e1420\">");
    body.push_str(THEME_BOOTSTRAP_SCRIPT);
    body.push_str("<title>&#x767B;&#x5F55;&#x6587;&#x4EF6;&#x670D;&#x52A1;</title><style>");
    body.push_str(LOGIN_PAGE_STYLE);
    body.push_str("</style></head><body>");
    body.push_str(THEME_TOGGLE_BUTTON);
    body.push_str("<main><h1>&#x767B;&#x5F55;&#x6587;&#x4EF6;&#x670D;&#x52A1;</h1><p>");
    body.push_str(&html_escape(message));
    body.push_str("</p><form method=\"post\" action=\"/__auth/login\"><label>&#x7528;&#x6237;&#x540D;</label><input name=\"username\" autocomplete=\"username\" required><label>&#x5BC6;&#x7801;</label><input type=\"password\" name=\"password\" autocomplete=\"current-password\" required><button type=\"submit\">&#x767B;&#x5F55;</button></form></main>");
    body.push_str(THEME_TOGGLE_SCRIPT);
    body.push_str("</body></html>");
    let csrf_token = issue_csrf_token(csrf_cookie_name, client_ip, user_agent);
    let body = body.replace(
        "<form method=\"post\" action=\"/__auth/login\">",
        &format!(
            "<form method=\"post\" action=\"/__auth/login\"><input type=\"hidden\" name=\"csrf_token\" value=\"{csrf_token}\">"
        ),
    );
    let cookie = format!(
        "{csrf_cookie_name}={csrf_token}; Path=/; Max-Age=43200; HttpOnly; SameSite=Strict{}",
        secure_cookie_attribute(secure_transport)
    );
    send_response_with_headers(
        stream,
        status,
        "text/html; charset=utf-8",
        body.as_bytes(),
        head_only,
        stop,
        &[("Set-Cookie", &cookie)],
    )
}

pub fn is_running(id: &str) -> bool {
    server_state()
        .get(id)
        .is_some_and(|server| !server.stop.load(Ordering::Relaxed))
}

pub fn lan_is_running(id: &str) -> bool {
    server_state()
        .get(id)
        .is_some_and(|server| server.lan_enabled && !server.stop.load(Ordering::Relaxed))
}

pub fn lan_host(id: &str) -> Option<String> {
    server_state()
        .get(id)
        .filter(|server| !server.stop.load(Ordering::Relaxed))
        .and_then(|server| server.lan_host.clone())
}

pub fn detect_lan_ipv4() -> Option<String> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(address)
            if !address.is_unspecified() && !address.is_loopback() && !address.is_link_local() =>
        {
            Some(address.to_string())
        }
        _ => None,
    }
}

fn detect_lan_networks() -> Vec<IpNetwork> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let output = std::process::Command::new("route.exe")
            .args(["print", "-4"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output()
            .ok();
        let Some(output) = output.filter(|output| output.status.success()) else {
            return Vec::new();
        };
        return parse_on_link_routes(&String::from_utf8_lossy(&output.stdout));
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn parse_on_link_routes(output: &str) -> Vec<IpNetwork> {
    let mut networks = Vec::new();
    for line in output.lines() {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() < 4 {
            continue;
        }
        let Ok(address) = columns[0].parse::<Ipv4Addr>() else {
            continue;
        };
        let Ok(mask) = columns[1].parse::<Ipv4Addr>() else {
            continue;
        };
        if columns[2].parse::<IpAddr>().is_ok() {
            continue;
        }
        let Ok(interface) = columns[3].parse::<Ipv4Addr>() else {
            continue;
        };
        if address.is_unspecified()
            || address.is_loopback()
            || address.is_link_local()
            || address.is_multicast()
            || interface.is_unspecified()
            || interface.is_loopback()
            || interface.is_link_local()
        {
            continue;
        }
        let mask = u32::from(mask);
        let prefix = mask.leading_ones() as u8;
        let expected_mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        if mask != expected_mask
            || u32::from(interface) & expected_mask != u32::from(address) & expected_mask
        {
            continue;
        }
        let network = IpNetwork {
            address: IpAddr::V4(address),
            prefix,
        };
        if !networks.iter().any(|existing: &IpNetwork| {
            existing.address == network.address && existing.prefix == network.prefix
        }) {
            networks.push(network);
        }
    }
    networks
}

pub fn stop() {
    let _lifecycle = lifecycle_lock();
    stop_all_inner();
}

pub fn stop_id(id: &str) {
    let _lifecycle = lifecycle_lock();
    stop_inner(id);
}

fn stop_inner(id: &str) {
    let server = server_state().remove(id);
    stop_server(server);
}

fn stop_all_inner() {
    let servers = std::mem::take(&mut *server_state());
    for server in servers.into_values() {
        stop_server(Some(server));
    }
}

fn stop_server(server: Option<RunningServer>) {
    if let Some(server) = server {
        server.stop.store(true, Ordering::Relaxed);
        server
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let mut clients = server
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (_, client) in clients.iter() {
            let _ = client.shutdown(Shutdown::Both);
        }
        clients.clear();
        drop(clients);
        let _ = server.thread.join();
    }
}

fn der_wrap(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut output = vec![tag];
    if value.len() < 128 {
        output.push(value.len() as u8);
    } else {
        let length_bytes = value.len().to_be_bytes();
        let first_nonzero = length_bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(length_bytes.len() - 1);
        let length_bytes = &length_bytes[first_nonzero..];
        output.push(0x80 | length_bytes.len() as u8);
        output.extend_from_slice(length_bytes);
    }
    output.extend_from_slice(value);
    output
}

fn der_sequence(fields: &[Vec<u8>]) -> Vec<u8> {
    let content = fields.concat();
    der_wrap(0x30, &content)
}

fn der_oid(arcs: &[u64]) -> Vec<u8> {
    let Some((&first, remainder)) = arcs.split_first() else {
        return der_wrap(0x06, &[]);
    };
    if remainder.is_empty() {
        return der_wrap(0x06, &[]);
    }
    let mut encoded = Vec::new();
    for arc in std::iter::once(&(first * 40 + remainder[0])).chain(remainder.iter().skip(1)) {
        let mut stack = [0u8; 10];
        let mut index = stack.len();
        let mut value = *arc;
        index -= 1;
        stack[index] = (value & 0x7f) as u8;
        value >>= 7;
        while value > 0 {
            index -= 1;
            stack[index] = ((value & 0x7f) as u8) | 0x80;
            value >>= 7;
        }
        encoded.extend_from_slice(&stack[index..]);
    }
    der_wrap(0x06, &encoded)
}

fn algorithm_identifier(oid: &[u64], parameters: &[Vec<u8>]) -> Vec<u8> {
    let mut fields = vec![der_oid(oid)];
    fields.extend_from_slice(parameters);
    der_sequence(&fields)
}

fn certificate_extension(oid: &[u64], value: &[u8]) -> Vec<u8> {
    der_sequence(&[der_oid(oid), der_wrap(0x04, value)])
}

fn self_signed_lan_certificate(san: &str, private_key_bytes: &[u8]) -> Result<Vec<u8>, String> {
    use ring::rand::SecureRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};

    let rng = ring::rand::SystemRandom::new();
    let key_pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, private_key_bytes, &rng)
            .map_err(|_| "Unable to load LAN TLS private key".to_string())?;
    let mut serial = [0u8; 20];
    rng.fill(&mut serial)
        .map_err(|_| "Unable to create LAN TLS certificate serial".to_string())?;
    serial[0] &= 0x7f;
    if serial.iter().all(|byte| *byte == 0) {
        serial[19] = 1;
    }

    let common_name = der_sequence(&[der_set(&[der_sequence(&[
        der_oid(&[2, 5, 4, 3]),
        der_wrap(0x0c, b"SideWeave LAN File Service"),
    ])])]);
    let now = chrono::Utc::now();
    let valid_until = now + chrono::Duration::days(3650);
    let validity = der_sequence(&[
        der_wrap(0x17, now.format("%y%m%d%H%M%SZ").to_string().as_bytes()),
        der_wrap(
            0x17,
            valid_until.format("%y%m%d%H%M%SZ").to_string().as_bytes(),
        ),
    ]);

    let san_value = if let Ok(address) = san.parse::<IpAddr>() {
        match address {
            IpAddr::V4(address) => der_wrap(0x87, &address.octets()),
            IpAddr::V6(address) => der_wrap(0x87, &address.octets()),
        }
    } else {
        der_wrap(0x82, san.as_bytes())
    };
    let subject_alt_names = der_sequence(&[san_value]);
    let extensions = der_wrap(
        0xa3,
        &der_sequence(&[
            certificate_extension(&[2, 5, 29, 17], &subject_alt_names),
            certificate_extension(&[2, 5, 29, 19], &der_sequence(&[])),
            certificate_extension(&[2, 5, 29, 15], &der_wrap(0x03, &[7, 0x80])),
            certificate_extension(
                &[2, 5, 29, 37],
                &der_sequence(&[der_oid(&[1, 3, 6, 1, 5, 5, 7, 3, 1])]),
            ),
        ]),
    );
    let signature_algorithm = algorithm_identifier(&[1, 2, 840, 10045, 4, 3, 2], &[]);
    let public_key_algorithm = algorithm_identifier(
        &[1, 2, 840, 10045, 2, 1],
        &[der_oid(&[1, 2, 840, 10045, 3, 1, 7])],
    );
    let subject_public_key_info = der_sequence(&[
        public_key_algorithm,
        der_wrap(0x03, &[&[0][..], key_pair.public_key().as_ref()].concat()),
    ]);
    let tbs_certificate = der_sequence(&[
        der_wrap(0xa0, &der_wrap(0x02, &[2])),
        der_wrap(0x02, &serial),
        signature_algorithm.clone(),
        common_name.clone(),
        validity,
        common_name,
        subject_public_key_info,
        extensions,
    ]);
    let signature = key_pair
        .sign(&rng, &tbs_certificate)
        .map_err(|_| "Unable to sign LAN TLS certificate".to_string())?;
    Ok(der_sequence(&[
        tbs_certificate,
        signature_algorithm,
        der_wrap(0x03, &[&[0][..], signature.as_ref()].concat()),
    ]))
}

fn der_set(fields: &[Vec<u8>]) -> Vec<u8> {
    let content = fields.concat();
    der_wrap(0x31, &content)
}

fn lan_tls_config(
    instance_id: &str,
    lan_host: Option<&str>,
) -> Result<Arc<rustls::ServerConfig>, String> {
    let san = lan_host
        .filter(|host| !host.is_empty())
        .unwrap_or("localhost");
    let identity = format!("{instance_id}\n{san}");
    let digest = digest(&SHA256, identity.as_bytes());
    let file_key = digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let directory = crate::config_manager::plugin_data_dir();
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Unable to create TLS certificate directory: {error}"))?;
    let certificate_path = directory.join(format!("lan-{file_key}.cert.der"));
    let private_key_path = directory.join(format!("lan-{file_key}.key.der"));

    let (certificate_bytes, private_key_bytes) =
        if certificate_path.is_file() && private_key_path.is_file() {
            (
                fs::read(&certificate_path)
                    .map_err(|error| format!("Unable to read LAN TLS certificate: {error}"))?,
                fs::read(&private_key_path)
                    .map_err(|error| format!("Unable to read LAN TLS private key: {error}"))?,
            )
        } else {
            use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
            let generated = EcdsaKeyPair::generate_pkcs8(
                &ECDSA_P256_SHA256_ASN1_SIGNING,
                &ring::rand::SystemRandom::new(),
            )
            .map_err(|_| "Unable to generate LAN TLS private key".to_string())?;
            let private_key = generated.as_ref().to_vec();
            let certificate = self_signed_lan_certificate(san, &private_key)?;
            fs::write(&certificate_path, &certificate)
                .map_err(|error| format!("Unable to save LAN TLS certificate: {error}"))?;
            fs::write(&private_key_path, &private_key)
                .map_err(|error| format!("Unable to save LAN TLS private key: {error}"))?;
            (certificate, private_key)
        };

    let certificate = CertificateDer::from(certificate_bytes);
    let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key_bytes));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("Unable to configure LAN TLS: {error}"))?;
    let mut config = builder
        .with_no_client_auth()
        .with_single_cert(vec![certificate], private_key)
        .map_err(|error| format!("Unable to configure LAN TLS certificate: {error}"))?;
    config.alpn_protocols.push(b"http/1.1".to_vec());
    Ok(Arc::new(config))
}

pub fn start(config: &mut FileShareConfig) -> Result<(), String> {
    let _lifecycle = lifecycle_lock();
    if !config.enabled {
        stop_inner(&config.id);
        return Ok(());
    }
    if config.id.trim().is_empty() {
        return Err("Instance ID is required".into());
    }
    if config.port < 1024 {
        return Err("Port must be between 1024 and 65535".into());
    }
    if !(64..=8192).contains(&config.media_preview_limit_kbps) {
        return Err("Media preview speed must be between 64 and 8192 KB/s".into());
    }
    if !config.allow_view && !config.allow_upload {
        return Err("At least one of viewing or uploading must be allowed".into());
    }
    if config.allow_view && config.view_directory.trim().is_empty() {
        return Err("A view directory is required".into());
    }
    if config.allow_upload && config.receive_directory.trim().is_empty() && !config.allow_view {
        return Err("A receive directory is required when viewing is disabled".into());
    }
    let root = if config.allow_view {
        let root = fs::canonicalize(&config.view_directory)
            .map_err(|e| format!("Unable to access view directory: {e}"))?;
        if !root.is_dir() {
            return Err("Configured path is not a directory".into());
        }
        config.view_directory = root.to_string_lossy().into_owned();
        Some(root)
    } else {
        None
    };
    if config.token.len() != 32 || !config.token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        config.token = uuid::Uuid::new_v4().simple().to_string();
    }
    validate_allowed_ips(&config.allowed_ips)?;

    let upload_directory = if config.allow_upload {
        let configured = if config.receive_directory.trim().is_empty() {
            let view = root
                .as_ref()
                .ok_or_else(|| "A receive directory is required".to_string())?;
            let name = view
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    "Unable to derive a default receive directory; choose one manually".to_string()
                })?;
            view.with_file_name(format!("{name}_uploads"))
        } else {
            PathBuf::from(config.receive_directory.trim())
        };
        fs::create_dir_all(&configured)
            .map_err(|e| format!("Unable to create receive directory: {e}"))?;
        let canonical = fs::canonicalize(&configured)
            .map_err(|e| format!("Unable to access receive directory: {e}"))?;
        if !canonical.is_dir() {
            return Err("Configured path is not a directory".into());
        }
        config.receive_directory = canonical.to_string_lossy().into_owned();
        Some(canonical)
    } else {
        None
    };
    let upload_quota = if let Some(directory) = upload_directory.as_deref() {
        let used_bytes = directory_upload_size(directory)
            .map_err(|e| format!("Unable to calculate receive directory usage: {e}"))?;
        let maximum_bytes = u64::from(config.max_total_upload_mb.clamp(1, 8192)) * 1024 * 1024;
        Some(Arc::new(UploadQuota {
            used_bytes: Mutex::new(used_bytes),
            maximum_bytes,
        }))
    } else {
        None
    };
    let media_rate_limiter = Arc::new(MediaRateLimiter::new(
        u64::from(config.media_preview_limit_kbps) * 1024,
    ));
    if config.require_login && list_file_share_users()?.is_empty() {
        return Err("Add a shared login account before starting a login-protected instance".into());
    }

    let lan_host = if config.lan_enabled {
        detect_lan_ipv4()
    } else {
        None
    };
    let lan_tls = if config.lan_enabled {
        Some(lan_tls_config(&config.id, lan_host.as_deref())?)
    } else {
        None
    };
    let lan_networks = if config.lan_enabled {
        detect_lan_networks()
    } else {
        Vec::new()
    };
    stop_inner(&config.id);
    let bind_address = if config.lan_enabled {
        Ipv4Addr::UNSPECIFIED
    } else {
        Ipv4Addr::LOCALHOST
    };
    let listener = TcpListener::bind(SocketAddr::from((bind_address, config.port)))
        .map_err(|e| format!("Unable to listen on {bind_address}:{}: {e}", config.port))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Unable to configure file service: {e}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let logs = access_log_buffer(&config.id);
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let thread_root = root.clone();
    let thread_token = config.token.clone();
    let thread_upload_directory = upload_directory.clone();
    let thread_allowed_ips = parse_allowed_ips(&config.allowed_ips)?;
    let thread_clients = clients.clone();
    let thread_logs = logs.clone();
    let thread_sessions = sessions.clone();
    let thread_stop = stop.clone();
    let thread_id = config.id.clone();
    let thread_allow_view = config.allow_view;
    let thread_allow_upload = config.allow_upload;
    let thread_require_login = config.require_login;
    let max_upload_bytes = u64::from(config.max_upload_mb.clamp(1, 1024)) * 1024 * 1024;
    let handle = thread::Builder::new()
        .name(format!("file-share-{}", config.id))
        .spawn(move || {
            serve_loop(
                listener,
                thread_id,
                thread_root,
                thread_token,
                thread_upload_directory,
                thread_allow_view,
                thread_allow_upload,
                thread_require_login,
                max_upload_bytes,
                upload_quota,
                media_rate_limiter,
                thread_allowed_ips,
                lan_networks,
                lan_tls,
                thread_stop,
                thread_clients,
                Arc::new(AtomicUsize::new(1)),
                thread_logs,
                thread_sessions,
            )
        })
        .map_err(|e| format!("Unable to start file service thread: {e}"))?;
    server_state().insert(
        config.id.clone(),
        RunningServer {
            lan_host,
            lan_enabled: config.lan_enabled,
            stop,
            clients,
            thread: handle,
            sessions,
        },
    );
    Ok(())
}

fn serve_loop(
    listener: TcpListener,
    instance_id: String,
    root: Option<PathBuf>,
    token: String,
    upload_directory: Option<PathBuf>,
    allow_view: bool,
    allow_upload: bool,
    require_login: bool,
    max_upload_bytes: u64,
    upload_quota: Option<Arc<UploadQuota>>,
    media_rate_limiter: Arc<MediaRateLimiter>,
    allowed_ips: Vec<IpNetwork>,
    lan_networks: Vec<IpNetwork>,
    lan_tls_config: Option<Arc<rustls::ServerConfig>>,
    stop: Arc<AtomicBool>,
    clients: Arc<Mutex<Vec<(usize, TcpStream)>>>,
    client_ids: Arc<AtomicUsize>,
    logs: Arc<Mutex<VecDeque<FileShareAccessLog>>>,
    sessions: Arc<Mutex<HashMap<String, LoginSession>>>,
) {
    let active = Arc::new(AtomicUsize::new(0));
    let mut workers: Vec<JoinHandle<()>> = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                let worker = workers.swap_remove(index);
                let _ = worker.join();
            } else {
                index += 1;
            }
        }

        let accepted = match listener.accept() {
            Ok((stream, peer_addr)) => Some((stream, peer_addr)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => None,
            Err(_) => {
                thread::sleep(Duration::from_millis(100));
                None
            }
        };

        if let Some((stream, peer_addr)) = accepted {
            // On Windows, accepted sockets can inherit the listener's nonblocking
            // mode. Request body reads then fail with WSAEWOULDBLOCK (10035) as
            // soon as the currently buffered data has been consumed. Keep the
            // accept loop nonblocking, but handle each client with blocking I/O.
            if stream.set_nonblocking(false).is_err() {
                continue;
            }

            let request_stream = if !peer_addr.ip().is_loopback() {
                let Some(tls_config) = lan_tls_config.as_ref() else {
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                };
                let Ok(connection) = rustls::ServerConnection::new(tls_config.clone()) else {
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                };
                ClientStream::Tls(rustls::StreamOwned::new(connection, stream))
            } else {
                ClientStream::Plain(stream)
            };
            let secure_transport = matches!(&request_stream, ClientStream::Tls(_));

            if active.fetch_add(1, Ordering::Relaxed) >= MAX_ACTIVE_REQUESTS {
                active.fetch_sub(1, Ordering::Relaxed);
                let _ = request_stream.set_write_timeout(Some(CLIENT_READ_TIMEOUT));
                let _ = send_response(
                    request_stream,
                    "503 Service Unavailable",
                    "text/plain; charset=utf-8",
                    b"Too many requests",
                    false,
                    &stop,
                );
                continue;
            }

            let client_id = client_ids.fetch_add(1, Ordering::Relaxed);
            let client_socket = match &request_stream {
                ClientStream::Plain(stream) => stream,
                ClientStream::Tls(stream) => stream.get_ref(),
            };
            let Ok(shutdown_stream) = client_socket.try_clone() else {
                active.fetch_sub(1, Ordering::Relaxed);
                continue;
            };
            {
                let mut registered = clients
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if stop.load(Ordering::Relaxed) {
                    active.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
                registered.push((client_id, shutdown_stream));
            }

            let request_root = root.clone();
            let request_instance_id = instance_id.clone();
            let request_token = token.clone();
            let request_upload_directory = upload_directory.clone();
            let request_allowed_ips = allowed_ips.clone();
            let request_lan_networks = lan_networks.clone();
            let request_stop = stop.clone();
            let request_active = active.clone();
            let request_clients = clients.clone();
            let request_max_upload_bytes = max_upload_bytes;
            let request_upload_quota = upload_quota.clone();
            let request_media_rate_limiter = media_rate_limiter.clone();
            let request_logs = logs.clone();
            let request_sessions = sessions.clone();
            match thread::Builder::new()
                .name(format!("file-share-request-{}", instance_id))
                .spawn(move || {
                    let _active = ActiveRequest {
                        active: request_active,
                        clients: request_clients,
                        client_id,
                    };
                    let _ = serve_request(
                        request_stream,
                        peer_addr,
                        secure_transport,
                        &request_instance_id,
                        request_root.as_deref(),
                        &request_token,
                        request_upload_directory.as_deref(),
                        allow_view,
                        allow_upload,
                        require_login,
                        request_logs,
                        request_sessions,
                        request_max_upload_bytes,
                        request_upload_quota.as_deref(),
                        &request_media_rate_limiter,
                        &request_allowed_ips,
                        &request_lan_networks,
                        &request_stop,
                    );
                }) {
                Ok(worker) => workers.push(worker),
                Err(_) => {
                    active.fetch_sub(1, Ordering::Relaxed);
                    remove_client(&clients, client_id);
                }
            }
        } else {
            thread::sleep(Duration::from_millis(40));
        }
    }

    for worker in workers {
        let _ = worker.join();
    }
}

struct ActiveRequest {
    active: Arc<AtomicUsize>,
    clients: Arc<Mutex<Vec<(usize, TcpStream)>>>,
    client_id: usize,
}

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::Relaxed);
        remove_client(&self.clients, self.client_id);
    }
}

fn remove_client(clients: &Mutex<Vec<(usize, TcpStream)>>, client_id: usize) {
    clients
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|(registered_id, _)| *registered_id != client_id);
}

fn serve_request<S: HttpStream>(
    mut stream: S,
    peer_addr: SocketAddr,
    secure_transport: bool,
    instance_id: &str,
    root: Option<&Path>,
    token: &str,
    upload_directory: Option<&Path>,
    allow_view: bool,
    allow_upload: bool,
    require_login: bool,
    logs: Arc<Mutex<VecDeque<FileShareAccessLog>>>,
    sessions: Arc<Mutex<HashMap<String, LoginSession>>>,
    max_upload_bytes: u64,
    upload_quota: Option<&UploadQuota>,
    media_rate_limiter: &MediaRateLimiter,
    allowed_ips: &[IpNetwork],
    lan_networks: &[IpNetwork],
    stop: &AtomicBool,
) -> io::Result<()> {
    let _access_log = AccessLogScope::begin(peer_addr, logs);
    stream.set_read_timeout(Some(CLIENT_READ_TIMEOUT))?;
    stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT))?;
    let request = match read_request_head(&mut stream) {
        Ok(request) => request,
        Err(RequestReadError::TooLarge) => {
            return send_response(
                stream,
                "431 Request Header Fields Too Large",
                "text/plain; charset=utf-8",
                b"Request headers too large",
                false,
                stop,
            )
        }
        Err(RequestReadError::Io(error)) => {
            set_access_error(&format!("Request read failed: {error}"));
            return Err(error);
        }
        Err(RequestReadError::ConnectionClosed) => return Ok(()),
        Err(RequestReadError::Malformed) => {
            return send_response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                b"Bad request",
                false,
                stop,
            )
        }
    };
    let is_loopback = peer_addr.ip().is_loopback();
    let client_ip = if is_loopback {
        request.cf_connecting_ip.unwrap_or(peer_addr.ip())
    } else {
        peer_addr.ip()
    };
    set_access_request(&request, &request.target, token, client_ip);
    let explicitly_allowed = !allowed_ips.is_empty()
        && allowed_ips
            .iter()
            .any(|network| network.contains(client_ip));
    let is_lan_peer = matches!(peer_addr.ip(), IpAddr::V4(address) if address.is_private())
        || lan_networks
            .iter()
            .any(|network| network.contains(peer_addr.ip()));
    if !is_loopback && !is_lan_peer && !explicitly_allowed {
        return send_response(
            stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            b"LAN access only",
            request.method == "HEAD",
            stop,
        );
    }
    if !allowed_ips.is_empty() && !explicitly_allowed {
        return send_response(
            stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            b"Access denied",
            request.method == "HEAD",
            stop,
        );
    }

    let raw_path = request.target.split('?').next().unwrap_or("/");
    let cookie_name = session_cookie_name(instance_id);
    let csrf_name = csrf_cookie_name(instance_id);
    if require_login && raw_path == "/__auth/login" && request.method == "POST" {
        let ip_key = client_ip.to_string();
        if login_rate_limited(&ip_key) {
            return send_response(
                stream,
                "429 Too Many Requests",
                "text/plain; charset=utf-8",
                b"Too many login attempts; try again later",
                false,
                stop,
            );
        }
        let Some(length) = request
            .content_length
            .filter(|length| *length <= 1024 * 1024)
        else {
            return send_response(
                stream,
                "413 Content Too Large",
                "text/plain; charset=utf-8",
                b"Invalid login request",
                false,
                stop,
            );
        };
        if !request
            .content_type
            .to_ascii_lowercase()
            .starts_with("application/x-www-form-urlencoded")
        {
            return send_response(
                stream,
                "415 Unsupported Media Type",
                "text/plain; charset=utf-8",
                b"Form encoding required",
                false,
                stop,
            );
        }
        let mut body = vec![0u8; length];
        if stream.read_exact(&mut body).is_err() {
            return send_response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                b"Invalid login request",
                false,
                stop,
            );
        }
        let form = parse_form(&body);
        let same_origin = same_origin_request(
            &request.origin,
            &request.referer,
            &request.fetch_site,
            &request.host,
        );
        let csrf_valid = form
            .get("csrf_token")
            .is_some_and(|token| {
                csrf_token_matches(
                    &request.cookie,
                    &csrf_name,
                    token,
                    client_ip,
                    &request.user_agent,
                )
            });
        if !same_origin && !csrf_valid {
            return send_response(
                stream,
                "403 Forbidden",
                "text/plain; charset=utf-8",
                b"Cross-origin request rejected",
                false,
                stop,
            );
        }
        let username = form.get("username").cloned().unwrap_or_default();
        let password = form.get("password").cloned().unwrap_or_default();
        if verify_user_password(&username, &password).unwrap_or(false) {
            clear_login_failures(&ip_key);
            let session_token = uuid::Uuid::new_v4().simple().to_string();
            sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    session_token.clone(),
                    LoginSession {
                        expires: Instant::now() + Duration::from_secs(12 * 60 * 60),
                        auth_epoch: AUTH_EPOCH.load(Ordering::Relaxed),
                    },
                );
            let cookie = format!(
                "{cookie_name}={session_token}; Path=/; Max-Age=43200; HttpOnly; SameSite=Strict{}",
                secure_cookie_attribute(secure_transport)
            );
            let csrf_cookie = format!(
                "{csrf_name}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict{}",
                secure_cookie_attribute(secure_transport)
            );
            return send_response_with_headers(
                stream,
                "303 See Other",
                "text/plain; charset=utf-8",
                b"Signed in",
                false,
                stop,
                &[
                    ("Location", "/"),
                    ("Set-Cookie", &cookie),
                    ("Set-Cookie", &csrf_cookie),
                ],
            );
        }
        record_login_failure(&ip_key);
        return send_login_page(
            stream,
            "401 Unauthorized",
            "Invalid username or password",
            &csrf_name,
            client_ip,
            &request.user_agent,
            false,
            secure_transport,
            stop,
        );
    }

    let session = if require_login {
        find_session(&request.cookie, &cookie_name, &sessions)
    } else {
        None
    };
    if require_login && raw_path == "/__auth/logout" && request.method == "POST" {
        let same_origin = same_origin_request(
            &request.origin,
            &request.referer,
            &request.fetch_site,
            &request.host,
        );
        let csrf_valid = if same_origin {
            true
        } else if let Some(length) = request.content_length.filter(|length| *length <= 1024) {
            let mut body = vec![0_u8; length];
            if stream.read_exact(&mut body).is_ok() {
                let form = parse_form(&body);
                form.get("csrf_token")
                    .is_some_and(|token| {
                        csrf_token_matches(
                            &request.cookie,
                            &csrf_name,
                            token,
                            client_ip,
                            &request.user_agent,
                        )
                    })
            } else {
                false
            }
        } else {
            false
        };
        if !same_origin && !csrf_valid {
            return send_response(
                stream,
                "403 Forbidden",
                "text/plain; charset=utf-8",
                b"Cross-origin request rejected",
                false,
                stop,
            );
        }
        if session.is_some() {
            if let Some(token) = session_token_from_cookie(&request.cookie, &cookie_name) {
                sessions
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(token);
            }
        }
        let cookie = format!(
            "{cookie_name}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict{}",
            secure_cookie_attribute(secure_transport)
        );
        let csrf_cookie = format!(
            "{csrf_name}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict{}",
            secure_cookie_attribute(secure_transport)
        );
        return send_response_with_headers(
            stream,
            "303 See Other",
            "text/plain; charset=utf-8",
            b"Signed out",
            false,
            stop,
            &[
                ("Location", "/"),
                ("Set-Cookie", &cookie),
                ("Set-Cookie", &csrf_cookie),
            ],
        );
    }
    if require_login && session.is_none() {
        return send_login_page(
            stream,
            "401 Unauthorized",
            "Please sign in to access this file service",
            &csrf_name,
            client_ip,
            &request.user_agent,
            false,
            secure_transport,
            stop,
        );
    }
    let csrf_token = if require_login {
        issue_csrf_token(&csrf_name, client_ip, &request.user_agent)
    } else {
        String::new()
    };
    let csrf_cookie = format!(
        "{csrf_name}={csrf_token}; Path=/; Max-Age=43200; HttpOnly; SameSite=Strict{}",
        secure_cookie_attribute(secure_transport)
    );

    let relative = match decode_share_path(&request.target, token, !require_login) {
        Some(relative) => relative,
        None => {
            return send_response(
                stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"Not found",
                request.method == "HEAD",
                stop,
            )
        }
    };
    let inline_file_preview = request
        .target
        .split_once('?')
        .map(|(_, query)| query.split('&').any(|part| part == "sw_preview=1"))
        .unwrap_or(false);
    let method = request.method.as_str();
    let same_origin = same_origin_request(
            &request.origin,
            &request.referer,
            &request.fetch_site,
            &request.host,
        );
    if require_login
        && (method == "PUT" || method == "POST")
        && !same_origin
        && !csrf_token_matches(
            &request.cookie,
            &csrf_name,
            &request.csrf_token,
            client_ip,
            &request.user_agent,
        )
    {
        return send_response(
            stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            b"Cross-origin request rejected",
            false,
            stop,
        );
    }
    if allow_upload && method == "PUT" {
        if !require_login && !allow_view && relative.as_os_str().is_empty() {
            return send_response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                b"A file name is required",
                false,
                stop,
            );
        }
        return serve_upload_request(
            stream,
            request,
            relative,
            upload_directory,
            max_upload_bytes,
            upload_quota,
            stop,
        );
    }
    if method != "GET" && method != "HEAD" {
        return send_response(
            stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            b"Only GET and HEAD are allowed",
            method == "HEAD",
            stop,
        );
    }
    if !allow_view {
        if method == "HEAD" {
            return send_response(
                stream,
                "403 Forbidden",
                "text/plain; charset=utf-8",
                b"Viewing is disabled",
                true,
                stop,
            );
        }
        let limits = upload_quota
            .map(|quota| (max_upload_bytes, quota.maximum_bytes))
            .unwrap_or((max_upload_bytes, 0));
        let body = upload_only_page(
            limits.0,
            limits.1,
            &path_prefix(token, require_login),
            &csrf_token,
        );
        return if require_login {
            send_response_with_headers(
                stream,
                "200 OK",
                "text/html; charset=utf-8",
                body.as_bytes(),
                false,
                stop,
                &[("Set-Cookie", &csrf_cookie)],
            )
        } else {
            send_response(
                stream,
                "200 OK",
                "text/html; charset=utf-8",
                body.as_bytes(),
                false,
                stop,
            )
        };
    }
    let Some(root) = root else {
        return send_response(
            stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            b"Viewing is disabled",
            method == "HEAD",
            stop,
        );
    };
    let target = root.join(&relative);
    let Ok(canonical_target) = fs::canonicalize(&target) else {
        return send_response(
            stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"Not found",
            method == "HEAD",
            stop,
        );
    };
    if !canonical_target.starts_with(root) {
        return send_response(
            stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"Not found",
            method == "HEAD",
            stop,
        );
    }
    let metadata = match fs::metadata(&canonical_target) {
        Ok(metadata) => metadata,
        Err(_) => {
            return send_response(
                stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"Not found",
                method == "HEAD",
                stop,
            )
        }
    };
    if metadata.is_dir() {
        let body = match directory_listing(
            root,
            &canonical_target,
            &relative,
            &path_prefix(token, require_login),
            &csrf_token,
            if allow_upload {
                Some((
                    max_upload_bytes,
                    upload_quota.map(|quota| quota.maximum_bytes).unwrap_or(0),
                ))
            } else {
                None
            },
        ) {
            Ok(body) => body,
            Err(_) => {
                return send_response(
                    stream,
                    "403 Forbidden",
                    "text/plain; charset=utf-8",
                    b"Directory cannot be read",
                    method == "HEAD",
                    stop,
                )
            }
        };
        return if require_login {
            send_response_with_headers(
                stream,
                "200 OK",
                "text/html; charset=utf-8",
                body.as_bytes(),
                method == "HEAD",
                stop,
                &[("Set-Cookie", &csrf_cookie)],
            )
        } else {
            send_response(
                stream,
                "200 OK",
                "text/html; charset=utf-8",
                body.as_bytes(),
                method == "HEAD",
                stop,
            )
        };
    }
    if !metadata.is_file() {
        return send_response(
            stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"Not found",
            method == "HEAD",
            stop,
        );
    }
    if inline_file_preview && !is_previewable_asset(&canonical_target) {
        return send_response(
            stream,
            "415 Unsupported Media Type",
            "text/plain; charset=utf-8",
            b"This file type cannot be previewed",
            method == "HEAD",
            stop,
        );
    }
    if inline_file_preview
        && is_previewable_text(&canonical_target)
        && metadata.len() > MAX_TEXT_PREVIEW_BYTES
    {
        return send_response(
            stream,
            "413 Payload Too Large",
            "text/plain; charset=utf-8",
            b"Text preview is limited to 2 MiB. Download the file to view the full contents.",
            false,
            stop,
        );
    }
    let mut file = match File::open(&canonical_target) {
        Ok(file) => file,
        Err(_) => {
            return send_response(
                stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"Not found",
                method == "HEAD",
                stop,
            )
        }
    };
    let content_type = content_type(&canonical_target);
    let limited_media_preview = inline_file_preview
        && (is_previewable_video(&canonical_target) || is_previewable_audio(&canonical_target));
    let filename = canonical_target
        .file_name()
        .map(|value| safe_filename(&value.to_string_lossy()))
        .unwrap_or_else(|| "download".into());
    let byte_range = match request
        .range
        .as_deref()
        .map(|header| parse_byte_range(header, metadata.len()))
    {
        Some(Ok(range)) => Some(range),
        Some(Err(())) => {
            set_access_status("416 Range Not Satisfiable");
            set_access_response_content_type("text/plain; charset=utf-8");
            write!(stream, "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n", metadata.len())?;
            return Ok(());
        }
        None => None,
    };
    let (status, content_length, content_range) = match byte_range {
        Some((start, requested_end)) => {
            let end = if limited_media_preview {
                requested_end
                    .min(start.saturating_add(MAX_MEDIA_PREVIEW_RANGE_BYTES.saturating_sub(1)))
            } else {
                requested_end
            };
            file.seek(SeekFrom::Start(start))?;
            (
                "206 Partial Content",
                end - start + 1,
                Some(format!("bytes {start}-{end}/{}", metadata.len())),
            )
        }
        None if limited_media_preview => return send_response(
            stream,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            b"Media preview requires a Range request. Use the download action for the full file.",
            method == "HEAD",
            stop,
        ),
        None => ("200 OK", metadata.len(), None),
    };
    set_access_status(status);
    set_access_response_content_type(content_type);
    let disposition = if inline_file_preview { "inline" } else { "attachment" };
    let preview_csp = if inline_file_preview
        && (is_previewable_video(&canonical_target) || is_previewable_audio(&canonical_target))
    {
        "Content-Security-Policy: default-src 'none'; media-src 'self'; sandbox\r\n"
    } else if inline_file_preview {
        "Content-Security-Policy: default-src 'none'; sandbox\r\n"
    } else {
        ""
    };
    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {content_length}\r\nContent-Type: {content_type}\r\nContent-Disposition: {disposition}; filename=\"{filename}\"\r\nAccept-Ranges: bytes\r\n{}Connection: close\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\n{preview_csp}\r\n", content_range.as_ref().map(|value| format!("Content-Range: {value}\r\n")).unwrap_or_default())?;
    if method == "HEAD" {
        return Ok(());
    }
    let mut buffer = [0u8; 32 * 1024];
    let mut remaining = content_length;
    loop {
        if stop.load(Ordering::Relaxed) || remaining == 0 {
            break;
        }
        let read_limit = remaining.min(buffer.len() as u64) as usize;
        let read = file.read(&mut buffer[..read_limit])?;
        if read == 0 {
            break;
        }
        if limited_media_preview && !media_rate_limiter.wait_for(read, stop) {
            break;
        }
        stream.write_all(&buffer[..read])?;
        add_access_sent_bytes(read);
        remaining -= read as u64;
    }
    Ok(())
}

fn serve_upload_request<S: HttpStream>(
    stream: S,
    request: RequestHead,
    relative: PathBuf,
    upload_directory: Option<&Path>,
    max_upload_bytes: u64,
    upload_quota: Option<&UploadQuota>,
    stop: &AtomicBool,
) -> io::Result<()> {
    if request.method != "PUT" {
        return send_response(
            stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"Not found",
            request.method == "HEAD",
            stop,
        );
    }

    let mut components = relative.components();
    let Some(std::path::Component::Normal(component)) = components.next() else {
        return send_response(
            stream,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            b"Invalid file name",
            false,
            stop,
        );
    };
    if components.next().is_some() {
        return send_response(
            stream,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            b"Only file names are allowed",
            false,
            stop,
        );
    }
    let original_name = component.to_string_lossy();
    let filename = safe_upload_filename(&original_name);
    if filename.is_empty() {
        return send_response(
            stream,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            b"Invalid file name",
            false,
            stop,
        );
    }
    let body = if request.transfer_encoding {
        UploadBody::Chunked
    } else if let Some(content_length) = request.content_length {
        UploadBody::Fixed(content_length as u64)
    } else {
        return send_response(
            stream,
            "411 Length Required",
            "text/plain; charset=utf-8",
            b"Content-Length is required",
            false,
            stop,
        );
    };
    set_access_expected_bytes(match body {
        UploadBody::Fixed(bytes) => Some(bytes),
        UploadBody::Chunked => None,
    });
    if matches!(body, UploadBody::Fixed(size) if size > max_upload_bytes || size > MAX_UPLOAD_BYTES)
    {
        return send_response(
            stream,
            "413 Content Too Large",
            "text/plain; charset=utf-8",
            b"File exceeds the configured upload limit",
            false,
            stop,
        );
    }

    let Some(directory) = upload_directory else {
        return send_response(
            stream,
            "503 Service Unavailable",
            "text/plain; charset=utf-8",
            b"Upload directory is unavailable",
            false,
            stop,
        );
    };
    let Some(quota) = upload_quota else {
        return send_response(
            stream,
            "503 Service Unavailable",
            "text/plain; charset=utf-8",
            b"Upload quota is unavailable",
            false,
            stop,
        );
    };
    save_upload(
        stream,
        directory,
        &filename,
        body,
        max_upload_bytes.min(MAX_UPLOAD_BYTES),
        quota,
        stop,
    )
}

#[derive(Clone, Copy)]
enum UploadBody {
    Fixed(u64),
    Chunked,
}

#[derive(Debug)]
enum UploadBodyError {
    Request(io::Error),
    InvalidFraming,
    TooLarge,
    QuotaExceeded,
    Storage,
    Stopped,
}

fn save_upload<S: HttpStream>(
    mut stream: S,
    directory: &Path,
    filename: &str,
    body: UploadBody,
    max_upload_bytes: u64,
    quota: &UploadQuota,
    stop: &AtomicBool,
) -> io::Result<()> {
    stream.set_read_timeout(Some(UPLOAD_READ_TIMEOUT))?;
    let destination = directory.join(filename);
    if destination.exists() {
        return send_response(
            stream,
            "409 Conflict",
            "text/plain; charset=utf-8",
            b"A file with this name already exists",
            false,
            stop,
        );
    }
    let mut reserved_bytes = 0;
    if let UploadBody::Fixed(content_length) = body {
        if !quota.reserve(content_length) {
            return send_response(
                stream,
                "507 Insufficient Storage",
                "text/plain; charset=utf-8",
                b"The upload directory has reached its configured size limit",
                false,
                stop,
            );
        }
        reserved_bytes = content_length;
    }
    let temporary = directory.join(format!(".sideweave-upload-{}.part", uuid::Uuid::new_v4()));
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
    {
        Ok(file) => file,
        Err(_) => {
            quota.release(reserved_bytes);
            return send_response(
                stream,
                "507 Insufficient Storage",
                "text/plain; charset=utf-8",
                b"Could not create a file in the upload directory",
                false,
                stop,
            );
        }
    };

    let write_result = match body {
        UploadBody::Fixed(content_length) => {
            copy_fixed_upload(&mut stream, &mut file, content_length, stop)
        }
        UploadBody::Chunked => copy_chunked_upload(
            &mut stream,
            &mut file,
            max_upload_bytes,
            quota,
            stop,
            &mut reserved_bytes,
        ),
    }
    .and_then(|()| {
        file.flush().map_err(|_| UploadBodyError::Storage)?;
        file.sync_all().map_err(|_| UploadBodyError::Storage)
    });
    drop(file);

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        quota.release(reserved_bytes);
        if matches!(error, UploadBodyError::Stopped) || stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let received_bytes = current_access_received_bytes();
        let (status, message): (&str, String) = match error {
            UploadBodyError::Request(error) if error.kind() == io::ErrorKind::TimedOut => {
                ("408 Request Timeout", "Upload timed out".into())
            }
            UploadBodyError::Request(error) => (
                "400 Bad Request",
                format!("Upload body read failed after {received_bytes} bytes: {error}"),
            ),
            UploadBodyError::InvalidFraming => {
                let message = match body {
                    UploadBody::Fixed(expected_bytes) => format!(
                        "Invalid or incomplete upload body: received {received_bytes} of {expected_bytes} bytes"
                    ),
                    UploadBody::Chunked => format!(
                        "Invalid or incomplete chunked upload body after {received_bytes} bytes"
                    ),
                };
                ("400 Bad Request", message)
            }
            UploadBodyError::TooLarge => (
                "413 Content Too Large",
                "File exceeds the configured upload limit".into(),
            ),
            UploadBodyError::QuotaExceeded => (
                "507 Insufficient Storage",
                "The upload directory has reached its configured size limit".into(),
            ),
            UploadBodyError::Storage => (
                "507 Insufficient Storage",
                "Could not write the uploaded file".into(),
            ),
            UploadBodyError::Stopped => return Ok(()),
        };
        set_access_error(&message);
        return send_response(
            stream,
            status,
            "text/plain; charset=utf-8",
            message.as_bytes(),
            false,
            stop,
        );
    }
    if stop.load(Ordering::Relaxed) {
        let _ = fs::remove_file(&temporary);
        quota.release(reserved_bytes);
        return Ok(());
    }

    match fs::hard_link(&temporary, &destination) {
        Ok(()) => {
            let _ = fs::remove_file(&temporary);
            send_response(
                stream,
                "201 Created",
                "text/plain; charset=utf-8",
                format!("Uploaded {filename}").as_bytes(),
                false,
                stop,
            )
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary);
            quota.release(reserved_bytes);
            send_response(
                stream,
                "409 Conflict",
                "text/plain; charset=utf-8",
                b"A file with this name already exists",
                false,
                stop,
            )
        }
        Err(_) => {
            let _ = fs::remove_file(&temporary);
            quota.release(reserved_bytes);
            send_response(
                stream,
                "507 Insufficient Storage",
                "text/plain; charset=utf-8",
                b"Could not place the uploaded file in the destination directory",
                false,
                stop,
            )
        }
    }
}

fn copy_fixed_upload<S: HttpStream>(
    stream: &mut S,
    file: &mut File,
    content_length: u64,
    stop: &AtomicBool,
) -> Result<(), UploadBodyError> {
    let mut remaining = content_length;
    let mut buffer = [0u8; 32 * 1024];
    while remaining > 0 && !stop.load(Ordering::Relaxed) {
        let read_limit = buffer.len().min(remaining as usize);
        let count = read_retry_interrupted(stream, &mut buffer[..read_limit])
            .map_err(UploadBodyError::Request)?;
        if count == 0 {
            return Err(UploadBodyError::InvalidFraming);
        }
        file.write_all(&buffer[..count])
            .map_err(|_| UploadBodyError::Storage)?;
        add_access_received_bytes(count);
        remaining -= count as u64;
    }
    if stop.load(Ordering::Relaxed) {
        Err(UploadBodyError::Stopped)
    } else {
        Ok(())
    }
}

fn copy_chunked_upload<S: HttpStream>(
    stream: &mut S,
    file: &mut File,
    max_upload_bytes: u64,
    quota: &UploadQuota,
    stop: &AtomicBool,
    reserved_bytes: &mut u64,
) -> Result<(), UploadBodyError> {
    let mut total_bytes = 0u64;
    let mut buffer = [0u8; 32 * 1024];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(UploadBodyError::Stopped);
        }
        let line = read_http_line(stream, 1024).map_err(UploadBodyError::Request)?;
        let line = std::str::from_utf8(&line).map_err(|_| UploadBodyError::InvalidFraming)?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        if size_text.is_empty() || !size_text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(UploadBodyError::InvalidFraming);
        }
        let chunk_size =
            u64::from_str_radix(size_text, 16).map_err(|_| UploadBodyError::InvalidFraming)?;
        if chunk_size == 0 {
            let mut trailer_bytes = 0usize;
            loop {
                let trailer =
                    read_http_line(stream, MAX_HEADER_BYTES).map_err(UploadBodyError::Request)?;
                trailer_bytes = trailer_bytes
                    .checked_add(trailer.len() + 2)
                    .ok_or(UploadBodyError::InvalidFraming)?;
                if trailer_bytes > MAX_HEADER_BYTES
                    || (!trailer.is_empty() && !trailer.contains(&b':'))
                {
                    return Err(UploadBodyError::InvalidFraming);
                }
                if trailer.is_empty() {
                    return Ok(());
                }
            }
        }
        total_bytes = total_bytes
            .checked_add(chunk_size)
            .ok_or(UploadBodyError::TooLarge)?;
        if total_bytes > max_upload_bytes || total_bytes > MAX_UPLOAD_BYTES {
            return Err(UploadBodyError::TooLarge);
        }
        let new_reserved_bytes = reserved_bytes
            .checked_add(chunk_size)
            .ok_or(UploadBodyError::QuotaExceeded)?;
        if !quota.reserve(chunk_size) {
            return Err(UploadBodyError::QuotaExceeded);
        }
        *reserved_bytes = new_reserved_bytes;

        let mut remaining = chunk_size;
        while remaining > 0 {
            if stop.load(Ordering::Relaxed) {
                return Err(UploadBodyError::Stopped);
            }
            let read_limit = buffer.len().min(remaining as usize);
            let count = read_retry_interrupted(stream, &mut buffer[..read_limit])
                .map_err(UploadBodyError::Request)?;
            if count == 0 {
                return Err(UploadBodyError::InvalidFraming);
            }
            file.write_all(&buffer[..count])
                .map_err(|_| UploadBodyError::Storage)?;
            add_access_received_bytes(count);
            remaining -= count as u64;
        }
        let mut crlf = [0u8; 2];
        stream
            .read_exact(&mut crlf)
            .map_err(UploadBodyError::Request)?;
        if crlf != *b"\r\n" {
            return Err(UploadBodyError::InvalidFraming);
        }
    }
}

fn read_retry_interrupted<S: HttpStream>(stream: &mut S, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match stream.read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

fn read_http_line<S: HttpStream>(stream: &mut S, max_bytes: usize) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte)?;
        if byte[0] == b'\n' {
            if line.pop() != Some(b'\r') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP line is missing CRLF",
                ));
            }
            return Ok(line);
        }
        line.push(byte[0]);
        if line.len() > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP line is too long",
            ));
        }
    }
}

const SHARE_PAGE_STYLE: &str = r#"
:root {
  color-scheme: light;
  --page: #f4f7fb;
  --surface: #ffffff;
  --surface-soft: #f8fafc;
  --text: #162033;
  --muted: #68758a;
  --line: #e3e9f1;
  --accent: #315ee8;
  --accent-soft: #edf2ff;
  --success: #16865b;
  --shadow: 0 18px 48px rgba(27, 48, 82, .08);
}
* { box-sizing: border-box; }
body {
  min-width: 320px;
  min-height: 100vh;
  margin: 0;
  background: radial-gradient(ellipse at 50% -18%, #e5edff 0, transparent 46%), var(--page);
  color: var(--text);
  font: 15px/1.55 Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  -webkit-font-smoothing: antialiased;
}
a { color: inherit; text-decoration: none; }
button, input { font: inherit; }
.page-shell { width: min(100% - 40px, 920px); margin: 0 auto; padding: 28px 0 54px; }
.sitebar { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.theme-toggle { display: inline-flex; min-height: 36px; flex: none; align-items: center; justify-content: center; gap: 7px; padding: 0 11px; border: 1px solid var(--line); border-radius: 9px; background: var(--surface); color: var(--text); font-size: 11px; font-weight: 650; cursor: pointer; }
.theme-toggle:hover { border-color: var(--accent); }
.theme-toggle-icon { font-size: 15px; line-height: 1; }
.logout-form { display: flex; justify-content: flex-end; margin: 12px 0 0; }
.sitebar .logout-form { margin: 0; }
.logout-form button { padding: 7px 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); color: var(--text); cursor: pointer; }
.brand { display: inline-flex; align-items: center; gap: 10px; color: var(--text); font-size: 14px; font-weight: 750; letter-spacing: -.02em; }
.brand-mark { display: grid; width: 34px; height: 34px; place-items: center; border-radius: 11px; background: linear-gradient(145deg, #4778ff, #284bc5); box-shadow: 0 5px 14px rgba(49, 94, 232, .24); color: white; font-size: 11px; letter-spacing: .04em; }
.share-badge { display: inline-flex; align-items: center; gap: 7px; padding: 7px 11px; border: 1px solid var(--line); border-radius: 999px; background: rgba(255,255,255,.72); color: var(--muted); font-size: 12px; }
.share-badge::before { width: 7px; height: 7px; border-radius: 50%; background: var(--success); content: ''; }
.page-main { margin-top: 20px; }
.page-main > .file-browser:first-child, .page-main > .upload-panel:first-child { margin-top: 0; }
.breadcrumbs { display: flex; align-items: center; gap: 8px; min-width: 0; padding: 11px 13px; overflow-x: auto; border: 1px solid var(--line); border-radius: 11px; background: rgba(255,255,255,.7); color: var(--muted); font-size: 12px; white-space: nowrap; scrollbar-width: thin; }
.breadcrumbs a { color: var(--accent); }
.breadcrumbs a:hover { text-decoration: underline; text-underline-offset: 3px; }
.crumb-separator { color: #a4afbf; }
.crumb-current { min-width: 0; overflow: hidden; color: var(--text); font-weight: 650; text-overflow: ellipsis; }
.upload-panel, .file-browser { margin-top: 18px; overflow: hidden; border: 1px solid var(--line); border-radius: 16px; background: var(--surface); box-shadow: var(--shadow); }
.upload-panel { padding: 20px; }
.upload-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: 18px; }
.upload-heading h2, .list-heading h2 { margin: 0; font-size: 16px; letter-spacing: -.02em; }
.upload-heading p { margin: 5px 0 0; color: var(--muted); font-size: 12px; }
.upload-limit { flex: none; padding: 6px 9px; border-radius: 8px; background: var(--surface-soft); color: var(--muted); font-size: 11px; }
.upload-controls { display: grid; grid-template-columns: minmax(0,1fr) auto; gap: 10px; margin-top: 17px; }
.upload-selection { display: grid; min-width: 0; gap: 8px; padding: 10px; border: 1px solid var(--line); border-radius: 11px; background: var(--surface-soft); }
.upload-pickers { display: flex; flex-wrap: wrap; gap: 8px; }
.file-pick-button, .file-picker-clear { min-height: 42px; padding: 0 13px; border: 1px solid var(--line); border-radius: 9px; background: var(--surface); color: var(--text); font-size: 12px; font-weight: 650; cursor: pointer; touch-action: manipulation; }
.file-pick-button { flex: 1 1 120px; }
.file-pick-button:hover, .file-picker-clear:hover { border-color: var(--accent); background: var(--accent-soft); }
.file-pick-button:disabled, .file-picker-clear:disabled { opacity: .55; cursor: not-allowed; }
.file-picker-clear { color: var(--muted); font-weight: 500; }
.file-picker-label { min-width: 0; max-height: 4.5em; overflow: auto; color: var(--muted); font-size: 11px; line-height: 1.5; overflow-wrap: anywhere; }
.file-input-hidden[hidden], .file-picker-clear[hidden] { display: none !important; }
.primary-button { min-height: 46px; padding: 0 18px; border: 1px solid var(--accent); border-radius: 10px; background: var(--accent); color: #fff; font-size: 13px; font-weight: 700; cursor: pointer; transition: background .15s, transform .15s; }
.primary-button:hover:not(:disabled) { background: #244dcc; transform: translateY(-1px); }
.primary-button:disabled { border-color: #bbc6d7; background: #bbc6d7; cursor: not-allowed; }
.upload-status { display: grid; max-height: 180px; gap: 6px; margin-top: 12px; overflow: auto; }
.upload-status:empty { display: none; }
.upload-result { margin: 0; padding: 7px 10px; border-radius: 8px; background: var(--surface-soft); color: var(--muted); font-size: 12px; overflow-wrap: anywhere; }
.upload-result.success { background: #eaf8f1; color: #146b49; }
.upload-result.error { background: #fff0f0; color: #a23131; }
.upload-result.progress { background: var(--accent-soft); color: #274cc0; }
.upload-progress-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 10px; }
.upload-progress-name { min-width: 0; overflow-wrap: anywhere; font-weight: 650; }
.upload-progress-meta { flex: none; color: var(--muted); font-size: 11px; text-align: right; }
.upload-progress-track { height: 7px; margin-top: 8px; overflow: hidden; border-radius: 999px; background: color-mix(in srgb, var(--text) 12%, transparent); }
.upload-progress-fill { width: 0; height: 100%; border-radius: inherit; background: var(--accent); transition: width .12s linear; }
.list-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 17px 20px; border-bottom: 1px solid var(--line); }
.item-count { color: var(--muted); font-size: 11px; }
.file-tools { display: grid; grid-template-columns: minmax(0,1fr) 190px; gap: 9px; padding: 12px 16px 0; }
.file-search, .file-type-filter { min-width: 0; height: 39px; padding: 0 11px; border: 1px solid var(--line); border-radius: 9px; background: var(--surface-soft); color: var(--text); font: inherit; font-size: 12px; }
.file-search:focus, .file-type-filter:focus { outline: 2px solid color-mix(in srgb, var(--accent) 38%, transparent); border-color: var(--accent); }
.file-row[hidden] { display: none; }
.filter-empty { padding: 24px 12px; color: var(--muted); font-size: 12px; text-align: center; }
.file-list { padding: 7px; }
.file-row { display: grid; grid-template-columns: 42px minmax(0,1fr) minmax(112px,auto) 40px; align-items: center; gap: 12px; min-height: 64px; padding: 9px 12px; border-radius: 11px; transition: background .14s; }
.file-row + .file-row { margin-top: 2px; }
.file-row:hover { background: var(--surface-soft); }
.file-details-trigger { display: grid; grid-column: 1 / 4; grid-template-columns: 42px minmax(0,1fr) minmax(112px,auto); align-items: center; gap: 12px; width: 100%; min-width: 0; padding: 0; border: 0; background: transparent; color: inherit; font: inherit; text-align: left; cursor: pointer; }
.file-row:focus-visible, .file-details-trigger:focus-visible, .file-download-button:focus-visible, .breadcrumbs a:focus-visible, .brand:focus-visible, .primary-button:focus-visible { outline: 3px solid rgba(49,94,232,.3); outline-offset: 2px; }
.file-details-dialog { width: min(96vw, 1100px); max-width: 96vw; max-height: 94vh; padding: 0; overflow: hidden; border: 1px solid var(--line); border-radius: 14px; background: var(--surface); color: var(--text); box-shadow: 0 24px 80px rgba(0,0,0,.35); }
.file-details-dialog::backdrop { background: rgba(7,12,22,.78); backdrop-filter: blur(3px); }
.file-details-shell { display: flex; max-height: 94vh; flex-direction: column; }
.file-details-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 62px; padding: 9px 14px 9px 18px; border-bottom: 1px solid var(--line); }
.file-details-heading { display: flex; min-width: 0; flex-direction: column; gap: 3px; }
.file-details-eyebrow { color: var(--muted); font-size: 10px; }
.file-details-title { margin: 0; overflow: hidden; font-size: 14px; font-weight: 650; text-overflow: ellipsis; white-space: nowrap; }
.file-details-actions { display: flex; flex: none; gap: 8px; }
.file-details-body { display: flex; min-height: 0; flex-direction: column; gap: 12px; padding: 12px 16px 16px; overflow: auto; }
.file-details-info { display: grid; grid-template-columns: repeat(auto-fit,minmax(145px,1fr)); align-content: start; gap: 7px; min-width: 0; margin: 0; }
.file-details-info > div { min-width: 0; padding: 8px 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface-soft); }
.file-details-info dt { color: var(--muted); font-size: 9px; }
.file-details-info dd { margin: 3px 0 0; overflow-wrap: anywhere; font-size: 11px; }
.file-details-preview { display: flex; min-height: 180px; max-height: calc(94vh - 190px); flex-direction: column; gap: 8px; padding: 10px; overflow: auto; border: 1px solid var(--line); border-radius: 10px; background: var(--surface-soft); }
.file-details-preview[hidden] { display: none; }
.file-details-preview [hidden] { display: none !important; }
.file-details-preview-heading { color: var(--muted); font-size: 10px; font-weight: 600; }
.file-details-preview img { display: block; max-width: 100%; max-height: calc(94vh - 240px); align-self: center; border-radius: 7px; background: #111722; object-fit: contain; cursor: zoom-in; }
.file-image-zoom { position: fixed; inset: 0; width: 100vw; height: 100vh; width: 100dvw; height: 100dvh; max-width: none; max-height: none; margin: 0; padding: 56px 24px 24px; overflow: hidden; border: 0; background: rgba(5,8,14,.96); color: #fff; }
.file-image-zoom::backdrop { background: rgba(5,8,14,.96); }
.file-image-zoom[open] { display: grid; place-items: center; }
.file-image-zoom img { display: block; max-width: 100%; max-height: 100%; object-fit: contain; cursor: zoom-out; }
.file-image-zoom-close { position: absolute; top: 14px; right: 16px; min-height: 36px; padding: 0 13px; border: 1px solid rgba(255,255,255,.32); border-radius: 8px; background: rgba(255,255,255,.1); color: #fff; font: inherit; cursor: pointer; }
.file-details-preview video { display: block; width: min(100%, 960px); max-height: calc(94vh - 240px); align-self: center; border-radius: 7px; background: #111722; }
.file-details-preview audio { display: block; width: min(100%, 760px); align-self: center; }
.file-details-preview video[hidden], .file-details-preview audio[hidden] { display: none; }
.file-media-load { align-self: center; }
.file-details-text { display: block; width: 100%; min-width: 0; min-height: 220px; height: min(58vh, 600px); resize: vertical; padding: 12px; border: 1px solid var(--line); border-radius: 8px; outline: none; background: var(--surface); color: var(--text); font: 12px/1.55 var(--font-mono, ui-monospace, SFMono-Regular, Consolas, monospace); tab-size: 4; white-space: pre; overflow: auto; }
.file-details-text[hidden] { display: none; }
.file-details-preview-status { max-width: 420px; padding: 16px; color: var(--text); font-size: 12px; line-height: 1.6; text-align: center; }
.file-details-preview-status[hidden] { display: none; }
.preview-action { display: inline-flex; min-height: 34px; align-items: center; padding: 0 11px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface-soft); color: var(--text); font: inherit; font-size: 12px; text-decoration: none; cursor: pointer; }
.preview-action:hover { border-color: var(--accent); }
.file-icon { display: grid; width: 38px; height: 38px; place-items: center; border-radius: 11px; background: var(--accent-soft); color: var(--accent); }
.file-icon.folder { background: #fff6df; color: #b77b0b; }
.file-icon.image { background: #e8f6ee; color: #258252; }
.file-icon.video { background: #fff0e7; color: #c45d2e; }
.file-icon.audio { background: #f1edff; color: #7251bb; }
.file-icon.archive { background: #fff5dd; color: #a87812; }
.file-icon.document { background: #fff0f0; color: #b23d49; }
.file-icon.spreadsheet { background: #e8f6ee; color: #258252; }
.file-icon.presentation { background: #fff0e7; color: #c45d2e; }
.file-icon.code { background: #eaf0ff; color: #3c5fb7; }
.file-icon.application { background: #edf1f6; color: #52647d; }
.file-icon.font { background: #fceefa; color: #a13c82; }
.file-icon svg { width: 20px; height: 20px; }
.file-main { display: flex; min-width: 0; flex-direction: column; gap: 2px; }
.file-name { overflow-wrap: anywhere; color: var(--text); font-size: 13px; font-weight: 650; }
.file-kind { display: flex; align-items: center; gap: 6px; color: var(--muted); font-size: 10px; }
.file-extension { padding: 1px 5px; border: 1px solid var(--line); border-radius: 4px; color: var(--muted); font: 9px/1.5 ui-monospace, SFMono-Regular, Consolas, monospace; }
.file-meta { display: flex; min-width: 0; flex-direction: column; gap: 3px; color: var(--muted); font-size: 11px; text-align: right; white-space: nowrap; }
.file-meta time { color: var(--muted); font-size: 10px; }
.file-arrow { color: #a4afbf; font-size: 18px; text-align: center; }
.file-action-spacer { grid-column: 4; }
.file-download-button { display: grid; width: 34px; height: 34px; place-items: center; justify-self: center; border: 1px solid var(--line); border-radius: 9px; background: var(--surface-soft); color: var(--accent); font-size: 17px; text-decoration: none; transition: border-color .14s, background .14s; }
.file-download-button:hover { border-color: var(--accent); background: var(--accent-soft); }
.empty-state { display: grid; min-height: 168px; place-content: center; justify-items: center; gap: 7px; padding: 24px; color: var(--muted); text-align: center; }
.empty-icon { display: grid; width: 42px; height: 42px; place-items: center; margin-bottom: 2px; border-radius: 14px; background: var(--surface-soft); color: #8794a8; font-size: 20px; }
.empty-state strong { color: var(--text); font-size: 13px; }
.empty-state span { font-size: 11px; }
.page-footer { padding: 19px 4px 0; color: #8a96a8; font-size: 10px; text-align: center; }
html[data-theme="dark"] { color-scheme: dark; --page: #0e1420; --surface: #171f2d; --surface-soft: #1c2737; --text: #e8edf6; --muted: #9ba9bc; --line: #2a3749; --accent: #86a5ff; --accent-soft: #222f4b; --success: #56d39d; --shadow: 0 18px 48px rgba(0,0,0,.18); }
html[data-theme="dark"] body { background: radial-gradient(ellipse at 50% -18%, #1b2b4a 0, transparent 46%), var(--page); }
html[data-theme="dark"] .share-badge { background: rgba(23,31,45,.8); }
html[data-theme="dark"] .breadcrumbs { background: rgba(23,31,45,.8); }
html[data-theme="dark"] .file-icon.folder { background: #382f1d; color: #edc16b; }
html[data-theme="dark"] .file-icon.image, html[data-theme="dark"] .file-icon.spreadsheet { background: #1d382b; color: #7ed9a5; }
html[data-theme="dark"] .file-icon.video, html[data-theme="dark"] .file-icon.presentation { background: #3c2c25; color: #f3a879; }
html[data-theme="dark"] .file-icon.audio { background: #302948; color: #b8a3ff; }
html[data-theme="dark"] .file-icon.archive { background: #3d341f; color: #e9c16d; }
html[data-theme="dark"] .file-icon.document { background: #3d272c; color: #f29aa2; }
html[data-theme="dark"] .file-icon.code { background: #24334f; color: #9bb4ff; }
html[data-theme="dark"] .file-icon.application { background: #273243; color: #b0bfd5; }
html[data-theme="dark"] .file-icon.font { background: #3a2637; color: #ed9bd4; }
html[data-theme="dark"] .upload-result.success { background: #18372c; color: #8be0b7; }
html[data-theme="dark"] .upload-result.error { background: #3c2529; color: #ffaaaa; }
html[data-theme="dark"] .upload-result.progress { background: #222f4b; color: #b7c8ff; }
html[data-theme="dark"] .upload-progress-track { background: rgba(255,255,255,.12); }
html[data-theme="dark"] .primary-button:disabled { border-color: #445168; background: #445168; color: #bbc5d4; }
@media (max-width: 620px) {
  .page-shell { width: min(100% - 24px, 920px); padding: max(14px, env(safe-area-inset-top)) 0 max(28px, env(safe-area-inset-bottom)); }
  .sitebar { flex-wrap: wrap; gap: 9px; }
  .theme-toggle, .logout-form button { min-height: 42px; }
  .page-main { margin-top: 14px; }
  .upload-panel { padding: 16px; }
  .upload-heading { flex-direction: column; gap: 9px; }
  .upload-limit { align-self: flex-start; }
  .upload-controls { grid-template-columns: 1fr; }
  .upload-pickers { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); }
  .file-pick-button { min-height: 46px; padding: 0 8px; }
  .file-tools { grid-template-columns: 1fr; padding: 11px 12px 0; }
  .upload-progress-head { flex-direction: column; gap: 4px; }
  .upload-progress-meta { text-align: left; }
  .primary-button { width: 100%; }
  .list-heading { padding: 15px 16px; }
  .file-list { padding: 6px; }
  .file-row { grid-template-columns: 38px minmax(0,1fr) auto; gap: 10px; min-height: 68px; padding: 9px 8px; }
  .file-download-button { width: 42px; height: 42px; }
  .file-details-trigger { grid-column: 1 / 3; grid-template-columns: 38px minmax(0,1fr); gap: 10px; }
  .file-icon { width: 34px; height: 34px; border-radius: 10px; }
  .file-icon svg { width: 18px; height: 18px; }
  .file-meta { grid-column: 2; margin-top: -6px; text-align: left; }
  .file-arrow { grid-column: 3; grid-row: 1 / span 2; }
  .file-action-spacer { grid-column: 3; grid-row: 1 / span 2; }
  .file-details-body { gap: 10px; padding: 10px; }
  .file-details-actions .preview-action { min-height: 42px; padding: 0 10px; }
  .file-details-dialog { width: calc(100vw - 16px); max-width: calc(100vw - 16px); max-height: calc(100dvh - 16px); }
  .file-details-shell { max-height: calc(100dvh - 16px); }
  .file-details-info { grid-template-columns: repeat(2,minmax(0,1fr)); }
  .file-details-preview { max-height: 58vh; max-height: 58dvh; padding: 8px; }
  .file-details-preview img { max-height: 48vh; }
  .file-details-preview video { max-height: 48vh; }
  .file-details-text { min-height: 200px; height: 48vh; height: 48dvh; }
}
@media (max-width: 380px) {
  .page-shell { width: min(100% - 20px, 920px); }
  .share-badge { padding: 6px 9px; font-size: 11px; }
  .file-row { grid-template-columns: 32px minmax(0,1fr) auto; gap: 8px; }
  .file-details-trigger { grid-template-columns: 32px minmax(0,1fr); gap: 8px; }
  .file-icon { width: 30px; height: 30px; }
  .file-download-button { width: 42px; height: 42px; }
  .file-details-info { grid-template-columns: minmax(0,1fr); }
  .file-details-header { gap: 8px; padding: 9px; }
  .file-details-actions { gap: 6px; }
  .file-details-actions .preview-action { min-height: 42px; padding: 0 10px; }
  .file-image-zoom { padding: calc(56px + env(safe-area-inset-top)) 10px calc(12px + env(safe-area-inset-bottom)); }
}
"#;

const THEME_BOOTSTRAP_SCRIPT: &str = r#"<script>(()=>{let theme='dark';try{theme=localStorage.getItem('sw-file-share-theme')||'dark'}catch(_){}document.documentElement.dataset.theme=theme==='light'?'light':'dark'})()</script>"#;
const THEME_TOGGLE_BUTTON: &str = r#"<button id="theme-toggle" class="theme-toggle" type="button" aria-label="切换到浅色模式"><span id="theme-toggle-icon" class="theme-toggle-icon" aria-hidden="true">☀</span><span id="theme-toggle-label">浅色模式</span></button>"#;
const THEME_TOGGLE_SCRIPT: &str = r#"<script>(()=>{const button=document.getElementById('theme-toggle');const icon=document.getElementById('theme-toggle-icon');const label=document.getElementById('theme-toggle-label');const meta=document.querySelector('meta[name="theme-color"]');function apply(theme,persist){document.documentElement.dataset.theme=theme;const light=theme==='light';icon.textContent=light?'☾':'☀';label.textContent=light?'深色模式':'浅色模式';button.setAttribute('aria-label',light?'切换到深色模式':'切换到浅色模式');if(meta)meta.content=light?'#f4f7fb':'#0e1420';if(persist){try{localStorage.setItem('sw-file-share-theme',theme)}catch(_){}}}apply(document.documentElement.dataset.theme==='light'?'light':'dark',false);button.addEventListener('click',()=>apply(document.documentElement.dataset.theme==='dark'?'light':'dark',true))})()</script>"#;
const FILE_DETAILS_SCRIPT: &str = r#"
<script>
const detailsDialog = document.getElementById('file-details-dialog');
const detailsTitle = document.getElementById('file-details-title');
const detailsType = document.getElementById('file-details-type');
const detailsExtension = document.getElementById('file-details-extension');
const detailsSize = document.getElementById('file-details-size');
const detailsModified = document.getElementById('file-details-modified');
const detailsPath = document.getElementById('file-details-path');
const detailsPreview = document.getElementById('file-details-preview');
const detailsImage = document.getElementById('file-details-image');
const imageZoomDialog = document.getElementById('file-image-zoom');
const imageZoom = document.getElementById('file-image-zoom-image');
const detailsVideo = document.getElementById('file-details-video');
const detailsAudio = document.getElementById('file-details-audio');
const detailsMediaLoad = document.getElementById('file-details-media-load');
const detailsText = document.getElementById('file-details-text');
const detailsStatus = document.getElementById('file-details-preview-status');
const detailsDownload = document.getElementById('file-details-download');
let previewRequestId = 0;
let pendingMedia = null;
for (const media of [detailsVideo, detailsAudio]) {
  media.addEventListener('error', () => {
    if (Number(media.dataset.previewRequestId) !== previewRequestId || !detailsDialog.open) return;
    detailsStatus.hidden = false;
    detailsStatus.textContent = '浏览器无法播放此编码，请下载原文件查看。';
  });
}
function decodeTextPreview(buffer) {
  const bytes = new Uint8Array(buffer);
  let encoding = 'utf-8';
  let offset = 0;
  if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) offset = 3;
  else if (bytes[0] === 0xff && bytes[1] === 0xfe) { encoding = 'utf-16le'; offset = 2; }
  else if (bytes[0] === 0xfe && bytes[1] === 0xff) { encoding = 'utf-16be'; offset = 2; }
  try {
    return new TextDecoder(encoding, { fatal: true }).decode(bytes.subarray(offset));
  } catch (_) {
    try { return new TextDecoder('gb18030').decode(bytes); }
    catch (_) { return new TextDecoder('utf-8').decode(bytes); }
  }
}
function clearDetailsPreview() {
  previewRequestId++;
  pendingMedia = null;
  if (imageZoomDialog.open && typeof imageZoomDialog.close === 'function') imageZoomDialog.close();
  imageZoom.removeAttribute('src');
  imageZoom.alt = '';
  detailsMediaLoad.hidden = true;
  detailsImage.removeAttribute('src');
  detailsImage.hidden = true;
  for (const media of [detailsVideo, detailsAudio]) {
    media.pause();
    media.removeAttribute('src');
    media.load();
    media.hidden = true;
  }
  detailsText.value = '';
  detailsText.hidden = true;
  detailsStatus.textContent = '';
  detailsStatus.hidden = true;
  detailsPreview.hidden = true;
}
function openImageZoom() {
  if (detailsImage.hidden) return;
  const source = detailsImage.currentSrc || detailsImage.src;
  if (!source) return;
  imageZoom.src = source;
  imageZoom.alt = detailsImage.alt || '';
  if (typeof imageZoomDialog.showModal === 'function') imageZoomDialog.showModal();
  else imageZoomDialog.setAttribute('open', '');
  document.getElementById('file-image-zoom-close').focus();
}
detailsImage.addEventListener('click', openImageZoom);
detailsImage.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    openImageZoom();
  }
});
document.getElementById('file-image-zoom-close').addEventListener('click', () => {
  if (typeof imageZoomDialog.close === 'function') imageZoomDialog.close();
  else imageZoomDialog.removeAttribute('open');
});
imageZoomDialog.addEventListener('click', (event) => {
  if (event.target === imageZoomDialog) {
    if (typeof imageZoomDialog.close === 'function') imageZoomDialog.close();
    else imageZoomDialog.removeAttribute('open');
  }
});
imageZoomDialog.addEventListener('close', () => {
  imageZoom.removeAttribute('src');
  imageZoom.alt = '';
});
detailsMediaLoad.addEventListener('click', () => {
  if (!pendingMedia || pendingMedia.requestId !== previewRequestId) return;
  const { kind, url, requestId } = pendingMedia;
  const media = kind === 'video' ? detailsVideo : detailsAudio;
  pendingMedia = null;
  detailsMediaLoad.hidden = true;
  media.hidden = false;
  media.dataset.previewRequestId = String(requestId);
  media.src = url;
  media.load();
  const playback = media.play();
  if (playback) playback.catch(() => {});
});
function closeFileDetails() {
  clearDetailsPreview();
  if (typeof detailsDialog.close === 'function') detailsDialog.close();
  else detailsDialog.removeAttribute('open');
}
for (const trigger of document.querySelectorAll('.file-details-trigger')) {
  trigger.addEventListener('click', () => {
    const data = trigger.dataset;
    detailsTitle.textContent = data.fileName || '';
    detailsType.textContent = data.fileType || '';
    detailsExtension.textContent = data.fileExtension ? '.' + data.fileExtension : '无';
    detailsSize.textContent = (data.fileSizeLabel || '') + ' (' + Number(data.fileSize || 0).toLocaleString() + ' bytes)';
    detailsModified.textContent = data.fileModified || '未知';
    detailsPath.textContent = data.filePath || '';
    detailsDownload.href = data.fileUrl || '#';
    detailsDownload.download = data.fileName || '';
    clearDetailsPreview();
    const requestId = previewRequestId;
    const hasPreview = Boolean(data.previewUrl);
    detailsPreview.hidden = !hasPreview;
    if (typeof detailsDialog.showModal === 'function') detailsDialog.showModal();
    else detailsDialog.setAttribute('open', '');
    if (!hasPreview) return;
    if (data.previewKind === 'image') {
      detailsImage.hidden = false;
      detailsImage.src = data.previewUrl;
      detailsImage.alt = data.fileName || '';
      return;
    }
    if (data.previewKind === 'video' || data.previewKind === 'audio') {
      pendingMedia = { kind: data.previewKind, url: data.previewUrl, requestId };
      detailsMediaLoad.textContent = data.previewKind === 'video'
        ? '\u70b9\u51fb\u540e\u52a0\u8f7d\u5e76\u64ad\u653e\u89c6\u9891'
        : '\u70b9\u51fb\u540e\u52a0\u8f7d\u5e76\u64ad\u653e\u97f3\u9891';
      detailsMediaLoad.hidden = false;
      return;
    }
    if (data.previewKind === 'text') {
      detailsStatus.hidden = false;
      detailsStatus.textContent = '正在加载文本预览…';
      fetch(data.previewUrl).then(async (response) => {
        if (!response.ok) return { error: response.status, text: '' };
        return { error: 0, text: decodeTextPreview(await response.arrayBuffer()) };
      }).then((result) => {
        if (requestId !== previewRequestId || !detailsDialog.open) return;
        if (result.error) {
          detailsStatus.textContent = result.error === 413
            ? '文本文件超过 2 MiB 在线预览上限，请下载查看。'
            : '文本预览加载失败，请下载查看。';
          return;
        }
        detailsText.value = result.text;
        detailsText.hidden = false;
        detailsStatus.hidden = true;
      }).catch(() => {
        if (requestId !== previewRequestId || !detailsDialog.open) return;
        detailsStatus.textContent = '文本预览加载失败，请下载查看。';
      });
    }
  });
}
document.getElementById('file-details-close').addEventListener('click', closeFileDetails);
detailsDialog.addEventListener('click', (event) => {
  if (event.target === detailsDialog) closeFileDetails();
});
detailsDialog.addEventListener('close', clearDetailsPreview);
</script>"#;
const LOGIN_PAGE_STYLE: &str = r#"
:root { color-scheme: dark; --page: #0e1420; --surface: #171f2d; --text: #e8edf6; --muted: #9ba9bc; --line: #2a3749; --accent: #86a5ff; --input: #111927; --danger: #ff9c9c; }
html[data-theme="light"] { color-scheme: light; --page: #f4f7fb; --surface: #fff; --text: #172033; --muted: #68758a; --line: #dbe2ec; --accent: #315eea; --input: #fff; --danger: #b42318; }
* { box-sizing: border-box; }
body { display: grid; min-height: 100vh; place-items: center; margin: 0; padding: 60px 16px 24px; background: radial-gradient(ellipse at 50% -18%, #1b2b4a 0, transparent 46%), var(--page); color: var(--text); font: 15px/1.5 system-ui, sans-serif; }
html[data-theme="light"] body { background: radial-gradient(ellipse at 50% -18%, #e5edff 0, transparent 46%), var(--page); }
main { width: min(380px, 90vw); padding: 24px; border: 1px solid var(--line); border-radius: 14px; background: var(--surface); box-shadow: 0 12px 40px rgba(0,0,0,.18); }
h1 { margin: 0 0 16px; font-size: 17px; }
label { display: block; margin: 14px 0 6px; color: var(--muted); font-size: 13px; }
form input, form button { width: 100%; padding: 11px; border: 1px solid var(--line); border-radius: 8px; background: var(--input); color: var(--text); font: inherit; }
form button { margin-top: 18px; border-color: var(--accent); background: #315eea; color: white; font-weight: 700; cursor: pointer; }
main p { color: var(--danger); font-size: 13px; }
.theme-toggle { position: fixed; top: 16px; right: 16px; display: inline-flex; min-height: 36px; align-items: center; gap: 7px; padding: 0 11px; border: 1px solid var(--line); border-radius: 9px; background: var(--surface); color: var(--text); font-size: 11px; font-weight: 650; cursor: pointer; }
.theme-toggle-icon { font-size: 15px; }
"#;

const FOLDER_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h6.9a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5.5a2 2 0 0 1-2-2z"/><path d="M3.8 10h16.4"/></svg>"#;
const FILE_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M7 3.8h6.5l4.7 4.7v11.7H7a2 2 0 0 1-2-2v-12a2 2 0 0 1 2-2.4z"/><path d="M13 4v5h5M8.5 13h7M8.5 16.5h7"/></svg>"#;

const UPLOAD_PANEL_TEMPLATE: &str = r#"<section class="upload-panel" aria-labelledby="upload-title"><div class="upload-heading"><div><h2 id="upload-title">上传到收件目录</h2><p>上传文件保存在独立收件目录，同名文件不会覆盖。</p></div><span class="upload-limit">单文件最多 MAX_UPLOAD_MB MB · 收件目录最多 MAX_TOTAL_MB MB</span></div><div class="upload-controls"><div class="upload-selection"><div class="upload-pickers"><button id="pick-images" class="file-pick-button" type="button" aria-label="&#36873;&#25321;&#22270;&#29255;">&#36873;&#25321;&#22270;&#29255;</button><button id="pick-files" class="file-pick-button" type="button" aria-label="&#36873;&#25321;&#25991;&#20214;">&#36873;&#25321;&#25991;&#20214;</button><button id="clear-files" class="file-picker-clear" type="button" hidden>&#28165;&#31354;&#36873;&#25321;</button></div><div id="file-label" class="file-picker-label" aria-live="polite">&#35831;&#36873;&#25321;&#22270;&#29255;&#25110;&#25991;&#20214;</div><input id="images" class="file-input-hidden" type="file" accept="image/*" multiple hidden aria-label="&#36873;&#25321;&#22270;&#29255;"><input id="files" class="file-input-hidden" type="file" multiple hidden aria-label="&#36873;&#25321;&#25991;&#20214;"></div><button id="upload" class="primary-button" type="button" disabled>开始上传</button></div><div id="status" class="upload-status" role="status" aria-live="polite"></div><script>
const imageInput=document.getElementById('images');
const input=document.getElementById('files');
const fileLabel=document.getElementById('file-label');
const button=document.getElementById('upload');
const status=document.getElementById('status');
const pickImages=document.getElementById('pick-images');
const pickFiles=document.getElementById('pick-files');
const clearFiles=document.getElementById('clear-files');
const selectedFiles=[];
const selectedKeys=new Set();
function updateSelection(){const names=selectedFiles.slice(0,4).map(file=>file.name).join('\u3001');fileLabel.textContent=selectedFiles.length?'\u5df2\u9009 '+selectedFiles.length+' \u4e2a\u6587\u4ef6\uff1a'+names+(selectedFiles.length>4?'\u7b49':''):'\u8bf7\u9009\u62e9\u56fe\u7247\u6216\u6587\u4ef6';button.disabled=selectedFiles.length===0;clearFiles.hidden=selectedFiles.length===0}
function addSelectedFiles(source){const files=Array.from(source.files||[]);source.value='';for(const file of files){const key=[file.name,file.size,file.lastModified,file.type].join('|');if(!selectedKeys.has(key)){selectedKeys.add(key);selectedFiles.push(file)}}updateSelection();status.replaceChildren()}
function setPickerDisabled(disabled){pickImages.disabled=disabled;pickFiles.disabled=disabled;clearFiles.disabled=disabled}
pickImages.addEventListener('click',()=>imageInput.click());
pickFiles.addEventListener('click',()=>input.click());
imageInput.addEventListener('change',()=>addSelectedFiles(imageInput));
input.addEventListener('change',()=>addSelectedFiles(input));
clearFiles.addEventListener('click',()=>{selectedFiles.length=0;selectedKeys.clear();status.replaceChildren();updateSelection()});
const maxUploadBytes=MAX_UPLOAD_BYTES;
const csrfToken='CSRF_TOKEN';
function report(message,state=''){const row=document.createElement('p');row.className='upload-result '+state;row.textContent=message;status.appendChild(row)}
function formatBytes(value){if(!Number.isFinite(value)||value<=0)return '0 B';const units=['B','KB','MB','GB','TB'];const tier=Math.min(Math.floor(Math.log(value)/Math.log(1024)),units.length-1);const amount=value/Math.pow(1024,tier);return amount.toFixed(tier===0||amount>=100?0:1)+' '+units[tier]}
function formatRemaining(seconds){if(!Number.isFinite(seconds)||seconds<0)return '计算中';if(seconds<60)return Math.ceil(seconds)+' 秒';const minutes=Math.floor(seconds/60);const rest=Math.ceil(seconds%60);if(minutes<60)return minutes+' 分 '+rest+' 秒';return Math.floor(minutes/60)+' 小时 '+(minutes%60)+' 分'}
function createProgressRow(file,index,count){const row=document.createElement('div');row.className='upload-result progress';const head=document.createElement('div');head.className='upload-progress-head';const name=document.createElement('span');name.className='upload-progress-name';name.textContent='上传中（'+(index+1)+'/'+count+'）：'+file.name;const meta=document.createElement('span');meta.className='upload-progress-meta';meta.textContent='等待发送 · 0 B / '+formatBytes(file.size);head.append(name,meta);const track=document.createElement('div');track.className='upload-progress-track';track.setAttribute('role','progressbar');track.setAttribute('aria-label','上传 '+file.name);track.setAttribute('aria-valuemin','0');track.setAttribute('aria-valuemax','100');track.setAttribute('aria-valuenow','0');const fill=document.createElement('div');fill.className='upload-progress-fill';track.append(fill);row.append(head,track);status.appendChild(row);return{row,name,meta,track,fill}}
function uploadFile(file,url,onProgress){return new Promise((resolve,reject)=>{const request=new XMLHttpRequest();request.open('PUT',url);if(csrfToken)request.setRequestHeader('X-SW-CSRF-Token',csrfToken);request.upload.addEventListener('progress',onProgress);request.addEventListener('load',()=>resolve({status:request.status,body:request.responseText}));request.addEventListener('error',()=>reject(new Error('网络连接中断，未收到服务器响应')));request.addEventListener('abort',()=>reject(new Error('上传已取消')));request.send(file)})}
button.addEventListener('click',async()=>{const files=selectedFiles.slice();if(!files.length){report('请先选择文件。','error');return}status.replaceChildren();button.disabled=true;setPickerDisabled(true);button.textContent='正在上传…';const base='UPLOAD_BASE';try{for(let index=0;index<files.length;index++){const file=files[index];if(file.size>maxUploadBytes){report('已跳过 '+file.name+'：超过单文件上限。','error');continue}const progress=createProgressRow(file,index,files.length);const startedAt=performance.now();try{const response=await uploadFile(file,new URL(base+encodeURIComponent(file.name),location.origin),event=>{const loaded=Math.min(event.loaded,file.size);const total=event.lengthComputable?event.total:file.size;const percent=total>0?Math.min(100,Math.floor(loaded/total*100)):0;const elapsed=Math.max((performance.now()-startedAt)/1000,0.001);const speed=loaded/elapsed;const remaining=speed>0&&total>loaded?(total-loaded)/speed:0;progress.fill.style.width=percent+'%';progress.track.setAttribute('aria-valuenow',String(percent));progress.meta.textContent='已发送 '+formatBytes(loaded)+' / '+formatBytes(total)+' · '+percent+'% · '+formatBytes(speed)+'/秒 · 剩余 '+formatRemaining(remaining)});let detail=response.body.trim();if(response.status<200||response.status>=300){if(/<!doctype html/i.test(response.body)){const page=new DOMParser().parseFromString(response.body,'text/html');const code=page.querySelector('.code-label')?.textContent?.trim()||('HTTP '+response.status);const ray=response.body.match(/Cloudflare Ray ID:\s*(?:<strong[^>]*>)?([^<\s]+)/i)?.[1];detail=[code,page.title?.trim(),ray?'Ray ID '+ray:''].filter(Boolean).join(' · ')}else if(!detail){detail='HTTP '+response.status}progress.row.className='upload-result error';progress.meta.textContent='上传失败 · '+detail}else{progress.row.className='upload-result success';progress.name.textContent='上传完成：'+file.name;progress.fill.style.width='100%';progress.track.setAttribute('aria-valuenow','100');progress.meta.textContent='完成 · '+formatBytes(file.size)}}catch(error){progress.row.className='upload-result error';progress.meta.textContent='上传失败 · '+String(error)}}}finally{button.disabled=selectedFiles.length===0;setPickerDisabled(false);button.textContent='重新上传'}});
</script></section>"#;

const IMAGE_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3.5" y="4.5" width="17" height="15" rx="2"/><circle cx="9" cy="9" r="1.5"/><path d="m4 17 5-5 3 3 2-2 6 5"/></svg>"#;
const VIDEO_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2"/><path d="m10 9 5 3-5 3z"/></svg>"#;
const AUDIO_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 18V5l11-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="17" cy="16" r="3"/><path d="M9 8 20 6"/></svg>"#;
const ARCHIVE_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m4 7 8-4 8 4-8 4zM4 7v10l8 4 8-4V7M12 11v10"/><path d="M10 6h4"/></svg>"#;
const SPREADSHEET_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="2"/><path d="M4 9h16M9 4v16m5-16v16M4 14h16"/></svg>"#;
const PRESENTATION_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8m-4-4v4m-5-5 3-4 2 2 3-4 3 4"/></svg>"#;
const FONT_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m4 19 6-14 6 14M6 14h8m1-5h5m-2.5-3v13m-2 0h4"/></svg>"#;
const CODE_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m8 8-4 4 4 4m8-8 4 4-4 4m-3-11-2 14"/></svg>"#;
const APP_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="3"/><path d="M8 9h8M8 13h3m-3 3h8"/></svg>"#;

fn file_type_details(name: &str) -> (&'static str, &'static str, String) {
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let category = match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "heic"
        | "heif" | "avif" | "tif" | "tiff" => ("image", "图片"),
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "wmv" | "mpeg" | "mpg" => {
            ("video", "视频")
        }
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "opus" | "wma" => ("audio", "音频"),
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "tgz" => {
            ("archive", "压缩文件")
        }
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "txt" | "md" => ("document", "文档"),
        "xls" | "xlsx" | "ods" | "csv" | "tsv" => ("spreadsheet", "表格"),
        "ppt" | "pptx" | "odp" => ("presentation", "演示文稿"),
        "rs" | "js" | "mjs" | "cjs" | "ts" | "jsx" | "tsx" | "py" | "java" | "c" | "h" | "cpp"
        | "hpp" | "go" | "rb" | "php" | "html" | "css" | "scss" | "json" | "yaml" | "yml"
        | "toml" | "xml" | "sh" | "ps1" | "sql" => ("code", "代码"),
        "exe" | "msi" | "msp" | "appx" | "msix" | "apk" | "deb" | "rpm" | "dmg" | "pkg" | "bat"
        | "cmd" => ("application", "程序"),
        "ttf" | "otf" | "woff" | "woff2" => ("font", "字体"),
        _ => ("other", "文件"),
    };
    (category.0, category.1, extension)
}

fn file_type_icon(category: &str) -> &'static str {
    match category {
        "folder" => FOLDER_ICON,
        "image" => IMAGE_ICON,
        "video" => VIDEO_ICON,
        "audio" => AUDIO_ICON,
        "archive" => ARCHIVE_ICON,
        "spreadsheet" => SPREADSHEET_ICON,
        "presentation" => PRESENTATION_ICON,
        "font" => FONT_ICON,
        "code" => CODE_ICON,
        "application" => APP_ICON,
        _ => FILE_ICON,
    }
}

fn upload_panel(
    max_upload_bytes: u64,
    max_total_bytes: u64,
    prefix: &str,
    csrf_token: &str,
) -> String {
    let base = if prefix.is_empty() {
        "/".to_string()
    } else {
        format!("{prefix}/")
    };
    UPLOAD_PANEL_TEMPLATE
        .replace("UPLOAD_BASE", &base)
        .replace("MAX_UPLOAD_BYTES", &max_upload_bytes.to_string())
        .replace("CSRF_TOKEN", csrf_token)
        .replace(
            "MAX_UPLOAD_MB",
            &(max_upload_bytes / (1024 * 1024)).to_string(),
        )
        .replace(
            "MAX_TOTAL_MB",
            &(max_total_bytes / (1024 * 1024)).to_string(),
        )
}

struct RequestHead {
    method: String,
    target: String,
    protocol: String,
    host: String,
    content_length: Option<usize>,
    transfer_encoding: bool,
    content_type: String,
    user_agent: String,
    referer: String,
    cookie: String,
    origin: String,
    fetch_site: String,
    csrf_token: String,
    range: Option<String>,
    cf_connecting_ip: Option<IpAddr>,
}

enum RequestReadError {
    TooLarge,
    Malformed,
    ConnectionClosed,
    Io(io::Error),
}

fn read_request_head<S: HttpStream>(stream: &mut S) -> Result<RequestHead, RequestReadError> {
    let mut bytes = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        let count = stream.read(&mut byte).map_err(RequestReadError::Io)?;
        if count == 0 {
            return Err(if bytes.is_empty() {
                RequestReadError::ConnectionClosed
            } else {
                RequestReadError::Malformed
            });
        }
        bytes.push(byte[0]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            bytes.truncate(end);
            break;
        }
        if bytes.len() >= MAX_HEADER_BYTES {
            return Err(RequestReadError::TooLarge);
        }
    }

    let head = String::from_utf8_lossy(&bytes);
    let mut lines = head.split("\r\n");
    let mut request = lines.next().unwrap_or_default().split_whitespace();
    let method = request.next().unwrap_or_default();
    let target = request.next().unwrap_or_default();
    let version = request.next().unwrap_or_default();
    let extra = request.next();
    if method.is_empty()
        || target.is_empty()
        || !(version == "HTTP/1.0" || version == "HTTP/1.1")
        || extra.is_some()
        || method.bytes().any(|byte| !byte.is_ascii_uppercase())
    {
        return Err(RequestReadError::Malformed);
    }
    let mut content_length = None;
    let mut transfer_encoding = false;
    let mut cf_connecting_ip = None;
    let mut host = String::new();
    let mut content_type = String::new();
    let mut user_agent = String::new();
    let mut referer = String::new();
    let mut cookie = String::new();
    let mut origin = String::new();
    let mut fetch_site = String::new();
    let mut csrf_token = String::new();
    let mut range = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(RequestReadError::Malformed);
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| RequestReadError::Malformed)?;
            if content_length.replace(parsed).is_some() {
                return Err(RequestReadError::Malformed);
            }
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            if transfer_encoding || !value.eq_ignore_ascii_case("chunked") {
                return Err(RequestReadError::Malformed);
            }
            transfer_encoding = true;
        } else if name.eq_ignore_ascii_case("cf-connecting-ip") {
            if cf_connecting_ip.is_some() {
                return Err(RequestReadError::Malformed);
            }
            cf_connecting_ip = Some(
                value
                    .parse::<IpAddr>()
                    .map_err(|_| RequestReadError::Malformed)?,
            );
        } else if name.eq_ignore_ascii_case("host") {
            host = value.to_string();
        } else if name.eq_ignore_ascii_case("content-type") {
            content_type = value.to_string();
        } else if name.eq_ignore_ascii_case("user-agent") {
            user_agent = value.to_string();
        } else if name.eq_ignore_ascii_case("referer") {
            referer = value.to_string();
        } else if name.eq_ignore_ascii_case("cookie") {
            cookie = value.to_string();
        } else if name.eq_ignore_ascii_case("origin") {
            origin = value.to_string();
        } else if name.eq_ignore_ascii_case("sec-fetch-site") {
            fetch_site = value.to_string();
        } else if name.eq_ignore_ascii_case("x-sw-csrf-token") {
            csrf_token = value.to_string();
        } else if name.eq_ignore_ascii_case("range") {
            if range.replace(value.to_string()).is_some() {
                return Err(RequestReadError::Malformed);
            }
        }
    }
    if transfer_encoding && content_length.is_some() {
        return Err(RequestReadError::Malformed);
    }
    Ok(RequestHead {
        method: method.to_string(),
        target: target.to_string(),
        protocol: version.to_string(),
        host,
        content_length,
        transfer_encoding,
        content_type,
        user_agent,
        referer,
        cookie,
        origin,
        fetch_site,
        csrf_token,
        range,
        cf_connecting_ip,
    })
}

fn parse_byte_range(header: &str, file_len: u64) -> Result<(u64, u64), ()> {
    let (unit, range) = header.split_once('=').ok_or(())?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return Err(());
    }
    if range.contains(',') {
        return Err(());
    }
    let (start, end) = range.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix_len = end.parse::<u64>().map_err(|_| ())?;
        if suffix_len == 0 || file_len == 0 {
            return Err(());
        }
        return Ok((file_len.saturating_sub(suffix_len), file_len - 1));
    }

    let start = start.parse::<u64>().map_err(|_| ())?;
    if start >= file_len {
        return Err(());
    }
    let end = if end.is_empty() {
        file_len - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(file_len - 1)
    };
    if start > end {
        return Err(());
    }
    Ok((start, end))
}

fn decode_share_path(target: &str, token: &str, require_token: bool) -> Option<PathBuf> {
    let raw_path = target.split('?').next()?;
    let raw_path = raw_path.strip_prefix('/')?;
    let mut segments = raw_path.split('/');
    if require_token && decode_segment(segments.next()?)? != token {
        return None;
    }
    if !require_token && raw_path.starts_with("__auth/") {
        return None;
    }
    let mut relative = PathBuf::new();
    for segment in segments.filter(|segment| !segment.is_empty()) {
        let decoded = decode_segment(segment)?;
        if decoded == "."
            || decoded == ".."
            || decoded.contains('/')
            || decoded.contains('\\')
            || decoded.contains(':')
            || decoded.chars().any(char::is_control)
        {
            return None;
        }
        relative.push(decoded);
    }
    Some(relative)
}

fn decode_segment(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hi = hex_value(*bytes.get(index + 1)?)?;
            let lo = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((hi << 4) | lo);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct IpNetwork {
    address: IpAddr,
    prefix: u8,
}

pub fn validate_allowed_ips(entries: &[String]) -> Result<(), String> {
    parse_allowed_ips(entries).map(|_| ())
}

fn parse_allowed_ips(entries: &[String]) -> Result<Vec<IpNetwork>, String> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.trim().is_empty())
        .map(|(index, entry)| {
            let value = entry.trim();
            let (address_text, prefix_text) = match value.split_once('/') {
                Some((address, prefix)) => (address, Some(prefix)),
                None => (value, None),
            };
            let address = address_text
                .parse::<IpAddr>()
                .map_err(|_| format!("白名单第 {} 项不是有效 IP 或 CIDR：{value}", index + 1))?;
            let max_prefix = if address.is_ipv4() { 32 } else { 128 };
            let prefix = match prefix_text {
                Some(text) => text
                    .parse::<u8>()
                    .map_err(|_| format!("白名单第 {} 项的 CIDR 前缀无效：{value}", index + 1))?,
                None => max_prefix,
            };
            if prefix > max_prefix {
                return Err(format!(
                    "白名单第 {} 项的 CIDR 前缀不能超过 {max_prefix}：{value}",
                    index + 1
                ));
            }
            Ok(IpNetwork { address, prefix })
        })
        .collect()
}

fn directory_upload_size(directory: &Path) -> io::Result<u64> {
    fs::read_dir(directory)?.try_fold(0u64, |total, entry| {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Ok(total);
        }
        let size = entry.metadata()?.len();
        total
            .checked_add(size)
            .ok_or_else(|| io::Error::other("upload directory size overflow"))
    })
}

impl IpNetwork {
    fn contains(self, candidate: IpAddr) -> bool {
        match (self.address, candidate) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - self.prefix)
                };
                u32::from(network) & mask == u32::from(address) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - self.prefix)
                };
                u128::from(network) & mask == u128::from(address) & mask
            }
            _ => false,
        }
    }
}

fn safe_upload_filename(value: &str) -> String {
    let mut filename = value
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
        })
        .take(180)
        .collect::<String>()
        .trim_matches(|character| character == ' ' || character == '.')
        .to_string();
    let device_name = filename
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || device_name
            .strip_prefix("COM")
            .or_else(|| device_name.strip_prefix("LPT"))
            .is_some_and(|number| {
                matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
    {
        filename.insert(0, '_');
    }
    if filename.starts_with(".sideweave-upload-") && filename.ends_with(".part") {
        filename.insert(0, '_');
    }
    filename
}

fn directory_listing(
    root: &Path,
    directory: &Path,
    relative: &Path,
    token: &str,
    csrf_token: &str,
    upload_limits: Option<(u64, u64)>,
) -> io::Result<String> {
    let mut entries = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".sideweave-upload-") && name.ends_with(".part") {
                return None;
            }
            let path = entry.path();
            let canonical = fs::canonicalize(&path).ok()?;
            if !canonical.starts_with(root) {
                return None;
            }
            let metadata = fs::metadata(&canonical).ok()?;
            let modified = metadata
                .modified()
                .map(|time| {
                    chrono::DateTime::<Local>::from(time)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|_| "未知".to_string());
            Some((name, metadata.is_dir(), metadata.len(), modified))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.to_lowercase().cmp(&right.0.to_lowercase()))
    });

    let title = if relative.as_os_str().is_empty() {
        "共享文件".to_string()
    } else {
        relative.to_string_lossy().replace('\\', "/")
    };
    let escaped_title = html_escape(&title);
    let root_href = listing_href(token, Path::new(""), true);
    let mut html = String::from(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\"><meta name=\"referrer\" content=\"no-referrer\"><meta name=\"theme-color\" content=\"#0e1420\"><title>",
    );
    html.push_str(&escaped_title);
    html.push_str(" · 文件共享</title>");
    html.push_str(THEME_BOOTSTRAP_SCRIPT);
    html.push_str("<style>");
    html.push_str(SHARE_PAGE_STYLE);
    html.push_str("</style></head><body><div class=\"page-shell\"><header class=\"sitebar\"><a class=\"brand\" href=\"");
    html.push_str(&root_href);
    html.push_str("\"><span class=\"brand-mark\" aria-hidden=\"true\">SW</span><span>SideWeave</span></a><span class=\"share-badge\">文件共享</span>");
    if token.is_empty() {
        html.push_str("<form method=\"post\" action=\"/__auth/logout\"><button type=\"submit\" style=\"padding:7px 10px;border:1px solid var(--line);border-radius:8px;background:var(--surface);color:var(--text);cursor:pointer\">退出登录</button></form>");
    }
    html.push_str(THEME_TOGGLE_BUTTON);
    html.push_str("</header><main class=\"page-main\">");
    if !relative.as_os_str().is_empty() {
        append_breadcrumbs(&mut html, relative, token);
    }
    if let Some((max_upload_bytes, max_total_bytes)) = upload_limits {
        html.push_str(&upload_panel(
            max_upload_bytes,
            max_total_bytes,
            token,
            csrf_token,
        ));
    }
    html.push_str("<section class=\"file-browser\" aria-labelledby=\"file-list-title\"><div class=\"list-heading\"><h2 id=\"file-list-title\">文件和文件夹</h2><span class=\"item-count\">");
    html.push_str(&format!("{} 项", entries.len()));
    html.push_str("</span></div><div class=\"file-tools\"><input id=\"file-search\" class=\"file-search\" type=\"search\" placeholder=\"搜索当前文件夹\" aria-label=\"搜索文件\"><select id=\"file-type-filter\" class=\"file-type-filter\" aria-label=\"按文件类型筛选\"><option value=\"all\">所有类型</option><option value=\"folder\">文件夹</option><option value=\"image\">图片</option><option value=\"video\">视频</option><option value=\"audio\">音频</option><option value=\"document\">文档</option><option value=\"spreadsheet\">表格</option><option value=\"presentation\">演示文稿</option><option value=\"archive\">压缩文件</option><option value=\"application\">程序</option><option value=\"code\">代码</option><option value=\"font\">字体</option><option value=\"other\">其他</option></select></div><div class=\"file-list\">");
    if !relative.as_os_str().is_empty() {
        let parent_href = listing_href(
            token,
            relative.parent().unwrap_or_else(|| Path::new("")),
            true,
        );
        html.push_str("<a class=\"file-row parent-row\" href=\"");
        html.push_str(&parent_href);
        html.push_str("\"><span class=\"file-icon\" aria-hidden=\"true\">←</span><span class=\"file-main\"><span class=\"file-name\">返回上一级</span><span class=\"file-kind\">上级文件夹</span></span><span class=\"file-meta\"></span><span class=\"file-arrow\" aria-hidden=\"true\">↰</span></a>");
    }
    if entries.is_empty() {
        html.push_str("<div class=\"empty-state\"><span class=\"empty-icon\" aria-hidden=\"true\">⌑</span><strong>这个文件夹是空的</strong><span>当前目录没有可浏览的文件。</span></div>");
    }
    for (name, is_dir, size, modified) in entries {
        let encoded_relative = relative.join(&name);
        let href = listing_href(token, &encoded_relative, is_dir);
        let label = html_escape(&name);
        let modified_label = html_escape(&modified);
        let search_name = html_escape(&name.to_lowercase());
        if is_dir {
            html.push_str("<a class=\"file-row\" data-kind=\"folder\" data-name=\"");
            html.push_str(&search_name);
            html.push_str("\" href=\"");
            html.push_str(&html_escape(&href));
            html.push_str("\" aria-label=\"打开文件夹：");
            html.push_str(&label);
            html.push_str("\"><span class=\"file-icon folder\" aria-hidden=\"true\">");
            html.push_str(file_type_icon("folder"));
            html.push_str("</span><span class=\"file-main\"><span class=\"file-name\">");
            html.push_str(&label);
            html.push_str("</span><span class=\"file-kind\">打开文件夹</span></span><span class=\"file-meta\"><span>文件夹</span><time>");
            html.push_str(&modified_label);
            html.push_str(
                "</time></span><span class=\"file-arrow\" aria-hidden=\"true\">›</span></a>",
            );
            continue;
        }

        let (category, type_label, extension) = file_type_details(&name);
        let size_label = format_size(size);
        let preview_kind = if is_previewable_image(Path::new(&name)) {
            Some("image")
        } else if is_previewable_text(Path::new(&name)) {
            Some("text")
        } else if is_previewable_video(Path::new(&name)) {
            Some("video")
        } else if is_previewable_audio(Path::new(&name)) {
            Some("audio")
        } else {
            None
        };
        let preview_href = preview_kind.map(|_| format!("{href}?sw_preview=1"));
        let display_path = encoded_relative.to_string_lossy().replace('\\', "/");
        html.push_str("<div class=\"file-row\" data-kind=\"");
        html.push_str(category);
        html.push_str("\" data-name=\"");
        html.push_str(&search_name);
        html.push_str("\"><button class=\"file-details-trigger\" type=\"button\" data-file-url=\"");
        html.push_str(&html_escape(&href));
        html.push_str("\" data-file-name=\"");
        html.push_str(&label);
        html.push_str("\" data-file-type=\"");
        html.push_str(&html_escape(type_label));
        html.push_str("\" data-file-extension=\"");
        html.push_str(&html_escape(&extension));
        html.push_str("\" data-file-size=\"");
        html.push_str(&size.to_string());
        html.push_str("\" data-file-size-label=\"");
        html.push_str(&html_escape(&size_label));
        html.push_str("\" data-file-modified=\"");
        html.push_str(&modified_label);
        html.push_str("\" data-file-path=\"");
        html.push_str(&html_escape(&display_path));
        if let Some(preview_href) = &preview_href {
            html.push_str("\" data-preview-url=\"");
            html.push_str(&html_escape(preview_href));
            html.push_str("\" data-preview-kind=\"");
            html.push_str(preview_kind.unwrap_or("text"));
        }
        html.push_str("\" aria-label=\"查看文件详情：");
        html.push_str(&label);
        html.push_str("\"><span class=\"file-icon ");
        html.push_str(category);
        html.push_str("\" aria-hidden=\"true\">");
        html.push_str(file_type_icon(category));
        html.push_str("</span><span class=\"file-main\"><span class=\"file-name\">");
        html.push_str(&label);
        html.push_str("</span><span class=\"file-kind\">");
        html.push_str(if preview_href.is_some() {
            "查看详情 / 预览"
        } else {
            "查看文件详情"
        });
        html.push_str(" · ");
        html.push_str(&html_escape(type_label));
        if !extension.is_empty() {
            html.push_str(" <span class=\"file-extension\">");
            html.push_str(&html_escape(&format!(".{extension}")));
            html.push_str("</span>");
        }
        html.push_str("</span></span><span class=\"file-meta\"><span>");
        html.push_str(&html_escape(&size_label));
        html.push_str("</span><time>");
        html.push_str(&modified_label);
        html.push_str("</time></span></button>");
        if preview_kind == Some("image") {
            html.push_str("<span class=\"file-action-spacer\" aria-hidden=\"true\"></span></div>");
        } else {
            html.push_str("<a class=\"file-arrow file-download-button\" href=\"");
            html.push_str(&html_escape(&href));
            html.push_str("\" aria-label=\"下载文件：");
            html.push_str(&label);
            html.push_str("\" title=\"下载文件：");
            html.push_str(&label);
            html.push_str("\" download>↓</a></div>");
        }
    }
    html.push_str("</div><div id=\"filter-empty\" class=\"filter-empty\" hidden>没有匹配的文件</div></section></main><footer class=\"page-footer\">通过专属链接安全访问 · SideWeave</footer></div></body></html>");
    html = html.replace(
        "<footer class=\"page-footer\">",
        "<dialog id=\"file-details-dialog\" class=\"file-details-dialog\" aria-labelledby=\"file-details-title\"><div class=\"file-details-shell\"><header class=\"file-details-header\"><div class=\"file-details-heading\"><span class=\"file-details-eyebrow\">文件详情</span><h2 id=\"file-details-title\" class=\"file-details-title\"></h2></div><div class=\"file-details-actions\"><a id=\"file-details-download\" class=\"preview-action\" download>下载</a><button id=\"file-details-close\" class=\"preview-action\" type=\"button\">关闭</button></div></header><div id=\"file-details-body\" class=\"file-details-body\"><dl class=\"file-details-info\"><div><dt>文件类型</dt><dd id=\"file-details-type\"></dd></div><div><dt>文件后缀</dt><dd id=\"file-details-extension\"></dd></div><div><dt>文件大小</dt><dd id=\"file-details-size\"></dd></div><div><dt>修改时间</dt><dd id=\"file-details-modified\"></dd></div><div><dt>位置</dt><dd id=\"file-details-path\"></dd></div></dl><section id=\"file-details-preview\" class=\"file-details-preview\" aria-label=\"文件预览\" hidden><div class=\"file-details-preview-heading\">预览</div><button id=\"file-details-media-load\" class=\"preview-action file-media-load\" type=\"button\" hidden>&#28857;&#20987;&#21518;&#21152;&#36733;&#24182;&#25773;&#25918;</button><img id=\"file-details-image\" alt=\"\" tabindex=\"0\" role=\"button\" title=\"&#28857;&#20987;&#25918;&#22823;&#26597;&#30475;\" hidden><video id=\"file-details-video\" controls playsinline preload=\"none\" aria-label=\"视频预览\" hidden></video><audio id=\"file-details-audio\" controls preload=\"none\" aria-label=\"音频预览\" hidden></audio><textarea id=\"file-details-text\" class=\"file-details-text\" readonly spellcheck=\"false\" aria-label=\"文本文件只读预览\" wrap=\"off\" hidden></textarea><div id=\"file-details-preview-status\" class=\"file-details-preview-status\" hidden></div></section></div></div></dialog><footer class=\"page-footer\">",
    );
    html = html.replace(
        "</body></html>",
        "<script>const fileSearch=document.getElementById('file-search');const fileTypeFilter=document.getElementById('file-type-filter');const visibleCount=document.querySelector('.item-count');const filterEmpty=document.getElementById('filter-empty');const fileRows=Array.from(document.querySelectorAll('.file-row[data-kind]'));function filterFileRows(){const query=fileSearch.value.trim().toLocaleLowerCase();const kind=fileTypeFilter.value;let visible=0;for(const row of fileRows){const matchesName=(row.dataset.name||'').includes(query);const matchesKind=kind==='all'||row.dataset.kind===kind;row.hidden=!(matchesName&&matchesKind);if(!row.hidden)visible++}visibleCount.textContent=visible===fileRows.length?fileRows.length+' 项':visible+' / '+fileRows.length+' 项';filterEmpty.hidden=visible!==0||fileRows.length===0}fileSearch.addEventListener('input',filterFileRows);fileTypeFilter.addEventListener('change',filterFileRows);filterFileRows();</script></body></html>",
    );
    html = html.replace(
        "</dialog><footer class=\"page-footer\">",
        "</dialog><dialog id=\"file-image-zoom\" class=\"file-image-zoom\" aria-label=\"&#22270;&#29255;&#39044;&#35272;\"><button id=\"file-image-zoom-close\" type=\"button\" class=\"file-image-zoom-close\" aria-label=\"&#20851;&#38381;&#22270;&#29255;&#39044;&#35272;\">&#215;</button><img id=\"file-image-zoom-image\" alt=\"\"></dialog><footer class=\"page-footer\">",
    );
    html = html.replace(
        "</body></html>",
        &format!("{FILE_DETAILS_SCRIPT}</body></html>"),
    );
    html = html.replace(
        "</body></html>",
        &format!("{THEME_TOGGLE_SCRIPT}</body></html>"),
    );
    if !csrf_token.is_empty() {
        html = html.replace(
            "<form method=\"post\" action=\"/__auth/logout\">",
            &format!(
                "<form method=\"post\" action=\"/__auth/logout\"><input type=\"hidden\" name=\"csrf_token\" value=\"{csrf_token}\">"
            ),
        );
    }
    Ok(html)
}

fn append_breadcrumbs(html: &mut String, relative: &Path, token: &str) {
    let parts = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    html.push_str("<nav class=\"breadcrumbs\" aria-label=\"文件路径\">");
    if parts.is_empty() {
        html.push_str("<span class=\"crumb-current\" aria-current=\"page\">共享空间</span>");
    } else {
        let root_href = listing_href(token, Path::new(""), true);
        html.push_str("<a href=\"");
        html.push_str(&root_href);
        html.push_str("\">共享空间</a>");
        let mut current = PathBuf::new();
        for (index, part) in parts.iter().enumerate() {
            current.push(part);
            html.push_str("<span class=\"crumb-separator\" aria-hidden=\"true\">/</span>");
            let label = html_escape(part);
            if index + 1 == parts.len() {
                html.push_str("<span class=\"crumb-current\" aria-current=\"page\">");
                html.push_str(&label);
                html.push_str("</span>");
            } else {
                let href = listing_href(token, &current, true);
                html.push_str("<a href=\"");
                html.push_str(&href);
                html.push_str("\">");
                html.push_str(&label);
                html.push_str("</a>");
            }
        }
    }
    html.push_str("</nav>");
}

fn listing_href(token: &str, relative: &Path, is_dir: bool) -> String {
    let mut href = if token.is_empty() {
        "/".to_string()
    } else {
        format!("{token}/")
    };
    for (index, component) in relative.components().enumerate() {
        if index > 0 {
            href.push('/');
        }
        href.push_str(&encode_segment(&component.as_os_str().to_string_lossy()));
    }
    if is_dir && !href.ends_with('/') {
        href.push('/');
    }
    href
}

fn encode_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn format_size(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", size as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "txt" | "text" | "log" | "md" | "markdown" | "csv" | "tsv" | "jsonc" | "yaml" | "yml"
        | "toml" | "ini" | "conf" | "cfg" | "properties" | "xml" | "css" | "scss" | "sass"
        | "less" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "vue" | "svelte" | "rs" | "go"
        | "py" | "rb" | "php" | "java" | "kt" | "kts" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs"
        | "sh" | "bash" | "zsh" | "ps1" | "bat" | "cmd" | "sql" | "graphql" | "gql" | "proto" => {
            "text/plain; charset=utf-8"
        }
        "pdf" => "application/pdf",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "m4a" => "audio/mp4",
        "wma" => "audio/x-ms-wma",
        "mp4" => "video/mp4",
        "m4v" => "video/x-m4v",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "wmv" => "video/x-ms-wmv",
        "mpeg" | "mpg" => "video/mpeg",
        "html" | "htm" => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn is_previewable_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "avif" | "ico"
    )
}

fn is_previewable_text(path: &Path) -> bool {
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        filename.as_str(),
        "dockerfile" | "makefile" | "readme" | "license" | "license.txt" | ".gitignore"
            | ".gitattributes" | ".editorconfig" | ".env"
    ) {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "txt" | "text" | "log" | "md" | "markdown" | "csv" | "tsv" | "json" | "jsonc"
            | "yaml" | "yml" | "toml" | "ini" | "conf" | "cfg" | "properties" | "xml"
            | "html" | "htm" | "css" | "scss" | "sass" | "less" | "js" | "mjs" | "cjs"
            | "ts" | "tsx" | "jsx" | "vue" | "svelte" | "rs" | "go" | "py" | "rb"
            | "php" | "java" | "kt" | "kts" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs"
            | "sh" | "bash" | "zsh" | "ps1" | "bat" | "cmd" | "sql" | "graphql" | "gql"
            | "proto"
    )
}

fn is_previewable_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "mp4" | "m4v" | "mov" | "avi" | "webm" | "ogv" | "mkv" | "wmv" | "mpeg" | "mpg"
    )
}

fn is_previewable_audio(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "oga" | "m4a" | "opus" | "wma"
    )
}

fn is_previewable_asset(path: &Path) -> bool {
    is_previewable_image(path)
        || is_previewable_text(path)
        || is_previewable_video(path)
        || is_previewable_audio(path)
}

fn safe_filename(value: &str) -> String {
    let safe = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ')
        })
        .take(120)
        .collect::<String>();
    if safe.is_empty() {
        "download".into()
    } else {
        safe
    }
}

fn upload_only_page(
    max_upload_bytes: u64,
    max_total_bytes: u64,
    prefix: &str,
    csrf_token: &str,
) -> String {
    let mut html = format!("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>文件上传</title><style>{}</style></head><body><div class=\"page-shell\"><header class=\"sitebar\"><span class=\"brand\">SideWeave</span><span class=\"share-badge\">仅上传</span></header><main class=\"page-main\">", SHARE_PAGE_STYLE);
    html = html.replace(
        "<title>文件上传</title>",
        "<meta name=\"theme-color\" content=\"#0e1420\"><title>文件上传</title>",
    );
    html = html.replace("<style>", &format!("{THEME_BOOTSTRAP_SCRIPT}<style>"));
    html = html.replace("</header>", &format!("{THEME_TOGGLE_BUTTON}</header>"));
    if prefix.is_empty() {
        html.push_str("<form method=\"post\" action=\"/__auth/logout\" class=\"logout-form\"><button type=\"submit\">退出登录</button></form>");
    }
    html.push_str(&upload_panel(
        max_upload_bytes,
        max_total_bytes,
        prefix,
        csrf_token,
    ));
    html.push_str("</main></div></body></html>");
    if !csrf_token.is_empty() {
        html = html.replace(
            "<form method=\"post\" action=\"/__auth/logout\" class=\"logout-form\">",
            &format!(
                "<form method=\"post\" action=\"/__auth/logout\" class=\"logout-form\"><input type=\"hidden\" name=\"csrf_token\" value=\"{csrf_token}\">"
            ),
        );
    }
    html = html.replace(
        "</body></html>",
        &format!("{THEME_TOGGLE_SCRIPT}</body></html>"),
    );
    html
}

fn send_response_with_headers<S: HttpStream>(
    mut stream: S,
    status: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    stop: &AtomicBool,
    extra_headers: &[(&str, &str)],
) -> io::Result<()> {
    set_access_status(status);
    set_access_response_content_type(content_type);
    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store, no-transform\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; img-src 'self'; media-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'self'\r\n", body.len())?;
    for (name, value) in extra_headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "\r\n")?;
    if !head_only && !stop.load(Ordering::Relaxed) {
        stream.write_all(body)?;
        add_access_sent_bytes(body.len());
    }
    Ok(())
}

fn send_response<S: HttpStream>(
    mut stream: S,
    status: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    stop: &AtomicBool,
) -> io::Result<()> {
    set_access_status(status);
    set_access_response_content_type(content_type);
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store, no-transform\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; img-src 'self'; media-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'self'\r\n\r\n",
        body.len()
    )?;
    if !head_only && !stop.load(Ordering::Relaxed) {
        stream.write_all(body)?;
        add_access_sent_bytes(body.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    struct RunningShares(Vec<String>);

    impl Drop for RunningShares {
        fn drop(&mut self) {
            for id in &self.0 {
                stop_id(id);
            }
        }
    }

    fn test_share(
        id: &str,
        token: &str,
        port: u16,
        view_directory: &Path,
        receive_directory: &Path,
        allow_view: bool,
        allow_upload: bool,
        lan_enabled: bool,
    ) -> FileShareConfig {
        FileShareConfig {
            id: id.into(),
            name: id.into(),
            enabled: true,
            view_directory: view_directory.to_string_lossy().into_owned(),
            receive_directory: receive_directory.to_string_lossy().into_owned(),
            token: token.into(),
            port,
            lan_enabled,
            allow_view,
            allow_upload,
            require_login: false,
            max_upload_mb: 1,
            max_total_upload_mb: 8,
            ..FileShareConfig::default()
        }
    }

    fn http_request(port: u16, method: &str, target: &str, body: &[u8]) -> String {
        http_request_with_headers(port, method, target, body, &[])
    }

    fn http_request_with_headers(
        port: u16,
        method: &str,
        target: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream, "{method} {target} HTTP/1.1\r\nHost: localhost\r\n").unwrap();
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        write!(
            stream,
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
        let mut response = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => response.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::ConnectionReset => break,
                Err(error) => panic!("Failed to read HTTP response: {error}"),
            }
        }
        String::from_utf8_lossy(&response).into_owned()
    }

    fn response_body(response: &str) -> &str {
        response.split_once("\r\n\r\n").unwrap().1
    }

    #[test]
    fn password_hash_uses_a_salt_and_verifies_passwords() {
        let password = "correct horse battery staple";
        let first = hash_password(password).unwrap();
        let second = hash_password(password).unwrap();

        assert_ne!(first, second);
        assert!(password_hash_matches(&first, password));
        assert!(!password_hash_matches(&first, "a different password"));
    }

    #[test]
    fn password_hash_rejects_malformed_and_unknown_formats() {
        assert!(!password_hash_matches("md5$deadbeef", "password"));
        assert!(!password_hash_matches(
            "pbkdf2-sha256$999999$00$00",
            "password"
        ));
        assert!(!password_hash_matches(
            "pbkdf2-sha256$600000$not-hex$00",
            "password"
        ));
    }

    #[test]
    fn login_cookies_are_scoped_to_each_instance() {
        assert_ne!(
            session_cookie_name("share-a"),
            session_cookie_name("share-b")
        );
        assert_eq!(
            session_token_from_cookie("other=x; swfs_token=abc123", "swfs_token"),
            Some("abc123")
        );
        assert_eq!(
            session_token_from_cookie("swfs_other=abc123", "swfs_token"),
            None
        );
    }

    #[test]
    fn sessions_expire_and_auth_epoch_invalidates_old_credentials() {
        let now = Instant::now();
        let epoch = AUTH_EPOCH.load(Ordering::Relaxed);
        let cookie_name = session_cookie_name("share-session-test");
        let sessions = Arc::new(Mutex::new(HashMap::from([
            (
                "valid".to_string(),
                LoginSession {
                    expires: now + Duration::from_secs(60),
                    auth_epoch: epoch,
                },
            ),
            (
                "expired".to_string(),
                LoginSession {
                    expires: now - Duration::from_secs(1),
                    auth_epoch: epoch,
                },
            ),
            (
                "stale-password".to_string(),
                LoginSession {
                    expires: now + Duration::from_secs(60),
                    auth_epoch: epoch.wrapping_sub(1),
                },
            ),
        ])));

        assert!(find_session(&format!("{cookie_name}=valid"), &cookie_name, &sessions).is_some());
        assert!(find_session(&format!("{cookie_name}=expired"), &cookie_name, &sessions).is_none());
        assert!(find_session(
            &format!("{cookie_name}=stale-password"),
            &cookie_name,
            &sessions
        )
        .is_none());
        assert_eq!(sessions.lock().unwrap().len(), 1);
    }

    #[test]
    fn login_failures_trigger_and_clear_the_rate_limit() {
        let ip = format!("test-{}", uuid::Uuid::new_v4());
        assert!(!login_rate_limited(&ip));
        for _ in 0..5 {
            record_login_failure(&ip);
        }
        assert!(login_rate_limited(&ip));
        clear_login_failures(&ip);
        assert!(!login_rate_limited(&ip));
    }

    #[test]
    fn share_paths_require_tokens_only_for_token_mode_and_reject_traversal() {
        assert_eq!(
            decode_share_path(
                "/0123456789abcdef0123456789abcdef/docs/readme.txt",
                "0123456789abcdef0123456789abcdef",
                true
            ),
            Some(PathBuf::from("docs/readme.txt"))
        );
        assert!(decode_share_path(
            "/wrong-token/file.txt",
            "0123456789abcdef0123456789abcdef",
            true
        )
        .is_none());
        assert_eq!(
            decode_share_path("/docs/readme.txt", "", false),
            Some(PathBuf::from("docs/readme.txt"))
        );
        assert!(decode_share_path("/docs/%2e%2e/secret.txt", "", false).is_none());
        assert!(decode_share_path("/docs%2fsecret.txt", "", false).is_none());
    }

    #[test]
    fn parallel_instances_enforce_permissions_and_keep_directories_separate() {
        let test_root = std::env::temp_dir().join(format!(
            "sideweave-file-share-test-{}",
            uuid::Uuid::new_v4()
        ));
        let a_view = test_root.join("a-view");
        let b_view = test_root.join("b-view");
        let b_receive = test_root.join("b-receive");
        let c_directory = test_root.join("c-directory");
        let d_view = test_root.join("d-view");
        let d_receive = test_root.join("d-receive");
        for directory in [
            &a_view,
            &b_view,
            &b_receive,
            &c_directory,
            &d_view,
            &d_receive,
        ] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(a_view.join("read-only.txt"), "share A").unwrap();
        fs::write(b_view.join("browse.txt"), "view B").unwrap();
        fs::write(d_view.join("private.txt"), "not exposed").unwrap();

        let reserved_ports = (0..4)
            .map(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap())
            .collect::<Vec<_>>();
        let ports = reserved_ports
            .iter()
            .map(|listener| listener.local_addr().unwrap().port())
            .collect::<Vec<_>>();
        drop(reserved_ports);

        let mut shares = vec![
            test_share(
                "share-a",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ports[0],
                &a_view,
                &a_view,
                true,
                false,
                false,
            ),
            test_share(
                "share-b",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ports[1],
                &b_view,
                &b_receive,
                true,
                true,
                false,
            ),
            test_share(
                "share-c",
                "cccccccccccccccccccccccccccccccc",
                ports[2],
                &c_directory,
                &c_directory,
                true,
                true,
                true,
            ),
            test_share(
                "share-d",
                "dddddddddddddddddddddddddddddddd",
                ports[3],
                &d_view,
                &d_receive,
                false,
                true,
                false,
            ),
        ];
        let mut running = RunningShares(Vec::new());
        for share in &mut shares {
            start(share).unwrap();
            running.0.push(share.id.clone());
        }

        assert!(shares.iter().all(|share| is_running(&share.id)));
        assert!(lan_is_running("share-c"));
        assert!(!lan_is_running("share-b"));
        let readonly = http_request(
            ports[0],
            "GET",
            "/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/read-only.txt",
            &[],
        );
        assert!(readonly.starts_with("HTTP/1.1 200 OK"));
        assert!(readonly.ends_with("share A"));
        let missing_token = http_request(ports[0], "GET", "/read-only.txt", &[]);
        assert!(missing_token.starts_with("HTTP/1.1 404 Not Found"));
        let rejected_upload = http_request(
            ports[0],
            "PUT",
            "/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/blocked.txt",
            b"blocked",
        );
        assert!(rejected_upload.starts_with("HTTP/1.1 405 Method Not Allowed"));

        let uploaded_separately = http_request(
            ports[1],
            "PUT",
            "/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/received.txt",
            b"received",
        );
        assert!(uploaded_separately.starts_with("HTTP/1.1 201 Created"));
        assert_eq!(
            fs::read(b_receive.join("received.txt")).unwrap(),
            b"received"
        );
        let separate_listing =
            http_request(ports[1], "GET", "/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/", &[]);
        assert!(response_body(&separate_listing).contains("browse.txt"));
        assert!(!response_body(&separate_listing).contains("received.txt"));

        let uploaded_in_view = http_request(
            ports[2],
            "PUT",
            "/cccccccccccccccccccccccccccccccc/visible.txt",
            b"visible",
        );
        assert!(uploaded_in_view.starts_with("HTTP/1.1 201 Created"));
        let nested_listing =
            http_request(ports[2], "GET", "/cccccccccccccccccccccccccccccccc/", &[]);
        assert!(response_body(&nested_listing).contains("visible.txt"));

        let upload_only_page =
            http_request(ports[3], "GET", "/dddddddddddddddddddddddddddddddd/", &[]);
        assert!(upload_only_page.starts_with("HTTP/1.1 200 OK"));
        assert!(!response_body(&upload_only_page).contains("private.txt"));
        let upload_only_put = http_request(
            ports[3],
            "PUT",
            "/dddddddddddddddddddddddddddddddd/delivered.txt",
            b"delivered",
        );
        assert!(upload_only_put.starts_with("HTTP/1.1 201 Created"));
        assert_eq!(
            fs::read(d_receive.join("delivered.txt")).unwrap(),
            b"delivered"
        );

        assert_eq!(get_access_logs("share-a", 100).len(), 3);
        assert_eq!(get_access_logs("share-b", 100).len(), 2);
        assert_eq!(get_access_logs("share-c", 100).len(), 2);
        assert_eq!(get_access_logs("share-d", 100).len(), 2);
        stop_id("share-d");
        assert!(!is_running("share-d"));
        assert!(shares[..3].iter().all(|share| is_running(&share.id)));

        drop(running);
        delete_access_logs("share-a");
        delete_access_logs("share-b");
        delete_access_logs("share-c");
        delete_access_logs("share-d");
        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn login_mode_requires_credentials_and_protects_uploads_with_sessions() {
        let test_root = std::env::temp_dir().join(format!(
            "sideweave-file-share-auth-test-{}",
            uuid::Uuid::new_v4()
        ));
        let account_path = test_root.join("accounts.json");
        let share_directory = test_root.join("share");
        fs::create_dir_all(&share_directory).unwrap();
        fs::write(share_directory.join("welcome.txt"), "welcome").unwrap();
        TEST_ACCOUNT_STORE_PATH.set(account_path).unwrap();
        add_file_share_user("authuser", "StrongPassword123").unwrap();

        let reserved = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let mut share = test_share(
            "share-login",
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            port,
            &share_directory,
            &share_directory,
            true,
            true,
            false,
        );
        share.require_login = true;
        start(&mut share).unwrap();
        let running = RunningShares(vec![share.id.clone()]);

        let unauthenticated = http_request(port, "GET", "/", &[]);
        assert!(unauthenticated.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(response_body(&unauthenticated).contains("/__auth/login"));

        let bad_login = http_request_with_headers(
            port,
            "POST",
            "/__auth/login",
            b"username=authuser&password=WrongPassword123",
            &[
                ("Origin", "http://localhost"),
                ("Content-Type", "application/x-www-form-urlencoded"),
            ],
        );
        assert!(bad_login.starts_with("HTTP/1.1 401 Unauthorized"));

        let valid_login = http_request_with_headers(
            port,
            "POST",
            "/__auth/login",
            b"username=authuser&password=StrongPassword123",
            &[
                ("Origin", "http://localhost"),
                ("Content-Type", "application/x-www-form-urlencoded"),
            ],
        );
        assert!(valid_login.starts_with("HTTP/1.1 303 See Other"));
        let cookie_header = valid_login
            .lines()
            .find_map(|line| line.strip_prefix("Set-Cookie: "))
            .unwrap();
        assert!(cookie_header.contains("HttpOnly"));
        assert!(cookie_header.contains("SameSite=Strict"));
        assert!(cookie_header.contains("Max-Age=43200"));
        let cookie = cookie_header.split(';').next().unwrap().to_string();

        let authorized = http_request_with_headers(port, "GET", "/", &[], &[("Cookie", &cookie)]);
        assert!(authorized.starts_with("HTTP/1.1 200 OK"));
        assert!(response_body(&authorized).contains("welcome.txt"));
        let csrf_rejected = http_request_with_headers(
            port,
            "PUT",
            "/blocked.txt",
            b"blocked",
            &[("Cookie", &cookie), ("Origin", "http://attacker.test")],
        );
        assert!(csrf_rejected.starts_with("HTTP/1.1 403 Forbidden"));
        let authorized_upload = http_request_with_headers(
            port,
            "PUT",
            "/received.txt",
            b"received",
            &[("Cookie", &cookie), ("Origin", "http://localhost")],
        );
        assert!(authorized_upload.starts_with("HTTP/1.1 201 Created"));
        assert_eq!(
            fs::read(share_directory.join("received.txt")).unwrap(),
            b"received"
        );

        let logout = http_request_with_headers(
            port,
            "POST",
            "/__auth/logout",
            b"",
            &[("Cookie", &cookie), ("Origin", "http://localhost")],
        );
        assert!(logout.starts_with("HTTP/1.1 303 See Other"));
        let after_logout = http_request_with_headers(port, "GET", "/", &[], &[("Cookie", &cookie)]);
        assert!(after_logout.starts_with("HTTP/1.1 401 Unauthorized"));

        let second_login = http_request_with_headers(
            port,
            "POST",
            "/__auth/login",
            b"username=authuser&password=StrongPassword123",
            &[
                ("Origin", "http://localhost"),
                ("Content-Type", "application/x-www-form-urlencoded"),
            ],
        );
        let second_cookie = second_login
            .lines()
            .find_map(|line| line.strip_prefix("Set-Cookie: "))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let session_token =
            session_token_from_cookie(&second_cookie, &session_cookie_name(&share.id))
                .unwrap()
                .to_string();
        {
            let servers = server_state();
            let sessions = servers.get(&share.id).unwrap().sessions.clone();
            sessions
                .lock()
                .unwrap()
                .get_mut(&session_token)
                .unwrap()
                .expires = Instant::now() - Duration::from_secs(1);
        }
        let expired_session =
            http_request_with_headers(port, "GET", "/", &[], &[("Cookie", &second_cookie)]);
        assert!(expired_session.starts_with("HTTP/1.1 401 Unauthorized"));

        let third_login = http_request_with_headers(
            port,
            "POST",
            "/__auth/login",
            b"username=authuser&password=StrongPassword123",
            &[
                ("Origin", "http://localhost"),
                ("Content-Type", "application/x-www-form-urlencoded"),
            ],
        );
        let third_cookie = third_login
            .lines()
            .find_map(|line| line.strip_prefix("Set-Cookie: "))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        stop_id(&share.id);
        start(&mut share).unwrap();
        let after_restart =
            http_request_with_headers(port, "GET", "/", &[], &[("Cookie", &third_cookie)]);
        assert!(after_restart.starts_with("HTTP/1.1 401 Unauthorized"));

        drop(running);
        delete_access_logs(&share.id);
        fs::remove_dir_all(test_root).unwrap();
    }
}
