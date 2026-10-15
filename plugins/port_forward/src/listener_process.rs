use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct ListenerInfo {
    pub protocol: &'static str,
    pub address: String,
    pub port: u16,
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
    pub can_terminate: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StopListenerRequest {
    pub port: u16,
    pub pid: u32,
    pub path: String,
}

#[cfg(windows)]
mod windows {
    use super::{ListenerInfo, StopListenerRequest};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::io;
    use std::mem::size_of;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::path::Path;
    use std::ptr;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE, NO_ERROR, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
        MIB_UDP6ROW_OWNER_PID, MIB_UDPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
        UDP_TABLE_OWNER_PID,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
        WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE,
    };

    const MAX_TABLE_RETRIES: usize = 4;
    const PROCESS_IMAGE_CAPACITY: usize = 32_768;

    #[derive(Clone)]
    struct ProcessInfo {
        name: String,
        path: Option<String>,
        can_terminate: bool,
        blocked_reason: Option<String>,
    }

    struct ProcessHandle(HANDLE);

    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn win32_error(code: u32) -> String {
        io::Error::from_raw_os_error(code as i32).to_string()
    }

    fn read_table(mut query: impl FnMut(*mut c_void, *mut u32) -> u32) -> Result<Vec<u64>, String> {
        let mut byte_count = 0u32;
        let status = query(ptr::null_mut(), &mut byte_count);
        if status != ERROR_INSUFFICIENT_BUFFER && status != NO_ERROR {
            return Err(format!("读取本机端口表失败: {}", win32_error(status)));
        }
        if byte_count == 0 {
            return Ok(Vec::new());
        }

        for _ in 0..MAX_TABLE_RETRIES {
            let word_count = (byte_count as usize).div_ceil(size_of::<u64>());
            let mut buffer = vec![0u64; word_count];
            let mut actual_byte_count = byte_count;
            let status = query(buffer.as_mut_ptr().cast::<c_void>(), &mut actual_byte_count);
            if status == NO_ERROR {
                return Ok(buffer);
            }
            if status != ERROR_INSUFFICIENT_BUFFER {
                return Err(format!("读取本机端口表失败: {}", win32_error(status)));
            }
            byte_count = actual_byte_count.max(byte_count.saturating_mul(2));
        }

        Err("本机端口变化过快，请稍后重试".into())
    }

    fn process_path(handle: HANDLE) -> Result<String, String> {
        let mut buffer = vec![0u16; PROCESS_IMAGE_CAPACITY];
        let mut length = buffer.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) };
        if ok == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(String::from_utf16_lossy(&buffer[..length as usize]))
    }

    fn process_info(pid: u32) -> ProcessInfo {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        let path = if handle.is_null() {
            None
        } else {
            let handle = ProcessHandle(handle);
            process_path(handle.0).ok()
        };
        let name = path
            .as_deref()
            .and_then(|value| Path::new(value).file_name())
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("PID {pid}"));
        let blocked_reason = blocked_reason(pid, &name, path.is_some());
        ProcessInfo {
            name,
            path,
            can_terminate: blocked_reason.is_none(),
            blocked_reason,
        }
    }

    fn blocked_reason(pid: u32, name: &str, has_path: bool) -> Option<String> {
        if pid <= 4 {
            return Some("Windows 核心进程不能从此处结束".into());
        }
        if pid == unsafe { GetCurrentProcessId() } {
            return Some("Agile Tiles 自身进程不能从此处结束".into());
        }
        if !has_path {
            return Some("无法读取进程路径，不能安全确认结束对象".into());
        }

        let name = name.to_ascii_lowercase();
        let protected = [
            "system",
            "registry",
            "smss.exe",
            "csrss.exe",
            "wininit.exe",
            "winlogon.exe",
            "services.exe",
            "lsass.exe",
            "svchost.exe",
        ];
        if protected.contains(&name.as_str()) {
            return Some("Windows 系统或服务宿主进程不能从此处结束".into());
        }
        None
    }

    fn push_listener(
        result: &mut Vec<ListenerInfo>,
        cache: &mut HashMap<u32, ProcessInfo>,
        protocol: &'static str,
        address: String,
        port: u16,
        pid: u32,
        filter: Option<u16>,
    ) {
        if port == 0 || filter.is_some_and(|filter| filter != port) {
            return;
        }
        let info = cache.entry(pid).or_insert_with(|| process_info(pid));
        result.push(ListenerInfo {
            protocol,
            address,
            port,
            pid,
            name: info.name.clone(),
            path: info.path.clone(),
            can_terminate: info.can_terminate,
            blocked_reason: info.blocked_reason.clone(),
        });
    }

    fn port_from_network_order(value: u32) -> u16 {
        u16::from_be(value as u16)
    }

    fn table_count<Row>(buffer: &[u64]) -> Result<usize, String> {
        let byte_length = buffer.len().saturating_mul(size_of::<u64>());
        if byte_length < size_of::<u32>() {
            return Err("Windows 返回了无效的端口表".into());
        }
        let count = unsafe { ptr::read_unaligned(buffer.as_ptr().cast::<u32>()) } as usize;
        let max_count = (byte_length - size_of::<u32>()) / size_of::<Row>();
        if count > max_count {
            return Err("Windows 返回的端口表长度不完整".into());
        }
        Ok(count)
    }

    fn get_tcp_table(family: u32) -> Result<Vec<u64>, String> {
        read_table(|table, size| unsafe {
            GetExtendedTcpTable(table, size, 0, family, TCP_TABLE_OWNER_PID_LISTENER, 0)
        })
    }

    fn get_udp_table(family: u32) -> Result<Vec<u64>, String> {
        read_table(|table, size| unsafe {
            GetExtendedUdpTable(table, size, 0, family, UDP_TABLE_OWNER_PID, 0)
        })
    }

    fn append_tcp4(
        result: &mut Vec<ListenerInfo>,
        cache: &mut HashMap<u32, ProcessInfo>,
        filter: Option<u16>,
    ) -> Result<(), String> {
        let buffer = get_tcp_table(AF_INET as u32)?;
        let count = table_count::<MIB_TCPROW_OWNER_PID>(&buffer)?;
        let rows = unsafe {
            buffer
                .as_ptr()
                .cast::<u8>()
                .add(size_of::<u32>())
                .cast::<MIB_TCPROW_OWNER_PID>()
        };
        for index in 0..count {
            let row = unsafe { rows.add(index).read_unaligned() };
            push_listener(
                result,
                cache,
                "TCP",
                Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()).to_string(),
                port_from_network_order(row.dwLocalPort),
                row.dwOwningPid,
                filter,
            );
        }
        Ok(())
    }

    fn append_tcp6(
        result: &mut Vec<ListenerInfo>,
        cache: &mut HashMap<u32, ProcessInfo>,
        filter: Option<u16>,
    ) -> Result<(), String> {
        let buffer = get_tcp_table(AF_INET6 as u32)?;
        let count = table_count::<MIB_TCP6ROW_OWNER_PID>(&buffer)?;
        let rows = unsafe {
            buffer
                .as_ptr()
                .cast::<u8>()
                .add(size_of::<u32>())
                .cast::<MIB_TCP6ROW_OWNER_PID>()
        };
        for index in 0..count {
            let row = unsafe { rows.add(index).read_unaligned() };
            let address = Ipv6Addr::from(row.ucLocalAddr);
            let address = if row.dwLocalScopeId == 0 {
                address.to_string()
            } else {
                format!("{address}%{}", row.dwLocalScopeId)
            };
            push_listener(
                result,
                cache,
                "TCP",
                address,
                port_from_network_order(row.dwLocalPort),
                row.dwOwningPid,
                filter,
            );
        }
        Ok(())
    }

    fn append_udp4(
        result: &mut Vec<ListenerInfo>,
        cache: &mut HashMap<u32, ProcessInfo>,
        filter: Option<u16>,
    ) -> Result<(), String> {
        let buffer = get_udp_table(AF_INET as u32)?;
        let count = table_count::<MIB_UDPROW_OWNER_PID>(&buffer)?;
        let rows = unsafe {
            buffer
                .as_ptr()
                .cast::<u8>()
                .add(size_of::<u32>())
                .cast::<MIB_UDPROW_OWNER_PID>()
        };
        for index in 0..count {
            let row = unsafe { rows.add(index).read_unaligned() };
            push_listener(
                result,
                cache,
                "UDP",
                Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()).to_string(),
                port_from_network_order(row.dwLocalPort),
                row.dwOwningPid,
                filter,
            );
        }
        Ok(())
    }

    fn append_udp6(
        result: &mut Vec<ListenerInfo>,
        cache: &mut HashMap<u32, ProcessInfo>,
        filter: Option<u16>,
    ) -> Result<(), String> {
        let buffer = get_udp_table(AF_INET6 as u32)?;
        let count = table_count::<MIB_UDP6ROW_OWNER_PID>(&buffer)?;
        let rows = unsafe {
            buffer
                .as_ptr()
                .cast::<u8>()
                .add(size_of::<u32>())
                .cast::<MIB_UDP6ROW_OWNER_PID>()
        };
        for index in 0..count {
            let row = unsafe { rows.add(index).read_unaligned() };
            let address = Ipv6Addr::from(row.ucLocalAddr);
            let address = if row.dwLocalScopeId == 0 {
                address.to_string()
            } else {
                format!("{address}%{}", row.dwLocalScopeId)
            };
            push_listener(
                result,
                cache,
                "UDP",
                address,
                port_from_network_order(row.dwLocalPort),
                row.dwOwningPid,
                filter,
            );
        }
        Ok(())
    }

    fn port_owned_by_pid(port: u16, pid: u32) -> Result<bool, String> {
        let mut result = Vec::new();
        let mut cache = HashMap::new();
        append_tcp4(&mut result, &mut cache, Some(port))?;
        append_tcp6(&mut result, &mut cache, Some(port))?;
        append_udp4(&mut result, &mut cache, Some(port))?;
        append_udp6(&mut result, &mut cache, Some(port))?;
        Ok(result.iter().any(|item| item.pid == pid))
    }

    pub fn list_listeners(filter: Option<u16>) -> Result<Vec<ListenerInfo>, String> {
        if filter == Some(0) {
            return Err("端口号必须在 1-65535 之间".into());
        }
        let mut result = Vec::new();
        let mut cache = HashMap::new();
        append_tcp4(&mut result, &mut cache, filter)?;
        append_tcp6(&mut result, &mut cache, filter)?;
        append_udp4(&mut result, &mut cache, filter)?;
        append_udp6(&mut result, &mut cache, filter)?;
        result.sort_by(|left, right| {
            left.port
                .cmp(&right.port)
                .then_with(|| left.protocol.cmp(right.protocol))
                .then_with(|| left.address.cmp(&right.address))
                .then_with(|| left.pid.cmp(&right.pid))
        });
        Ok(result)
    }

    pub fn stop_listener(request: StopListenerRequest) -> Result<(), String> {
        if request.port == 0 || request.pid <= 4 {
            return Err("不能结束 Windows 核心进程或无效端口".into());
        }
        if request.path.trim().is_empty() {
            return Err("缺少进程路径，不能安全确认结束对象".into());
        }

        let snapshot = list_listeners(Some(request.port))?;
        let listed = snapshot
            .iter()
            .find(|item| item.pid == request.pid)
            .ok_or_else(|| "该进程已不再占用此端口，请刷新列表后重试".to_string())?;
        if !listed.can_terminate {
            return Err(listed
                .blocked_reason
                .clone()
                .unwrap_or_else(|| "该进程不能结束".into()));
        }

        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                0,
                request.pid,
            )
        };
        if handle.is_null() {
            return Err(format!("无法打开进程：{}", io::Error::last_os_error()));
        }
        let handle = ProcessHandle(handle);
        let current_path = process_path(handle.0)?;
        if !current_path.eq_ignore_ascii_case(&request.path) {
            return Err("进程身份已变化，请刷新列表后重新确认".into());
        }
        let name = Path::new(&current_path)
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(reason) = blocked_reason(request.pid, &name, true) {
            return Err(reason);
        }
        if !port_owned_by_pid(request.port, request.pid)? {
            return Err("该进程已不再占用此端口，请刷新列表后重试".into());
        }

        let ok = unsafe { TerminateProcess(handle.0, 1) };
        if ok == 0 {
            return Err(format!("结束进程失败：{}", io::Error::last_os_error()));
        }
        match unsafe { WaitForSingleObject(handle.0, 3_000) } {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err("已发出结束请求，但进程仍未退出".into()),
            _ => Err(format!("等待进程退出失败：{}", io::Error::last_os_error())),
        }
    }
}

#[cfg(windows)]
pub use windows::{list_listeners, stop_listener};

#[cfg(not(windows))]
pub fn list_listeners(_filter: Option<u16>) -> Result<Vec<ListenerInfo>, String> {
    Err("端口查看器目前仅支持 Windows".into())
}

#[cfg(not(windows))]
pub fn stop_listener(_request: StopListenerRequest) -> Result<(), String> {
    Err("结束占用端口的进程目前仅支持 Windows".into())
}
