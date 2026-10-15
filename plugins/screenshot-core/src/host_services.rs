use agile_tiles_plugin_sdk::{
    PluginId, PluginRequest, PluginRequestKind, PluginResponse, PluginWireMessage,
    DEFAULT_REQUEST_TIMEOUT_MS,
};
use agile_tiles_plugin_ipc::{FrameReader, FrameWriter};
use serde_json::{json, Value};
use std::fs::File;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_HOST_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Calls host services over the currently active external-plugin request.
/// The host already supports these nested requests and encrypts secrets with
/// the current Windows user's DPAPI key before writing them to disk.
pub struct HostServicesClient<'a> {
    reader: &'a mut FrameReader<File>,
    writer: &'a mut FrameWriter<File>,
    plugin_id: PluginId,
}

impl<'a> HostServicesClient<'a> {
    pub fn new(
        reader: &'a mut FrameReader<File>,
        writer: &'a mut FrameWriter<File>,
        plugin_id: PluginId,
    ) -> Self {
        Self {
            reader,
            writer,
            plugin_id,
        }
    }

    pub fn get_secret(&mut self, key: &str) -> Result<Option<String>, String> {
        let value = self.call("host.secrets.get", json!({ "key": key }))?;
        match value {
            Value::Null => Ok(None),
            Value::String(value) => Ok(Some(value)),
            _ => Err("宿主返回的密钥格式无效".into()),
        }
    }

    pub fn set_secret(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.call("host.secrets.set", json!({ "key": key, "value": value }))?;
        Ok(())
    }

    pub fn remove_secret(&mut self, key: &str) -> Result<(), String> {
        self.call("host.secrets.remove", json!({ "key": key }))?;
        Ok(())
    }

    fn call(&mut self, method: &str, payload: Value) -> Result<Value, String> {
        let request_id = format!(
            "screenshot-host-{}",
            NEXT_HOST_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        );
        let request = PluginRequest {
            request_id: request_id.clone(),
            plugin_id: self.plugin_id.clone(),
            kind: PluginRequestKind::Call,
            method: Some(method.into()),
            payload,
            timeout_ms: Some(DEFAULT_REQUEST_TIMEOUT_MS),
        };
        self.writer
            .write_message(&PluginWireMessage::Request(request))
            .map_err(|error| format!("请求宿主安全存储失败：{error:?}"))?;

        loop {
            match self
                .reader
                .read_message()
                .map_err(|error| format!("读取宿主安全存储响应失败：{error:?}"))?
            {
                PluginWireMessage::Response(PluginResponse {
                    request_id: response_id,
                    ok,
                    result,
                    error,
                    ..
                }) if response_id == request_id => {
                    if ok {
                        return Ok(result);
                    }
                    return Err(error
                        .map(|error| error.message)
                        .unwrap_or_else(|| "宿主拒绝了安全存储请求".into()));
                }
                PluginWireMessage::Event(_) => {}
                _ => return Err("宿主安全存储响应与请求不匹配".into()),
            }
        }
    }
}
