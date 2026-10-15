use agile_tiles_plugin_sdk::{
    ErrorCode, PluginEvent, PluginHandshakeResponse, PluginId, PluginRequest, PluginRequestKind,
    PluginWireMessage, API_VERSION,
};
use agile_tiles_plugin_ipc::{FrameReader, FrameWriter};
use agile_tiles_plugin_sdk::{
    ClipboardService, EventBus, FileService, GlobalShortcutService, HostError, HostServices,
    NotificationOptions, NotificationService, Plugin, PluginContext, SecretService,
    ShortcutAction, ShortcutHandler, StorageService,
    SystemService, WindowEventHandler, WindowService, WindowSpec,
};
use agile_tiles_plugin_ssh_manager::SshPlugin;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Value};
use std::env;
use std::fs::{File, OpenOptions};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

struct Transport {
    reader: FrameReader<File>,
    writer: FrameWriter<File>,
}

impl Transport {
    fn call(
        &mut self,
        plugin_id: &PluginId,
        method: &str,
        payload: Value,
    ) -> Result<Value, HostError> {
        let request = PluginRequest {
            request_id: request_id(),
            plugin_id: plugin_id.clone(),
            kind: PluginRequestKind::Call,
            method: Some(method.into()),
            payload,
            timeout_ms: Some(8_000),
        };
        let request_id = request.request_id.clone();
        self.writer
            .write_message(&PluginWireMessage::Request(request))
            .map_err(|error| HostError::new(format!("host request write failed: {error:?}")))?;
        loop {
            let message = self
                .reader
                .read_message()
                .map_err(|error| HostError::new(format!("host response read failed: {error:?}")))?;
            match message {
                PluginWireMessage::Response(response) if response.request_id == request_id => {
                    if response.ok {
                        return Ok(response.result);
                    }
                    return Err(HostError::new(
                        response
                            .error
                            .map(|error| error.message)
                            .unwrap_or_else(|| "host service failed".into()),
                    ));
                }
                PluginWireMessage::Event(_) => continue,
                _ => return Err(HostError::new("unexpected host response")),
            }
        }
    }
}

struct RemoteHostServices {
    plugin_id: PluginId,
    transport: Arc<Mutex<Transport>>,
}

impl RemoteHostServices {
    fn call(&self, method: &str, payload: Value) -> Result<Value, HostError> {
        self.transport
            .lock()
            .map_err(|_| HostError::new("host transport lock poisoned"))?
            .call(&self.plugin_id, method, payload)
    }
}

impl StorageService for RemoteHostServices {
    fn get(&self, _plugin_id: &PluginId, key: &str) -> Result<Option<String>, HostError> {
        self.call("host.storage.get", json!({ "key": key }))
            .and_then(|value| {
                serde_json::from_value(value).map_err(|error| HostError::new(error.to_string()))
            })
    }
    fn set(&self, _plugin_id: &PluginId, key: &str, value: &str) -> Result<(), HostError> {
        self.call("host.storage.set", json!({ "key": key, "value": value }))
            .map(|_| ())
    }
    fn remove(&self, _plugin_id: &PluginId, key: &str) -> Result<(), HostError> {
        self.call("host.storage.remove", json!({ "key": key }))
            .map(|_| ())
    }
}

impl SecretService for RemoteHostServices {
    fn get(&self, _plugin_id: &PluginId, key: &str) -> Result<Option<String>, HostError> {
        self.call("host.secrets.get", json!({ "key": key }))
            .and_then(|value| {
                serde_json::from_value(value).map_err(|error| HostError::new(error.to_string()))
            })
    }

    fn set(&self, _plugin_id: &PluginId, key: &str, value: &str) -> Result<(), HostError> {
        self.call("host.secrets.set", json!({ "key": key, "value": value }))
            .map(|_| ())
    }

    fn remove(&self, _plugin_id: &PluginId, key: &str) -> Result<(), HostError> {
        self.call("host.secrets.remove", json!({ "key": key }))
            .map(|_| ())
    }
}

impl NotificationService for RemoteHostServices {
    fn notify(&self, title: &str, body: &str) -> Result<(), HostError> {
        self.call(
            "host.notification.notify",
            json!({ "title": title, "body": body }),
        )
        .map(|_| ())
    }

    fn notify_with(&self, notification: &NotificationOptions) -> Result<(), HostError> {
        self.call("host.notification.notify", json!(notification))
            .map(|_| ())
    }
}

