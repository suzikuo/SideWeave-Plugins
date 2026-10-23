use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// 检测本地 127.0.0.1:port 是否有服务正在监听
pub fn is_local_port_listening(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(80)).is_ok()
}
