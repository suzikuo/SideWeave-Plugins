use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginHandshakeResponse, PluginId, PluginRequest,
    PluginRequestKind, PluginResponse, PluginWireMessage, API_VERSION,
};
use sideweave_plugin_ipc::{FrameReader, FrameWriter};
use model::{Settings, StatusSnapshot};
use serde_json::{json, Value};
use std::env;
use std::fs::OpenOptions;
use std::sync::{Arc, Mutex};

#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod hdr_capture;
#[cfg(windows)]
mod export;
mod host_services;
mod model;
#[cfg(windows)]
mod recorder;
#[cfg(windows)]
mod qr;
#[cfg(windows)]
mod scroll;
#[cfg(windows)]
mod scroll_input;
#[cfg(windows)]
mod session;
pub mod tencent_ocr;
mod toolbar;
mod translation;
#[cfg(windows)]
mod windows;

const PLUGIN_ID: &str = "screenshot";
const TENCENT_CREDENTIALS_KEY: &str = "tencent-ocr-credentials";

fn main() -> Result<(), String> {
    #[cfg(windows)]
    windows::enable_dpi_awareness();

    let args: Vec<String> = env::args().collect();
    let pipe = argument(&args, "--pipe")?;
    let plugin_id = PluginId::new(argument(&args, "--plugin-id")?).map_err(|e| e.to_string())?;
    if plugin_id.as_str() != PLUGIN_ID {
        return Err(format!("不支持的截图插件 id：{plugin_id}"));
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe)
        .map_err(|error| format!("打开插件管道失败：{error}"))?;
    let reader_file = file.try_clone().map_err(|error| error.to_string())?;
    let mut reader = FrameReader::new(reader_file);
    let mut writer = FrameWriter::new(file);

    let handshake = reader
        .read_message()
        .map_err(|error| format!("读取插件握手失败：{error:?}"))?;
    let PluginWireMessage::Handshake(handshake) = handshake else {
        return Err("插件第一条消息必须是握手".into());
    };
    let accepted = handshake.plugin_id == plugin_id && handshake.api_version == API_VERSION;
    let handshake_response = PluginWireMessage::HandshakeResponse(PluginHandshakeResponse {
        request_id: handshake.request_id,
        plugin_id: plugin_id.clone(),
        accepted,
        api_version: API_VERSION,
        capabilities: vec![
            "plugin.call".into(),
            "storage".into(),
            "native.capture".into(),
            "native.clipboard_image".into(),
            "native.ocr".into(),
            "native.pin".into(),
        ],
        error: (!accepted).then(|| PluginError {
            code: ErrorCode::UnsupportedApiVersion,
            message: "截图核心握手被拒绝".into(),
        }),
    });
    writer
        .write_message(&handshake_response)
        .map_err(|error| format!("写入握手响应失败：{error:?}"))?;
    if !accepted {
        return Err("截图核心握手被拒绝".into());
    }

    let settings = Arc::new(Mutex::new(Settings::load()));
    let image = Arc::new(Mutex::new(None));
    let status = Arc::new(Mutex::new(StatusSnapshot::default()));
    let tencent_credentials = Arc::new(Mutex::new(None));
    #[cfg(windows)]
    let recording = recorder::RecordingManager::default();
    #[cfg(windows)]
    let ui = windows::start(
        Arc::clone(&settings),
        Arc::clone(&image),
        Arc::clone(&status),
        recording.clone(),
        Arc::clone(&tencent_credentials),
    );
    let core = Core {
        settings,
        status,
        tencent_credentials,
        #[cfg(windows)]
        recording,
        #[cfg(windows)]
        ui,
    };

    loop {
        let message = reader
            .read_message()
            .map_err(|error| format!("读取插件请求失败：{error:?}"))?;
        let PluginWireMessage::Request(request) = message else {
            continue;
        };
        let should_stop = matches!(request.kind, PluginRequestKind::Shutdown);
        let mut host =
            host_services::HostServicesClient::new(&mut reader, &mut writer, plugin_id.clone());
        let response = core.handle(&request, &mut host);
        drop(host);
        writer
            .write_message(&PluginWireMessage::Response(response))
            .map_err(|error| format!("写入插件响应失败：{error:?}"))?;
        if should_stop {
            break;
        }
    }
    Ok(())
}

struct Core {
    settings: Arc<Mutex<Settings>>,
    status: Arc<Mutex<StatusSnapshot>>,
    tencent_credentials: Arc<Mutex<Option<tencent_ocr::TencentCloudCredentials>>>,
    #[cfg(windows)]
    recording: recorder::RecordingManager,
    #[cfg(windows)]
    ui: windows::UiBridge,
}

