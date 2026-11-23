use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use url::{Host, Url};

/// 只探测环回源站；远程源站返回 None，避免用本地端口误报状态。
pub fn local_origin_online(service: &str) -> Option<bool> {
    let (address, port) = super::config_manager::normalize_service_address(service).ok()?;
    let origin = Url::parse(&address).ok()?;
    if !super::config_manager::is_loopback_origin(&origin) {
        return None;
    }
    let ip = match origin.host()? {
        Host::Ipv4(ip) => ip.into(),
        Host::Ipv6(ip) => ip.into(),
        Host::Domain(_) => std::net::Ipv4Addr::LOCALHOST.into(),
    };
    Some(TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_millis(80)).is_ok())
}

/// 检测本地 127.0.0.1:port 是否有服务正在监听
pub fn is_local_port_listening(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(80)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn remote_origin_has_no_local_status() {
        assert_eq!(local_origin_online("https://example.com"), None);
        assert_eq!(local_origin_online("http://192.0.2.10:8080"), None);
    }

    #[test]
    fn checks_the_actual_loopback_address() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(
            local_origin_online(&format!("https://localhost:{port}")),
            Some(true)
        );
        // The same port on a different loopback IP must not report 127.0.0.1's listener.
        assert_eq!(
            local_origin_online(&format!("http://127.0.0.2:{port}")),
            Some(false)
        );
    }
}
