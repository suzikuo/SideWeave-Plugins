use super::*;
use sideweave_plugin_api::{
    PluginBuffer, PluginDylibApi, PluginHostApi, DYNAMIC_PLUGIN_ABI_VERSION,
};
use std::collections::HashMap;
use std::ffi::c_void;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::{Duration, Instant};

// Exercise the same ABI, host storage scope, and ports.* routing as an installed plugin.
struct NativePlugin {
    api: Box<PluginDylibApi>,
    state: *mut c_void,
    storage: HashMap<String, String>,
}

unsafe extern "C" fn host_call(
    user_data: *mut c_void,
    method_ptr: *const u8,
    method_len: usize,
    payload_ptr: *const u8,
    payload_len: usize,
) -> PluginBuffer {
    let storage = &mut *(user_data as *mut HashMap<String, String>);
    let method = std::str::from_utf8(std::slice::from_raw_parts(method_ptr, method_len)).unwrap();
    let payload: Value =
        serde_json::from_slice(std::slice::from_raw_parts(payload_ptr, payload_len)).unwrap();
    let key = payload["key"].as_str().unwrap_or_default();
    let result = match method {
        // Like the host, ignore the caller-supplied plugin_id and use this plugin's store.
        "host.storage.get" => json!(storage.get(key)),
        "host.storage.set" => {
            storage.insert(key.into(), payload["value"].as_str().unwrap().into());
            Value::Null
        }
        _ => return PluginBuffer::empty(),
    };
    let mut bytes = serde_json::to_vec(&json!({ "ok": true, "result": result })).unwrap();
    let buffer = PluginBuffer {
        ptr: bytes.as_mut_ptr(),
        len: bytes.len(),
        capacity: bytes.capacity(),
    };
    std::mem::forget(bytes);
    buffer
}

unsafe extern "C" fn free_host_buffer(_: *mut c_void, buffer: PluginBuffer) {
    drop(Vec::from_raw_parts(buffer.ptr, buffer.len, buffer.capacity));
}

impl NativePlugin {
    fn new(rule: Value) -> Self {
        let api = unsafe {
            Box::from_raw(
                sideweave_plugin_sdk::dylib::entry::<DevConnectionPlugin>() as *mut PluginDylibApi
            )
        };
        let state = unsafe { api.create.unwrap()() };
        Self {
            api,
            state,
            storage: HashMap::from([(
                "dev_connection_port_forward".into(),
                json!({ "rules": [rule] }).to_string(),
            )]),
        }
    }

    fn request(&mut self, kind: &str, method: Option<&str>, payload: Value) -> Value {
        let request = serde_json::to_vec(&json!({
            "request_id": "test", "plugin_id": "dev_connection", "kind": kind,
            "method": method, "payload": payload,
        }))
        .unwrap();
        let host = PluginHostApi {
            abi_version: DYNAMIC_PLUGIN_ABI_VERSION,
            user_data: &mut self.storage as *mut _ as *mut c_void,
            call: Some(host_call),
            free_buffer: Some(free_host_buffer),
        };
        let buffer =
            unsafe { self.api.handle.unwrap()(self.state, request.as_ptr(), request.len(), host) };
        let response: Value = unsafe {
            serde_json::from_slice(std::slice::from_raw_parts(buffer.ptr, buffer.len)).unwrap()
        };
        unsafe { self.api.free_buffer.unwrap()(buffer) };
        assert_eq!(response["plugin_id"], "dev_connection");
        response
    }
}

impl Drop for NativePlugin {
    fn drop(&mut self) {
        self.request("shutdown", None, Value::Null);
        unsafe { self.api.destroy.unwrap()(self.state) };
    }
}

#[test]
fn native_port_forward_reports_conflicts_and_relays_after_retry() {
    let target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    target.set_nonblocking(true).unwrap();
    let target_port = target.local_addr().unwrap().port();
    let occupied = TcpListener::bind(("0.0.0.0", 0)).unwrap();
    let listen_port = occupied.local_addr().unwrap().port();
    let mut plugin = NativePlugin::new(json!({
        "id": "conflict-and-relay", "name": "test", "listen_port": listen_port,
        "target_host": "127.0.0.1", "target_port": target_port, "enabled": true,
    }));
    assert_eq!(plugin.request("activate", None, Value::Null)["ok"], true);
    let listed = plugin.request("call", Some("ports.list"), json!({}));
    assert_eq!(listed["result"][0]["running"], false);
    assert!(listed["result"][0]["last_error"]
        .as_str()
        .unwrap()
        .contains(&listen_port.to_string()));
    assert!(listed["result"][0]["last_error"]
        .as_str()
        .unwrap()
        .contains("独立端口工具"));

    let failed = plugin.request(
        "call",
        Some("ports.set_running"),
        json!({ "id": "conflict-and-relay", "running": true }),
    );
    assert_eq!(failed["ok"], false);
    drop(occupied);
    let started = plugin.request(
        "call",
        Some("ports.set_running"),
        json!({ "id": "conflict-and-relay", "running": true }),
    );
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["result"]["running"], true);
    assert!(started["result"]["last_error"].is_null());

    // Set this to a local LAN address to verify the path that WSL's loopback
    // listener does not cover; the default stays portable for automated runs.
    let client_host =
        std::env::var("SIDEWEAVE_PORT_TEST_CLIENT_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let mut client = TcpStream::connect((client_host.as_str(), listen_port)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let request = b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n";
    client.write_all(request).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut upstream = loop {
        match target.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "forwarder did not connect to target"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept target connection: {error}"),
        }
    };
    upstream.set_nonblocking(false).unwrap();
    upstream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut received = vec![0; request.len()];
    upstream.read_exact(&mut received).unwrap();
    assert_eq!(received, request);
    let reply = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
    upstream.write_all(reply).unwrap();
    let mut response = vec![0; reply.len()];
    client.read_exact(&mut response).unwrap();
    assert_eq!(response, reply);
    // A browser keeps its write side open while waiting for a reply, then may send
    // another request over the same connection. Neither idle period is an error.
    client.write_all(request).unwrap();
    upstream.read_exact(&mut received).unwrap();
    assert_eq!(received, request);
    upstream.write_all(reply).unwrap();
    client.read_exact(&mut response).unwrap();
    assert_eq!(response, reply);
    client.shutdown(Shutdown::Write).unwrap();
    assert_eq!(upstream.read(&mut [0]).unwrap(), 0);
    upstream.shutdown(Shutdown::Write).unwrap();
    assert_eq!(client.read(&mut [0]).unwrap(), 0);

    let stopped = plugin.request(
        "call",
        Some("ports.set_running"),
        json!({ "id": "conflict-and-relay", "running": false }),
    );
    assert_eq!(stopped["result"]["running"], false);
    assert!(stopped["result"]["last_error"].is_null());
    assert!(TcpStream::connect(("127.0.0.1", listen_port)).is_err());
}