impl Core {
    fn handle(
        &self,
        request: &PluginRequest,
        host: &mut host_services::HostServicesClient<'_>,
    ) -> PluginResponse {
        let result = match request.kind {
            PluginRequestKind::Call => self.call(request, host),
            PluginRequestKind::Activate => {
                // The native hotkey can open the capture toolbar without going
                // through `start_capture`, so warm the in-memory OCR credentials
                // as soon as the host activates this plugin.
                self.refresh_tencent_credentials(host).map(|_| json!(null))
            }
            PluginRequestKind::Deactivate => Ok(json!(null)),
            PluginRequestKind::Shutdown => {
                #[cfg(windows)]
                self.ui.shutdown();
                Ok(json!(null))
            }
            PluginRequestKind::GetInfo => {
                // Some hosts query info without sending Activate first. Load the
                // credentials here as well so a hotkey-triggered toolbar works
                // immediately after the plugin process is restarted.
                self.refresh_tencent_credentials(host).map(|_| {
                    json!({
                        "native": true,
                        "features": ["region", "annotation", "scroll", "ocr", "translation", "pin", "watermark", "recording", "animated_webp", "gif", "mp4"]
                    })
                })
            }
        };
        match result {
            Ok(result) => success(request, result),
            Err(message) => failure(request, message),
        }
    }

