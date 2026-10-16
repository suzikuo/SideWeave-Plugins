use crate::models::FileShareConfig;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_ACTIVE_REQUESTS: usize = 32;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const CLIENT_READ_TIMEOUT: Duration = Duration::from_secs(5);
const UPLOAD_READ_TIMEOUT: Duration = Duration::from_secs(300);
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

struct RunningServer {
    lan_host: Option<String>,
    lan_enabled: bool,
    stop: Arc<AtomicBool>,
    clients: Arc<Mutex<Vec<(usize, TcpStream)>>>,
    thread: JoinHandle<()>,
}

struct UploadQuota {
    used_bytes: Mutex<u64>,
    maximum_bytes: u64,
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

static SERVER: OnceLock<Mutex<Option<RunningServer>>> = OnceLock::new();
static LIFECYCLE: OnceLock<Mutex<()>> = OnceLock::new();

fn server_state() -> MutexGuard<'static, Option<RunningServer>> {
    SERVER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn lifecycle_lock() -> MutexGuard<'static, ()> {
    LIFECYCLE
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn is_running() -> bool {
    server_state()
        .as_ref()
        .is_some_and(|server| !server.stop.load(Ordering::Relaxed))
}

pub fn lan_is_running() -> bool {
    server_state()
        .as_ref()
        .is_some_and(|server| server.lan_enabled && !server.stop.load(Ordering::Relaxed))
}

pub fn lan_host() -> Option<String> {
    server_state()
        .as_ref()
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
    stop_inner();
}

fn stop_inner() {
    let server = server_state().take();
    if let Some(server) = server {
        server.stop.store(true, Ordering::Relaxed);
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

pub fn start(config: &mut FileShareConfig) -> Result<(), String> {
    let _lifecycle = lifecycle_lock();
    if !config.enabled {
        stop_inner();
        return Ok(());
    }
    if config.port < 1024 {
        return Err("本机文件服务端口必须在 1024 到 65535 之间".into());
    }

    let root = fs::canonicalize(&config.directory)
        .map_err(|error| format!("共享目录无法访问：{error}"))?;
    if !root.is_dir() {
        return Err("共享路径不是一个目录".into());
    }
    config.directory = root.to_string_lossy().into_owned();
    if config.token.len() != 32 || !config.token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        config.token = uuid::Uuid::new_v4().simple().to_string();
    }
    validate_allowed_ips(&config.allowed_ips)?;

    let upload_directory = if config.upload_enabled {
        let configured = if config.upload_directory.trim().is_empty() {
            let name = root
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| "无法为上传目录生成默认名称，请填写目录路径".to_string())?;
            root.with_file_name(format!("{name}_uploads"))
        } else {
            PathBuf::from(config.upload_directory.trim())
        };
        fs::create_dir_all(&configured).map_err(|error| format!("创建上传目录失败：{error}"))?;
        let canonical =
            fs::canonicalize(&configured).map_err(|error| format!("上传目录无法访问：{error}"))?;
        if !canonical.is_dir() {
            return Err("上传路径不是一个目录".into());
        }
        if canonical.starts_with(&root) || root.starts_with(&canonical) {
            return Err("上传目录必须与只读共享目录分开，避免上传内容被公开或覆盖共享目录".into());
        }
        config.upload_directory = canonical.to_string_lossy().into_owned();
        Some(canonical)
    } else {
        None
    };
    let upload_quota = if let Some(directory) = upload_directory.as_deref() {
        let used_bytes = directory_upload_size(directory)
            .map_err(|error| format!("读取上传目录用量失败：{error}"))?;
        let maximum_bytes = u64::from(config.max_total_upload_mb.clamp(1, 8192)) * 1024 * 1024;
        Some(Arc::new(UploadQuota {
            used_bytes: Mutex::new(used_bytes),
            maximum_bytes,
        }))
    } else {
        None
    };

    let lan_host = if config.lan_enabled {
        detect_lan_ipv4()
    } else {
        None
    };
    let lan_networks = if config.lan_enabled {
        detect_lan_networks()
    } else {
        Vec::new()
    };

    stop_inner();

    let bind_address = if config.lan_enabled {
        Ipv4Addr::UNSPECIFIED
    } else {
        Ipv4Addr::LOCALHOST
    };
    let listener =
        TcpListener::bind(SocketAddr::from((bind_address, config.port))).map_err(|error| {
            format!(
                "无法监听文件服务端口 {}:{}：{error}",
                bind_address, config.port
            )
        })?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("无法配置本机文件服务：{error}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread_root = root.clone();
    let thread_token = config.token.clone();
    let thread_upload_directory = upload_directory.clone();
    let thread_upload_enabled = config.upload_enabled;
    let thread_max_upload_bytes = u64::from(config.max_upload_mb.clamp(1, 1024)) * 1024 * 1024;
    let thread_upload_quota = upload_quota.clone();
    let thread_allowed_ips = parse_allowed_ips(&config.allowed_ips)?;
    let thread_lan_networks = lan_networks.clone();
    let clients = Arc::new(Mutex::new(Vec::new()));
    let thread_clients = clients.clone();
    let client_ids = Arc::new(AtomicUsize::new(1));
    let thread_client_ids = client_ids.clone();
    let handle = thread::Builder::new()
        .name("cloudflare-file-share".into())
        .spawn(move || {
            serve_loop(
                listener,
                thread_root,
                thread_token,
                thread_upload_directory,
                thread_upload_enabled,
                thread_max_upload_bytes,
                thread_upload_quota,
                thread_allowed_ips,
                thread_lan_networks,
                thread_stop,
                thread_clients,
                thread_client_ids,
            )
        })
        .map_err(|error| format!("无法启动本机文件服务线程：{error}"))?;

    *server_state() = Some(RunningServer {
        lan_host,
        lan_enabled: config.lan_enabled,
        stop,
        clients,
        thread: handle,
    });
    Ok(())
}

fn serve_loop(
    listener: TcpListener,
    root: PathBuf,
    token: String,
    upload_directory: Option<PathBuf>,
    upload_enabled: bool,
    max_upload_bytes: u64,
    upload_quota: Option<Arc<UploadQuota>>,
    allowed_ips: Vec<IpNetwork>,
    lan_networks: Vec<IpNetwork>,
    stop: Arc<AtomicBool>,
    clients: Arc<Mutex<Vec<(usize, TcpStream)>>>,
    client_ids: Arc<AtomicUsize>,
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
            if active.fetch_add(1, Ordering::Relaxed) >= MAX_ACTIVE_REQUESTS {
                active.fetch_sub(1, Ordering::Relaxed);
                let _ = stream.set_write_timeout(Some(CLIENT_READ_TIMEOUT));
                let _ = send_response(
                    stream,
                    "503 Service Unavailable",
                    "text/plain; charset=utf-8",
                    b"Too many requests",
                    false,
                    &stop,
                );
                continue;
            }

            let client_id = client_ids.fetch_add(1, Ordering::Relaxed);
            let Ok(shutdown_stream) = stream.try_clone() else {
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
            let request_token = token.clone();
            let request_upload_directory = upload_directory.clone();
            let request_allowed_ips = allowed_ips.clone();
            let request_lan_networks = lan_networks.clone();
            let request_stop = stop.clone();
            let request_active = active.clone();
            let request_clients = clients.clone();
            let request_max_upload_bytes = max_upload_bytes;
            let request_upload_quota = upload_quota.clone();
            match thread::Builder::new()
                .name("cloudflare-file-request".into())
                .spawn(move || {
                    let _active = ActiveRequest {
                        active: request_active,
                        clients: request_clients,
                        client_id,
                    };
                    let _ = serve_request(
                        stream,
                        peer_addr,
                        &request_root,
                        &request_token,
                        request_upload_directory.as_deref(),
                        upload_enabled,
                        request_max_upload_bytes,
                        request_upload_quota.as_deref(),
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

fn serve_request(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    root: &Path,
    token: &str,
    upload_directory: Option<&Path>,
    upload_enabled: bool,
    max_upload_bytes: u64,
    upload_quota: Option<&UploadQuota>,
    allowed_ips: &[IpNetwork],
    lan_networks: &[IpNetwork],
    stop: &AtomicBool,
) -> io::Result<()> {
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
            );
        }
        Err(RequestReadError::Io(error)) => return Err(error),
        Err(RequestReadError::Malformed) => {
            return send_response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                b"Bad request",
                false,
                stop,
            );
        }
    };

    let is_loopback = peer_addr.ip().is_loopback();
    let client_ip = if is_loopback {
        request.cf_connecting_ip.unwrap_or(peer_addr.ip())
    } else {
        peer_addr.ip()
    };
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

    let Some(relative) = decode_share_path(&request.target, token) else {
        return send_response(
            stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"Not found",
            request.method == "HEAD",
            stop,
        );
    };
    let method = request.method.as_str();
    if upload_enabled && method == "PUT" {
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
            );
        }
    };

    if metadata.is_dir() {
        let body = match directory_listing(
            root,
            &canonical_target,
            &relative,
            token,
            upload_enabled.then_some((
                max_upload_bytes,
                upload_quota.map(|quota| quota.maximum_bytes).unwrap_or(0),
            )),
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
                );
            }
        };
        return send_response(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            body.as_bytes(),
            method == "HEAD",
            stop,
        );
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
            );
        }
    };
    let content_type = content_type(&canonical_target);
    let filename = canonical_target
        .file_name()
        .map(|value| safe_filename(&value.to_string_lossy()))
        .unwrap_or_else(|| "download".into());
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nContent-Disposition: attachment; filename=\"{}\"\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\n\r\n",
        metadata.len(),
        content_type,
        filename
    )?;

    if method == "HEAD" {
        return Ok(());
    }
    let mut buffer = [0u8; 32 * 1024];
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        stream.write_all(&buffer[..read])?;
    }
    Ok(())
}