impl EventBus for RemoteHostServices {
    fn publish(&self, event: PluginEvent) -> Result<(), HostError> {
        self.call("host.events.publish", json!({ "event": event }))
            .map(|_| ())
    }
}

impl SystemService for RemoteHostServices {
    fn open_external(&self, url: &str) -> Result<(), HostError> {
        self.call("host.system.open_external", json!({ "url": url }))
            .map(|_| ())
    }
    fn spawn(&self, program: &str, args: &[String]) -> Result<(), HostError> {
        self.call(
            "host.system.spawn",
            json!({ "program": program, "args": args }),
        )
        .map(|_| ())
    }
    fn spawn_with_cwd(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<(), HostError> {
        self.call(
            "host.system.spawn_with_cwd",
            json!({ "program": program, "args": args, "cwd": cwd }),
        )
        .map(|_| ())
    }
    fn pick_file(&self, extensions: &[String]) -> Result<Option<String>, HostError> {
        self.call("host.system.pick_file", json!({ "extensions": extensions }))
            .and_then(|value| {
                serde_json::from_value(value).map_err(|error| HostError::new(error.to_string()))
            })
    }
    fn extract_icon(&self, path: &str) -> Result<Option<String>, HostError> {
        self.call("host.system.extract_icon", json!({ "path": path }))
            .and_then(|value| {
                serde_json::from_value(value).map_err(|error| HostError::new(error.to_string()))
            })
    }
}

impl FileService for RemoteHostServices {
    fn read_private(
        &self,
        _plugin_id: &PluginId,
        path: &str,
    ) -> Result<Option<Vec<u8>>, HostError> {
        self.call("host.files.read_private", json!({ "path": path }))
            .and_then(|value| {
                let encoded: Option<String> = serde_json::from_value(value)
                    .map_err(|error| HostError::new(error.to_string()))?;
                encoded
                    .map(|value| {
                        BASE64
                            .decode(value)
                            .map_err(|error| HostError::new(error.to_string()))
                    })
                    .transpose()
            })
    }
    fn write_private(
        &self,
        _plugin_id: &PluginId,
        path: &str,
        bytes: &[u8],
    ) -> Result<(), HostError> {
        self.call(
            "host.files.write_private",
            json!({ "path": path, "bytes": BASE64.encode(bytes) }),
        )
        .map(|_| ())
    }
    fn remove_private(&self, _plugin_id: &PluginId, path: &str) -> Result<(), HostError> {
        self.call("host.files.remove_private", json!({ "path": path }))
            .map(|_| ())
    }
    fn read_external(&self, path: &str) -> Result<Vec<u8>, HostError> {
        self.call("host.files.read_external", json!({ "path": path }))
            .and_then(|value| {
                let encoded: String = serde_json::from_value(value)
                    .map_err(|error| HostError::new(error.to_string()))?;
                BASE64
                    .decode(encoded)
                    .map_err(|error| HostError::new(error.to_string()))
            })
    }
    fn private_path(&self, _plugin_id: &PluginId, path: &str) -> Result<String, HostError> {
        self.call("host.files.private_path", json!({ "path": path }))
            .and_then(|value| {
                serde_json::from_value(value).map_err(|error| HostError::new(error.to_string()))
            })
    }
}

impl ClipboardService for RemoteHostServices {
    fn read_text(&self) -> Result<Option<String>, HostError> {
        self.call("host.clipboard.read", Value::Null)
            .and_then(|value| {
                serde_json::from_value(value).map_err(|error| HostError::new(error.to_string()))
            })
    }
    fn write_text(&self, text: &str) -> Result<(), HostError> {
        self.call("host.clipboard.write", json!({ "text": text }))
            .map(|_| ())
    }
}

impl GlobalShortcutService for RemoteHostServices {
    fn register(
        &self,
        _plugin_id: &PluginId,
        _bindings: &[(String, ShortcutAction)],
        _handler: ShortcutHandler,
    ) -> Result<(), HostError> {
        Ok(())
    }
    fn unregister(&self, _plugin_id: &PluginId) -> Result<(), HostError> {
        Ok(())
    }
}

impl WindowService for RemoteHostServices {
    fn create(
        &self,
        _spec: WindowSpec,
        _handler: Option<Arc<dyn WindowEventHandler>>,
    ) -> Result<(), HostError> {
        Ok(())
    }
    fn show(&self, _id: &str) -> Result<(), HostError> {
        Ok(())
    }
    fn hide(&self, _id: &str) -> Result<(), HostError> {
        Ok(())
    }
    fn close(&self, _id: &str) -> Result<(), HostError> {
        Ok(())
    }
    fn emit(&self, _id: &str, _topic: &str, _payload: Value) -> Result<(), HostError> {
        Ok(())
    }
    fn set_clickthrough(&self, _id: &str, _enabled: bool) -> Result<(), HostError> {
        Ok(())
    }
}

impl HostServices for RemoteHostServices {
    fn storage(&self) -> &dyn StorageService {
        self
    }
    fn secrets(&self) -> &dyn SecretService {
        self
    }
    fn notification(&self) -> &dyn NotificationService {
        self
    }
    fn events(&self) -> &dyn EventBus {
        self
    }
    fn system(&self) -> &dyn SystemService {
        self
    }
    fn files(&self) -> &dyn FileService {
        self
    }
    fn clipboard(&self) -> &dyn ClipboardService {
        self
    }
    fn shortcuts(&self) -> &dyn GlobalShortcutService {
        self
    }
    fn windows(&self) -> &dyn WindowService {
        self
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let pipe = argument(&args, "--pipe")?;
    let plugin_id =
        PluginId::new(argument(&args, "--plugin-id")?).map_err(|error| error.to_string())?;
    if plugin_id.as_str() != "ssh_manager" {
        return Err(format!("unsupported SSH plugin id: {plugin_id}"));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe)
        .map_err(|error| format!("open plugin pipe failed: {error}"))?;
    let reader_file = file.try_clone().map_err(|error| error.to_string())?;
    let transport = Arc::new(Mutex::new(Transport {
        reader: FrameReader::new(reader_file),
        writer: FrameWriter::new(file),
    }));
    let handshake = transport
        .lock()
        .map_err(|_| "transport lock poisoned".to_string())?
        .reader
        .read_message()
        .map_err(|error| format!("read handshake failed: {error:?}"))?;
    let PluginWireMessage::Handshake(handshake) = handshake else {
        return Err("first plugin message must be a handshake".into());
    };
    let accepted = handshake.plugin_id == plugin_id && handshake.api_version == API_VERSION;
    let response = PluginWireMessage::HandshakeResponse(PluginHandshakeResponse {
        request_id: handshake.request_id,
        plugin_id: plugin_id.clone(),
        accepted,
        api_version: API_VERSION,
        capabilities: vec![
            "plugin.call".into(),
            "storage".into(),
            "filesystem.read".into(),
            "filesystem.write".into(),
            "system.file_picker".into(),
            "process.spawn".into(),
            "network.connect".into(),
        ],
        error: (!accepted).then(|| agile_tiles_plugin_sdk::PluginError {
            code: ErrorCode::UnsupportedApiVersion,
            message: "plugin handshake rejected".into(),
        }),
    });
    transport
        .lock()
        .map_err(|_| "transport lock poisoned".to_string())?
        .writer
        .write_message(&response)
        .map_err(|error| format!("write handshake failed: {error:?}"))?;
    if !accepted {
        return Err("plugin handshake rejected".into());
    }
    let services: Arc<dyn HostServices> = Arc::new(RemoteHostServices {
        plugin_id: plugin_id.clone(),
        transport: Arc::clone(&transport),
    });
    let context = PluginContext::new(plugin_id, services);
    let mut plugin = SshPlugin::new();
    loop {
        let message = transport
            .lock()
            .map_err(|_| "transport lock poisoned".to_string())?
            .reader
            .read_message()
            .map_err(|error| format!("read plugin request failed: {error:?}"))?;
        let PluginWireMessage::Request(request) = message else {
            continue;
        };
        let should_stop = matches!(request.kind, PluginRequestKind::Shutdown);
        let response = plugin.handle(request, &context);
        transport
            .lock()
            .map_err(|_| "transport lock poisoned".to_string())?
            .writer
            .write_message(&PluginWireMessage::Response(response))
            .map_err(|error| format!("write plugin response failed: {error:?}"))?;
        if should_stop {
            break;
        }
    }
    Ok(())
}

fn argument(args: &[String], name: &str) -> Result<String, String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .ok_or_else(|| format!("missing argument: {name}"))
}

fn request_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("host-{nanos:x}")
}