    fn call(
        &self,
        request: &PluginRequest,
        host: &mut host_services::HostServicesClient<'_>,
    ) -> Result<Value, String> {
        match request.method.as_deref() {
            Some("get_settings") => {
                let configured = self.refresh_tencent_credentials(host)?;
                let settings = self.settings.lock().map_err(|_| "设置锁已损坏")?.clone();
                let mut value =
                    serde_json::to_value(settings).map_err(|error| error.to_string())?;
                value["tencentOcrConfigured"] = json!(configured);
                Ok(value)
            }
            Some("set_settings") => {
                let mut next: Settings = serde_json::from_value(request.payload.clone())
                    .map_err(|error| format!("设置格式无效：{error}"))?;
                next.normalize();
                self.update_tencent_credentials(host, &request.payload)?;
                next.save()?;
                *self.settings.lock().map_err(|_| "设置锁已损坏")? = next.clone();
                #[cfg(windows)]
                self.ui.update_hotkey(&next.hotkey);
                Ok(json!(null))
            }
            Some("start_capture") => {
                #[cfg(windows)]
                {
                    self.refresh_tencent_credentials(host)?;
                    self.ui.start(
                        request
                            .payload
                            .get("mode")
                            .and_then(Value::as_str)
                            .unwrap_or("region")
                            == "scroll",
                    );
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("截图核心目前只支持 Windows".into())
                }
            }
            Some("start_recording") => {
                #[cfg(windows)]
                {
                    let fps = request
                        .payload
                        .get("fps")
                        .and_then(Value::as_u64)
                        .unwrap_or(15) as u32;
                    self.ui.start_recording(fps);
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("屏幕录制目前只支持 Windows".into())
                }
            }
            Some("stop_recording") => {
                #[cfg(windows)]
                {
                    self.ui.stop_recording();
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("屏幕录制目前只支持 Windows".into())
                }
            }
            Some("cancel_recording") => {
                #[cfg(windows)]
                {
                    self.ui.cancel_recording();
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("屏幕录制目前只支持 Windows".into())
                }
            }
            Some("export_recording") => {
                #[cfg(windows)]
                {
                    let format = match request
                        .payload
                        .get("format")
                        .and_then(Value::as_str)
                    {
                        Some("gif") => recorder::AnimationFormat::Gif,
                        Some("webp") => recorder::AnimationFormat::Webp,
                        Some("mp4") => recorder::AnimationFormat::Mp4,
                        _ => return Err("导出格式必须是 MP4、GIF 或 WebP".into()),
                    };
                    self.ui.export_recording(format);
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("屏幕录制目前只支持 Windows".into())
                }
            }
            Some("clear_recording") => {
                #[cfg(windows)]
                {
                    self.ui.clear_recording();
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("屏幕录制目前只支持 Windows".into())
                }
            }
            Some("cancel_capture") => {
                #[cfg(windows)]
                self.ui.cancel();
                Ok(json!(null))
            }
            Some("get_status") => {
                let mut value = self.status.lock().map_err(|_| "状态锁已损坏")?.value();
                #[cfg(windows)]
                {
                    value["recording"] = json!(self.recording.snapshot());
                }
                Ok(value)
            }
            Some("pin_last") => {
                #[cfg(windows)]
                {
                    self.ui.pin_last();
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("贴图目前只支持 Windows".into())
                }
            }
            Some("clear_ocr") => {
                self.status.lock().map_err(|_| "状态锁已损坏")?.ocr_text = None;
                Ok(json!(null))
            }
            Some("choose_save_directory") => {
                let path = pick_directory()?;
                Ok(json!(path))
            }
            Some("open_save_directory") => {
                let target = request
                    .payload
                    .get("path")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        self.settings
                            .lock()
                            .ok()
                            .and_then(|s| {
                                if !s.save_directory.trim().is_empty() {
                                    Some(s.save_directory.clone())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_default()
                    });
                open_directory(&target)?;
                Ok(json!(null))
            }
            Some(method) => Err(format!("不支持的截图方法：{method}")),
            None => Err("缺少插件方法".into()),
        }
    }

    fn refresh_tencent_credentials(
        &self,
        host: &mut host_services::HostServicesClient<'_>,
    ) -> Result<bool, String> {
        let stored = host.get_secret(TENCENT_CREDENTIALS_KEY)?;
        let credentials = stored
            .as_deref()
            .map(serde_json::from_str::<StoredTencentCredentials>)
            .transpose()
            .map_err(|_| "宿主中保存的腾讯云 OCR 凭据格式无效".to_string())?
            .filter(|value| !value.secret_id.trim().is_empty() && !value.secret_key.is_empty())
            .map(StoredTencentCredentials::into_credentials);
        let configured = credentials.is_some();
        *self
            .tencent_credentials
            .lock()
            .map_err(|_| "OCR 凭据锁已损坏")? = credentials;
        Ok(configured)
    }

    fn update_tencent_credentials(
        &self,
        host: &mut host_services::HostServicesClient<'_>,
        payload: &Value,
    ) -> Result<(), String> {
        let Some(update) = payload.get("tencentCredentials") else {
            return Ok(());
        };
        let update: TencentCredentialsUpdate = serde_json::from_value(update.clone())
            .map_err(|error| format!("腾讯云 OCR 凭据格式无效：{error}"))?;
        if update.clear {
            host.remove_secret(TENCENT_CREDENTIALS_KEY)?;
            *self
                .tencent_credentials
                .lock()
                .map_err(|_| "OCR 凭据锁已损坏")? = None;
            return Ok(());
        }

        let secret_id = update.secret_id.trim();
        let secret_key = update.secret_key.trim();
        if secret_id.is_empty() && secret_key.is_empty() && update.token.trim().is_empty() {
            return Ok(());
        }
        if secret_id.is_empty() || secret_key.is_empty() {
            return Err("更新腾讯云凭据时，SecretId 和 SecretKey 都必须填写".into());
        }
        let stored = StoredTencentCredentials {
            secret_id: secret_id.to_string(),
            secret_key: secret_key.to_string(),
            token: update.token.trim().to_string(),
        };
        let serialized = serde_json::to_string(&stored).map_err(|error| error.to_string())?;
        host.set_secret(TENCENT_CREDENTIALS_KEY, &serialized)?;
        *self
            .tencent_credentials
            .lock()
            .map_err(|_| "OCR 凭据锁已损坏")? = Some(stored.into_credentials());
        Ok(())
    }

}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TencentCredentialsUpdate {
    secret_id: String,
    secret_key: String,
    token: String,
    clear: bool,
}

impl Default for TencentCredentialsUpdate {
    fn default() -> Self {
        Self {
            secret_id: String::new(),
            secret_key: String::new(),
            token: String::new(),
            clear: false,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredTencentCredentials {
    secret_id: String,
    secret_key: String,
    #[serde(default)]
    token: String,
}

impl StoredTencentCredentials {
    fn into_credentials(self) -> tencent_ocr::TencentCloudCredentials {
        let credentials =
            tencent_ocr::TencentCloudCredentials::new(self.secret_id, self.secret_key);
        if self.token.trim().is_empty() {
            credentials
        } else {
            credentials.with_token(self.token)
        }
    }
}

fn success(request: &PluginRequest, result: Value) -> PluginResponse {
    PluginResponse {
        request_id: request.request_id.clone(),
        plugin_id: request.plugin_id.clone(),
        ok: true,
        result,
        error: None,
    }
}

fn failure(request: &PluginRequest, message: String) -> PluginResponse {
    PluginResponse {
        request_id: request.request_id.clone(),
        plugin_id: request.plugin_id.clone(),
        ok: false,
        result: Value::Null,
        error: Some(PluginError {
            code: ErrorCode::Internal,
            message,
        }),
    }
}

fn argument(args: &[String], name: &str) -> Result<String, String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .ok_or_else(|| format!("缺少参数：{name}"))
}

fn pick_directory() -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = '选择截图保存目录'
$dialog.ShowNewFolderButton = $true
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    Write-Output $dialog.SelectedPath
}
"#;
        let output = Command::new("powershell")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .map_err(|e| format!("启动文件夹选择器失败: {e}"))?;

        if !output.status.success() {
            return Ok(None);
        }
        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if selected.is_empty() {
            Ok(None)
        } else {
            Ok(Some(selected))
        }
    }
    #[cfg(not(windows))]
    {
        Err("仅支持 Windows".into())
    }
}

fn open_directory(path: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let target = if path.trim().is_empty() {
            std::env::var_os("USERPROFILE")
                .map(std::path::PathBuf::from)
                .map(|p| p.join("Pictures").join("SideWeave").join("Screenshots"))
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        } else {
            std::path::PathBuf::from(path.trim())
        };
        let _ = std::fs::create_dir_all(&target);
        std::process::Command::new("explorer")
            .arg(&target)
            .spawn()
            .map_err(|e| format!("打开目录失败: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("仅支持 Windows".into())
    }
}