fn serve_upload_request(
    stream: TcpStream,
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

fn save_upload(
    mut stream: TcpStream,
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
        let (status, message) = match error {
            UploadBodyError::Request(error) if error.kind() == io::ErrorKind::TimedOut => {
                ("408 Request Timeout", "Upload timed out")
            }
            UploadBodyError::Request(_) | UploadBodyError::InvalidFraming => {
                ("400 Bad Request", "Invalid or incomplete upload body")
            }
            UploadBodyError::TooLarge => (
                "413 Content Too Large",
                "File exceeds the configured upload limit",
            ),
            UploadBodyError::QuotaExceeded => (
                "507 Insufficient Storage",
                "The upload directory has reached its configured size limit",
            ),
            UploadBodyError::Storage => (
                "507 Insufficient Storage",
                "Could not write the uploaded file",
            ),
            UploadBodyError::Stopped => return Ok(()),
        };
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

fn copy_fixed_upload(
    stream: &mut TcpStream,
    file: &mut File,
    content_length: u64,
    stop: &AtomicBool,
) -> Result<(), UploadBodyError> {
    let mut remaining = content_length;
    let mut buffer = [0u8; 32 * 1024];
    while remaining > 0 && !stop.load(Ordering::Relaxed) {
        let read_limit = buffer.len().min(remaining as usize);
        let count = stream
            .read(&mut buffer[..read_limit])
            .map_err(UploadBodyError::Request)?;
        if count == 0 {
            return Err(UploadBodyError::InvalidFraming);
        }
        file.write_all(&buffer[..count])
            .map_err(|_| UploadBodyError::Storage)?;
        remaining -= count as u64;
    }
    if stop.load(Ordering::Relaxed) {
        Err(UploadBodyError::Stopped)
    } else {
        Ok(())
    }
}

fn copy_chunked_upload(
    stream: &mut TcpStream,
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
            let count = stream
                .read(&mut buffer[..read_limit])
                .map_err(UploadBodyError::Request)?;
            if count == 0 {
                return Err(UploadBodyError::InvalidFraming);
            }
            file.write_all(&buffer[..count])
                .map_err(|_| UploadBodyError::Storage)?;
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

fn read_http_line(stream: &mut TcpStream, max_bytes: usize) -> io::Result<Vec<u8>> {
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
.brand { display: inline-flex; align-items: center; gap: 10px; color: var(--text); font-size: 14px; font-weight: 750; letter-spacing: -.02em; }
.brand-mark { display: grid; width: 34px; height: 34px; place-items: center; border-radius: 11px; background: linear-gradient(145deg, #4778ff, #284bc5); box-shadow: 0 5px 14px rgba(49, 94, 232, .24); color: white; font-size: 11px; letter-spacing: .04em; }
.share-badge { display: inline-flex; align-items: center; gap: 7px; padding: 7px 11px; border: 1px solid var(--line); border-radius: 999px; background: rgba(255,255,255,.72); color: var(--muted); font-size: 12px; }
.share-badge::before { width: 7px; height: 7px; border-radius: 50%; background: var(--success); content: ''; }
.page-main { margin-top: 34px; }
.page-intro { padding: 0 2px 22px; }
.eyebrow { margin: 0 0 8px; color: var(--accent); font-size: 10px; font-weight: 800; letter-spacing: .16em; text-transform: uppercase; }
h1 { margin: 0; font-size: clamp(26px, 4vw, 36px); line-height: 1.2; letter-spacing: -.045em; overflow-wrap: anywhere; }
.lede { margin: 10px 0 0; color: var(--muted); font-size: 14px; }
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
.file-picker { position: relative; display: flex; min-width: 0; min-height: 46px; align-items: center; gap: 10px; padding: 0 13px; overflow: hidden; border: 1px dashed #c5cfdf; border-radius: 10px; background: var(--surface-soft); color: var(--muted); cursor: pointer; }
.file-picker:hover { border-color: var(--accent); background: var(--accent-soft); }
.file-picker-icon { display: grid; width: 25px; height: 25px; flex: none; place-items: center; border-radius: 7px; background: var(--accent-soft); color: var(--accent); font-size: 18px; line-height: 1; }
.file-picker-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.file-picker input { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; cursor: pointer; }
.primary-button { min-height: 46px; padding: 0 18px; border: 1px solid var(--accent); border-radius: 10px; background: var(--accent); color: #fff; font-size: 13px; font-weight: 700; cursor: pointer; transition: background .15s, transform .15s; }
.primary-button:hover:not(:disabled) { background: #244dcc; transform: translateY(-1px); }
.primary-button:disabled { border-color: #bbc6d7; background: #bbc6d7; cursor: not-allowed; }
.upload-status { display: grid; max-height: 180px; gap: 6px; margin-top: 12px; overflow: auto; }
.upload-status:empty { display: none; }
.upload-result { margin: 0; padding: 7px 10px; border-radius: 8px; background: var(--surface-soft); color: var(--muted); font-size: 12px; overflow-wrap: anywhere; }
.upload-result.success { background: #eaf8f1; color: #146b49; }
.upload-result.error { background: #fff0f0; color: #a23131; }
.upload-result.progress { background: var(--accent-soft); color: #274cc0; }
.list-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 17px 20px; border-bottom: 1px solid var(--line); }
.item-count { color: var(--muted); font-size: 11px; }
.file-list { padding: 7px; }
.file-row { display: grid; grid-template-columns: 42px minmax(0,1fr) minmax(72px,auto) 24px; align-items: center; gap: 12px; min-height: 64px; padding: 9px 12px; border-radius: 11px; transition: background .14s; }
.file-row + .file-row { margin-top: 2px; }
.file-row:hover { background: var(--surface-soft); }
.file-row:focus-visible, .breadcrumbs a:focus-visible, .brand:focus-visible, .primary-button:focus-visible { outline: 3px solid rgba(49,94,232,.3); outline-offset: 2px; }
.file-icon { display: grid; width: 38px; height: 38px; place-items: center; border-radius: 11px; background: var(--accent-soft); color: var(--accent); }
.file-icon.folder { background: #fff6df; color: #b77b0b; }
.file-icon svg { width: 20px; height: 20px; }
.file-main { display: flex; min-width: 0; flex-direction: column; gap: 2px; }
.file-name { overflow-wrap: anywhere; color: var(--text); font-size: 13px; font-weight: 650; }
.file-kind { color: var(--muted); font-size: 10px; }
.file-meta { color: var(--muted); font-size: 11px; text-align: right; white-space: nowrap; }
.file-arrow { color: #a4afbf; font-size: 18px; text-align: center; }
.empty-state { display: grid; min-height: 168px; place-content: center; justify-items: center; gap: 7px; padding: 24px; color: var(--muted); text-align: center; }
.empty-icon { display: grid; width: 42px; height: 42px; place-items: center; margin-bottom: 2px; border-radius: 14px; background: var(--surface-soft); color: #8794a8; font-size: 20px; }
.empty-state strong { color: var(--text); font-size: 13px; }
.empty-state span { font-size: 11px; }
.page-footer { padding: 19px 4px 0; color: #8a96a8; font-size: 10px; text-align: center; }
@media (prefers-color-scheme: dark) {
  :root { color-scheme: dark; --page: #0e1420; --surface: #171f2d; --surface-soft: #1c2737; --text: #e8edf6; --muted: #9ba9bc; --line: #2a3749; --accent: #86a5ff; --accent-soft: #222f4b; --success: #56d39d; --shadow: 0 18px 48px rgba(0,0,0,.18); }
  body { background: radial-gradient(ellipse at 50% -18%, #1b2b4a 0, transparent 46%), var(--page); }
  .share-badge { background: rgba(23,31,45,.8); }
  .file-icon.folder { background: #382f1d; color: #edc16b; }
  .upload-result.success { background: #18372c; color: #8be0b7; }
  .upload-result.error { background: #3c2529; color: #ffaaaa; }
  .upload-result.progress { background: #222f4b; color: #b7c8ff; }
  .primary-button:disabled { border-color: #445168; background: #445168; color: #bbc5d4; }
}
@media (max-width: 620px) {
  .page-shell { width: min(100% - 24px, 920px); padding: 17px 0 34px; }
  .page-main { margin-top: 26px; }
  .page-intro { padding-bottom: 17px; }
  .lede { font-size: 13px; }
  .upload-panel { padding: 16px; }
  .upload-heading { flex-direction: column; gap: 9px; }
  .upload-limit { align-self: flex-start; }
  .upload-controls { grid-template-columns: 1fr; }
  .primary-button { width: 100%; }
  .list-heading { padding: 15px 16px; }
  .file-list { padding: 6px; }
  .file-row { grid-template-columns: 38px minmax(0,1fr) auto; gap: 10px; min-height: 62px; padding: 9px 8px; }
  .file-icon { width: 34px; height: 34px; border-radius: 10px; }
  .file-icon svg { width: 18px; height: 18px; }
  .file-meta { grid-column: 2; margin-top: -6px; text-align: left; }
  .file-arrow { grid-column: 3; grid-row: 1 / span 2; }
}
@media (max-width: 380px) {
  .page-shell { width: min(100% - 20px, 920px); }
  .share-badge { padding: 6px 9px; font-size: 11px; }
  .file-row { grid-template-columns: 32px minmax(0,1fr) auto; gap: 8px; }
  .file-icon { width: 30px; height: 30px; }
}
"#;

const FOLDER_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h6.9a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5.5a2 2 0 0 1-2-2z"/><path d="M3.8 10h16.4"/></svg>"#;
const FILE_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M7 3.8h6.5l4.7 4.7v11.7H7a2 2 0 0 1-2-2v-12a2 2 0 0 1 2-2.4z"/><path d="M13 4v5h5M8.5 13h7M8.5 16.5h7"/></svg>"#;

const UPLOAD_PANEL_TEMPLATE: &str = r#"<section class="upload-panel" aria-labelledby="upload-title"><div class="upload-heading"><div><h2 id="upload-title">上传到收件目录</h2><p>上传文件保存在独立收件目录，同名文件不会覆盖。</p></div><span class="upload-limit">单文件最多 MAX_UPLOAD_MB MB · 收件目录最多 MAX_TOTAL_MB MB</span></div><div class="upload-controls"><label class="file-picker"><span class="file-picker-icon" aria-hidden="true">＋</span><span id="file-label" class="file-picker-label">点击选择文件或从手机中选取</span><input id="files" type="file" multiple aria-label="选择要上传的文件"></label><button id="upload" class="primary-button" type="button" disabled>开始上传</button></div><div id="status" class="upload-status" role="status" aria-live="polite"></div><script>
const input=document.getElementById('files');
const fileLabel=document.getElementById('file-label');
const button=document.getElementById('upload');
const status=document.getElementById('status');
const maxUploadBytes=MAX_UPLOAD_BYTES;
function report(message,state=''){const row=document.createElement('p');row.className='upload-result '+state;row.textContent=message;status.appendChild(row)}
input.addEventListener('change',()=>{const files=Array.from(input.files||[]);fileLabel.textContent=files.length?files.map(file=>file.name).join('、'):'点击选择文件或从手机中选取';button.disabled=files.length===0;status.replaceChildren()});
button.addEventListener('click',async()=>{const files=Array.from(input.files||[]);if(!files.length){report('请先选择文件。','error');return}status.replaceChildren();button.disabled=true;button.textContent='正在上传…';const token=location.pathname.split('/').filter(Boolean)[0];const base='/'+token+'/';try{for(let index=0;index<files.length;index++){const file=files[index];if(file.size>maxUploadBytes){report('已跳过 '+file.name+'：超过单文件上限。','error');continue}report('上传中（'+(index+1)+'/'+files.length+'）：'+file.name,'progress');try{const response=await fetch(new URL(base+encodeURIComponent(file.name),location.origin),{method:'PUT',body:file});const message=await response.text();let detail=message.trim();if(!response.ok&&/<!doctype html/i.test(message)){const page=new DOMParser().parseFromString(message,'text/html');const code=page.querySelector('.code-label')?.textContent?.trim()||('HTTP '+response.status);const ray=message.match(/Cloudflare Ray ID:\s*(?:<strong[^>]*>)?([^<\s]+)/i)?.[1];detail=[code,page.title?.trim(),ray?'Ray ID '+ray:''].filter(Boolean).join(' · ')}report((response.ok?'上传完成：':'上传失败：')+file.name+(detail?' · '+detail:''),response.ok?'success':'error')}catch(error){report('上传失败：'+file.name+' · '+String(error),'error')}}}finally{button.disabled=false;button.textContent='重新上传'}});
</script></section>"#;

fn upload_panel(max_upload_bytes: u64, max_total_bytes: u64) -> String {
    UPLOAD_PANEL_TEMPLATE
        .replace("MAX_UPLOAD_BYTES", &max_upload_bytes.to_string())
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
    content_length: Option<usize>,
    transfer_encoding: bool,
    cf_connecting_ip: Option<IpAddr>,
}

enum RequestReadError {
    TooLarge,
    Malformed,
    Io(io::Error),
}

fn read_request_head(stream: &mut TcpStream) -> Result<RequestHead, RequestReadError> {
    let mut bytes = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        let count = stream.read(&mut byte).map_err(RequestReadError::Io)?;
        if count == 0 {
            return Err(RequestReadError::Malformed);
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
    if method.is_empty()
        || target.is_empty()
        || !(version == "HTTP/1.0" || version == "HTTP/1.1")
        || method.bytes().any(|byte| !byte.is_ascii_uppercase())
    {
        return Err(RequestReadError::Malformed);
    }
    let mut content_length = None;
    let mut transfer_encoding = false;
    let mut cf_connecting_ip = None;
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
        }
    }
    if transfer_encoding && content_length.is_some() {
        return Err(RequestReadError::Malformed);
    }
    Ok(RequestHead {
        method: method.to_string(),
        target: target.to_string(),
        content_length,
        transfer_encoding,
        cf_connecting_ip,
    })
}

fn decode_share_path(target: &str, token: &str) -> Option<PathBuf> {
    let raw_path = target.split('?').next()?;
    let raw_path = raw_path.strip_prefix('/')?;
    let mut segments = raw_path.split('/');
    if decode_segment(segments.next()?)? != token {
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
    filename
}

fn directory_listing(
    root: &Path,
    directory: &Path,
    relative: &Path,
    token: &str,
    upload_limits: Option<(u64, u64)>,
) -> io::Result<String> {
    let mut entries = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let canonical = fs::canonicalize(&path).ok()?;
            if !canonical.starts_with(root) {
                return None;
            }
            let metadata = fs::metadata(&canonical).ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            Some((name, metadata.is_dir(), metadata.len()))
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
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\"><meta name=\"referrer\" content=\"no-referrer\"><meta name=\"theme-color\" content=\"#f4f7fb\"><title>",
    );
    html.push_str(&escaped_title);
    html.push_str(" · 文件共享</title><style>");
    html.push_str(SHARE_PAGE_STYLE);
    html.push_str("</style></head><body><div class=\"page-shell\"><header class=\"sitebar\"><a class=\"brand\" href=\"");
    html.push_str(&root_href);
    html.push_str("\"><span class=\"brand-mark\" aria-hidden=\"true\">SW</span><span>SideWeave</span></a><span class=\"share-badge\">文件共享</span></header><main class=\"page-main\"><section class=\"page-intro\"><p class=\"eyebrow\">FILE SHARING</p><h1>");
    html.push_str(&escaped_title);
    html.push_str("</h1><p class=\"lede\">浏览文件夹，点击文件即可下载。</p></section>");
    append_breadcrumbs(&mut html, relative, token);
    if let Some((max_upload_bytes, max_total_bytes)) = upload_limits {
        html.push_str(&upload_panel(max_upload_bytes, max_total_bytes));
    }
    html.push_str("<section class=\"file-browser\" aria-labelledby=\"file-list-title\"><div class=\"list-heading\"><h2 id=\"file-list-title\">文件和文件夹</h2><span class=\"item-count\">");
    html.push_str(&format!("{} 项", entries.len()));
    html.push_str("</span></div><div class=\"file-list\">");
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
    for (name, is_dir, size) in entries {
        let encoded_relative = relative.join(&name);
        let href = listing_href(token, &encoded_relative, is_dir);
        let label = html_escape(&name);
        let meta = if is_dir {
            "文件夹".to_string()
        } else {
            format_size(size)
        };
        html.push_str("<a class=\"file-row\" href=\"");
        html.push_str(&href);
        html.push_str("\" aria-label=\"");
        html.push_str(if is_dir {
            "打开文件夹："
        } else {
            "下载文件："
        });
        html.push_str(&label);
        html.push_str("\"><span class=\"file-icon");
        if is_dir {
            html.push_str(" folder");
        }
        html.push_str("\">");
        html.push_str(if is_dir { FOLDER_ICON } else { FILE_ICON });
        html.push_str("</span><span class=\"file-main\"><span class=\"file-name\">");
        html.push_str(&label);
        html.push_str("</span><span class=\"file-kind\">");
        html.push_str(if is_dir {
            "打开文件夹"
        } else {
            "点击下载"
        });
        html.push_str("</span></span><span class=\"file-meta\">");
        html.push_str(&meta);
        html.push_str("</span><span class=\"file-arrow\" aria-hidden=\"true\">");
        html.push_str(if is_dir { "›" } else { "↓" });
        html.push_str("</span></a>");
    }
    html.push_str("</div></section></main><footer class=\"page-footer\">通过专属链接安全访问 · SideWeave</footer></div></body></html>");
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
    let mut href = format!("/{token}/");
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
        "txt" | "log" | "md" | "csv" => "text/plain; charset=utf-8",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "html" | "htm" => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
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

fn send_response(
    mut stream: TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    stop: &AtomicBool,
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store, no-transform\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'\r\n\r\n",
        body.len()
    )?;
    if !head_only && !stop.load(Ordering::Relaxed) {
        stream.write_all(body)?;
    }
    Ok(())
}
