use base64::Engine;
use image::GenericImageView;
use model::{Settings, StatusSnapshot};
use serde_json::{json, Value};
use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginHandshakeResponse, PluginId, PluginRequest, PluginRequestKind,
    PluginResponse, PluginWireMessage, API_VERSION,
};
use sideweave_plugin_ipc::{FrameReader, FrameWriter};
use std::env;
use std::fs::OpenOptions;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[cfg(windows)]
mod capture;
mod doc_export;
#[cfg(windows)]
mod export;
#[cfg(windows)]
mod hdr_capture;
mod history;
mod host_services;
mod model;
mod ocr;
#[cfg(windows)]
mod qr;
#[cfg(windows)]
mod recorder;
#[cfg(windows)]
mod scroll;
#[cfg(windows)]
mod scroll_input;
#[cfg(windows)]
mod session;
pub mod tencent_ocr;
mod theme;
mod toolbar;
mod translation;
#[cfg(windows)]
#[allow(dead_code)]
mod wgc;
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
        if let Err(error) = writer.write_message(&PluginWireMessage::Response(response)) {
            match error {
                sideweave_plugin_ipc::IpcError::Protocol(
                    sideweave_plugin_api::ProtocolError::MessageTooLarge(bytes),
                ) => {
                    let fallback = failure(
                        &request,
                        format!("返回数据超出 IPC 帧限制（{bytes} 字节），已安全拦截"),
                    );
                    writer
                        .write_message(&PluginWireMessage::Response(fallback))
                        .map_err(|e| format!("写入错误响应失败：{e:?}"))?;
                }
                other => return Err(format!("写入插件响应失败：{other:?}")),
            }
        }
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
                self.refresh_tencent_credentials(host).map(|_| json!(null))
            }
            PluginRequestKind::Deactivate => Ok(json!(null)),
            PluginRequestKind::Shutdown => {
                #[cfg(windows)]
                self.ui.shutdown();
                Ok(json!(null))
            }
            PluginRequestKind::GetInfo => self.refresh_tencent_credentials(host).map(|_| {
                json!({
                    "native": true,
                    "features": [
                        "region",
                        "annotation",
                        "scroll",
                        "ocr",
                        "offline_ocr",
                        "translation",
                        "in_place_translation",
                        "pin",
                        "quick_pin",
                        "demo",
                        "watermark",
                        "recording",
                        "animated_webp",
                        "gif",
                        "mp4",
                        "collage",
                        "pdf_export",
                        "pptx_export",
                        "history"
                    ]
                })
            }),
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
                value["systemPrefersLight"] = json!(theme::system_prefers_light());
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
                self.ui.update_hotkeys(next);
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
            Some("start_demo") => {
                #[cfg(windows)]
                {
                    self.ui.start_demo();
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("演示模式目前只支持 Windows".into())
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
                    let format = match request.payload.get("format").and_then(Value::as_str) {
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
            Some("quick_pin") => {
                #[cfg(windows)]
                {
                    self.ui.quick_pin();
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("贴图目前只支持 Windows".into())
                }
            }
            Some("pin_file") => {
                #[cfg(windows)]
                {
                    let path = request
                        .payload
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "缺少图片路径".to_string())?;
                    self.ui.pin_file(path.to_string());
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("贴图目前只支持 Windows".into())
                }
            }
            Some("copy_file") => {
                #[cfg(windows)]
                {
                    let path = request
                        .payload
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "缺少图片路径".to_string())?;
                    let img = image::open(Path::new(path))
                        .map_err(|e| format!("读取图片失败：{e}"))?;
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    export::copy_to_clipboard(w as usize, h as usize, rgba.as_raw())?;
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("仅支持 Windows".into())
                }
            }
            Some("copy_image_data_url") => {
                #[cfg(windows)]
                {
                    let data_url = request
                        .payload
                        .get("dataUrl")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "缺少图片数据".to_string())?;
                    let img = doc_export::decode_page_image(data_url)?;
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    export::copy_to_clipboard(w as usize, h as usize, rgba.as_raw())?;
                    Ok(json!(null))
                }
                #[cfg(not(windows))]
                {
                    Err("仅支持 Windows".into())
                }
            }
            Some("quick_translate") => {
                let text = request
                    .payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let source_lang = request
                    .payload
                    .get("sourceLang")
                    .and_then(Value::as_str)
                    .unwrap_or("auto");
                let target_lang = request
                    .payload
                    .get("targetLang")
                    .and_then(Value::as_str)
                    .unwrap_or("zh-CN");
                let provider_str = request
                    .payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or("youdao");
                let provider = translation::TranslateProvider::from_str(provider_str);
                let resp = translation::translate_text(text, source_lang, target_lang, provider)?;
                serde_json::to_value(resp).map_err(|e| e.to_string())
            }
            Some("get_history") => {
                let save_dir = self
                    .settings
                    .lock()
                    .map_err(|_| "设置锁已损坏")?
                    .resolved_save_directory();
                let kind = request.payload.get("kind").and_then(Value::as_str);
                let tag = request.payload.get("tag").and_then(Value::as_str);
                let query = request.payload.get("query").and_then(Value::as_str);
                let include_dir = request
                    .payload
                    .get("includeDirScan")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let items = history::list_items(&save_dir, kind, tag, query, include_dir);
                serde_json::to_value(items).map_err(|e| e.to_string())
            }
            Some("add_history_tag") => {
                let id = request
                    .payload
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少历史项 id".to_string())?;
                let path = request.payload.get("path").and_then(Value::as_str);
                let tag = request
                    .payload
                    .get("tag")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少标签内容".to_string())?;
                let tags = history::add_tag(id, path, tag)?;
                Ok(json!(tags))
            }
            Some("remove_history_tag") => {
                let id = request
                    .payload
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少历史项 id".to_string())?;
                let path = request.payload.get("path").and_then(Value::as_str);
                let tag = request
                    .payload
                    .get("tag")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少标签内容".to_string())?;
                let tags = history::remove_tag(id, path, tag)?;
                Ok(json!(tags))
            }
            Some("delete_history_item") => {
                let id = request
                    .payload
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少历史项 id".to_string())?;
                let path = request.payload.get("path").and_then(Value::as_str);
                let delete_file = request
                    .payload
                    .get("deleteFile")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                history::delete_item(id, path, delete_file)?;
                Ok(json!(null))
            }
            Some("clear_history") => {
                history::clear_all()?;
                Ok(json!(null))
            }
            Some("clear_thumb_cache") => {
                history::clear_thumb_cache()?;
                Ok(json!(null))
            }
            Some("open_history_file") => {
                let path = request
                    .payload
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少文件路径".to_string())?;
                let reveal = request
                    .payload
                    .get("reveal")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                open_or_reveal_file(path, reveal)?;
                Ok(json!(null))
            }
            Some("read_image_data_url") => {
                let path = request
                    .payload
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少图片路径".to_string())?;
                let max_dim = request
                    .payload
                    .get("maxDim")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32;
                read_image_as_data_url(path, max_dim)
            }
            Some("read_image_chunk") => {
                let path = request
                    .payload
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少图片路径".to_string())?;
                let max_dim = request
                    .payload
                    .get("maxDim")
                    .and_then(Value::as_u64)
                    .unwrap_or(1600) as u32;
                let chunk_index = request
                    .payload
                    .get("chunkIndex")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let chunk_size = request
                    .payload
                    .get("chunkSize")
                    .and_then(Value::as_u64)
                    .unwrap_or(140_000)
                    .clamp(16_384, 180_000) as usize;
                read_image_chunk(path, max_dim, chunk_index, chunk_size)
            }
            Some("upload_temp_chunk") => {
                let upload_id = request
                    .payload
                    .get("uploadId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少 uploadId".to_string())?;
                let chunk_b64 = request
                    .payload
                    .get("chunkBase64")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "缺少 chunkBase64".to_string())?;
                let append = request
                    .payload
                    .get("append")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let ext = request
                    .payload
                    .get("ext")
                    .and_then(Value::as_str)
                    .unwrap_or("png");
                let path = write_temp_upload_chunk(upload_id, chunk_b64, append, ext)?;
                Ok(json!({ "path": path }))
            }
            Some("choose_images") => {
                let paths = pick_image_files()?;
                Ok(json!(paths))
            }
            Some("export_document") => {
                let mut req: doc_export::ExportDocumentRequest =
                    serde_json::from_value(request.payload.clone())
                        .map_err(|e| format!("文档导出参数无效：{e}"))?;
                if req.save_directory.is_none() {
                    let dir = self
                        .settings
                        .lock()
                        .map_err(|_| "设置锁已损坏")?
                        .resolved_save_directory();
                    req.save_directory = Some(dir.to_string_lossy().into_owned());
                }
                let (path, width, height) = doc_export::export_document(&req)?;
                history::record_item("edit", &path, width, height);
                Ok(json!({
                    "path": path,
                    "width": width,
                    "height": height
                }))
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

/// Maximum base64 payload returned in a single `read_image_data_url` response
/// so the JSON envelope stays safely below `sideweave_plugin_api::MAX_MESSAGE_BYTES` (256 KB).
const SAFE_INLINE_B64_LIMIT: usize = 180 * 1024;

struct CachedChunkImage {
    key: String,
    mime: String,
    b64: String,
    width: u32,
    height: u32,
}

static CHUNK_IMAGE_CACHE: std::sync::OnceLock<Mutex<Option<CachedChunkImage>>> =
    std::sync::OnceLock::new();

fn encode_jpeg_bytes(img: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| format!("编码 JPEG 失败：{e}"))?;
    Ok(out)
}

fn read_image_as_data_url(path: &str, max_dim: u32) -> Result<Value, String> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err("文件不存在".into());
    }
    // Fast disk-cached JPEG thumbnail path for history / preview cards
    if max_dim > 0 && max_dim <= 480 {
        let (jpeg, orig_w, orig_h) = history::read_or_build_thumbnail(p, max_dim)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
        return Ok(json!({
            "dataUrl": format!("data:image/jpeg;base64,{b64}"),
            "width": orig_w,
            "height": orig_h
        }));
    }

    let img = image::open(p).map_err(|e| format!("读取图片失败：{e}"))?;
    let (orig_w, orig_h) = img.dimensions();
    let mut cur_dim = if max_dim > 0 { max_dim } else { 1280 };
    let mut quality = 86u8;
    for _ in 0..5 {
        let target = if orig_w > cur_dim || orig_h > cur_dim {
            img.thumbnail(cur_dim, cur_dim)
        } else {
            img.clone()
        };
        let bytes = encode_jpeg_bytes(&target, quality)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        if b64.len() <= SAFE_INLINE_B64_LIMIT || cur_dim <= 480 {
            return Ok(json!({
                "dataUrl": format!("data:image/jpeg;base64,{b64}"),
                "width": orig_w,
                "height": orig_h
            }));
        }
        cur_dim = (cur_dim * 3 / 4).max(480);
        quality = quality.saturating_sub(8).max(65);
    }
    Err("图片过大，无法内联传输".into())
}

fn read_image_chunk(
    path: &str,
    max_dim: u32,
    chunk_index: usize,
    chunk_size: usize,
) -> Result<Value, String> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err("文件不存在".into());
    }
    let meta = std::fs::metadata(p).map_err(|e| format!("读取文件信息失败：{e}"))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let cache_key = format!("{}|{}|{}|{}", path.to_ascii_lowercase(), meta.len(), mtime, max_dim);

    let cache_lock = CHUNK_IMAGE_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache_lock.lock().map_err(|_| "图片缓存锁已损坏")?;
    let needs_load = guard.as_ref().is_none_or(|c| c.key != cache_key);
    if needs_load {
        let img = image::open(p).map_err(|e| format!("读取图片失败：{e}"))?;
        let (orig_w, orig_h) = img.dimensions();
        let target = if max_dim > 0 && (orig_w > max_dim || orig_h > max_dim) {
            img.thumbnail(max_dim, max_dim)
        } else {
            img
        };
        let has_alpha = matches!(
            target.color(),
            image::ColorType::Rgba8
                | image::ColorType::La8
                | image::ColorType::Rgba16
                | image::ColorType::La16
                | image::ColorType::Rgba32F
        );
        let (mime, bytes) = if has_alpha {
            let mut png_buf = Cursor::new(Vec::new());
            if target.write_to(&mut png_buf, image::ImageFormat::Png).is_ok()
                && png_buf.get_ref().len() <= 900 * 1024
            {
                ("image/png".to_string(), png_buf.into_inner())
            } else {
                ("image/jpeg".to_string(), encode_jpeg_bytes(&target, 90)?)
            }
        } else {
            ("image/jpeg".to_string(), encode_jpeg_bytes(&target, 90)?)
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        *guard = Some(CachedChunkImage {
            key: cache_key,
            mime,
            b64,
            width: orig_w,
            height: orig_h,
        });
    }

    let entry = guard.as_ref().ok_or_else(|| "图片缓存为空".to_string())?;
    let total_len = entry.b64.len();
    let total_chunks = total_len.div_ceil(chunk_size).max(1);
    let start = (chunk_index * chunk_size).min(total_len);
    let end = (start + chunk_size).min(total_len);
    let chunk = &entry.b64[start..end];
    Ok(json!({
        "chunk": chunk,
        "chunkIndex": chunk_index,
        "totalChunks": total_chunks,
        "mime": entry.mime,
        "width": entry.width,
        "height": entry.height
    }))
}

fn write_temp_upload_chunk(
    upload_id: &str,
    chunk_b64: &str,
    append: bool,
    ext: &str,
) -> Result<String, String> {
    let safe_id: String = upload_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(64)
        .collect();
    if safe_id.is_empty() {
        return Err("无效的 uploadId".into());
    }
    let safe_ext = match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "jpg",
        "webp" => "webp",
        _ => "png",
    };
    let dir = env::temp_dir().join("SideWeave").join("studio-upload");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败：{e}"))?;
    let file_path = dir.join(format!("{safe_id}.{safe_ext}"));
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(chunk_b64.trim())
        .map_err(|e| format!("Base64 解码失败：{e}"))?;
    use std::io::Write;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&file_path)
        .map_err(|e| format!("写入临时图片失败：{e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("写入临时图片数据失败：{e}"))?;
    Ok(file_path.to_string_lossy().into_owned())
}

fn pick_image_files() -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        use std::mem::{size_of, zeroed};
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Controls::Dialogs::{
            GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST,
            OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

        let title: Vec<u16> = std::ffi::OsStr::new("选择图片")
            .encode_wide()
            .chain(Some(0))
            .collect();
        let filter: Vec<u16> =
            std::ffi::OsStr::new("图片文件 (*.png;*.jpg;*.jpeg;*.webp;*.bmp)\0*.png;*.jpg;*.jpeg;*.webp;*.bmp\0所有文件 (*.*)\0*.*\0\0")
                .encode_wide()
                .collect();
        let mut buffer = vec![0u16; 32 * 1024];
        let mut ofn: OPENFILENAMEW = unsafe { zeroed() };
        ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = unsafe { GetForegroundWindow() };
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = buffer.as_mut_ptr();
        ofn.nMaxFile = buffer.len() as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.nFilterIndex = 1;
        ofn.Flags = OFN_EXPLORER
            | OFN_ALLOWMULTISELECT
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR;

        let ok = unsafe { GetOpenFileNameW(&mut ofn) };
        if ok == 0 {
            return Ok(Vec::new());
        }

        // Parse double-null terminated list:
        // Single file: "C:\dir\file.png\0\0"
        // Multi file:  "C:\dir\0file1.png\0file2.png\0\0"
        let mut parts: Vec<String> = Vec::new();
        let mut start = 0usize;
        while start < buffer.len() {
            let rel_end = buffer[start..]
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(buffer.len() - start);
            if rel_end == 0 {
                break;
            }
            parts.push(String::from_utf16_lossy(&buffer[start..start + rel_end]));
            start += rel_end + 1;
        }
        if parts.is_empty() {
            return Ok(Vec::new());
        }
        if parts.len() == 1 {
            return Ok(parts);
        }
        let dir = Path::new(&parts[0]);
        let paths = parts[1..]
            .iter()
            .map(|name| dir.join(name).to_string_lossy().into_owned())
            .collect();
        Ok(paths)
    }
    #[cfg(not(windows))]
    {
        Err("仅支持 Windows".into())
    }
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
            .args(["-STA", "-NoProfile", "-NonInteractive", "-Command", script])
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

fn open_or_reveal_file(path: &str, reveal: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        if reveal {
            std::process::Command::new("explorer")
                .args(["/select,", path])
                .spawn()
                .map_err(|e| format!("定位文件失败: {e}"))?;
        } else {
            std::process::Command::new("explorer")
                .arg(path)
                .spawn()
                .map_err(|e| format!("打开文件失败: {e}"))?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (path, reveal);
        Err("仅支持 Windows".into())
    }
}

#[cfg(test)]
mod studio_ipc_tests {
    use super::*;

    #[test]
    fn chunked_image_read_and_upload_stay_within_ipc_frame_budget() {
        let dir = env::temp_dir().join("sideweave-studio-ipc-test");
        let _ = std::fs::create_dir_all(&dir);
        let img_path = dir.join("sample_large.png");
        // Create a noisy 900x600 image that would exceed 256KB as uncompressed PNG
        let mut img = image::RgbaImage::new(900, 600);
        let mut seed = 0x1234_5678u32;
        for px in img.pixels_mut() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *px = image::Rgba([
                (seed >> 16) as u8,
                (seed >> 8) as u8,
                seed as u8,
                255,
            ]);
        }
        img.save(&img_path).unwrap();

        let path_str = img_path.to_string_lossy().into_owned();
        // 1. Thumbnail cache read_image_as_data_url must stay well below 256 KB
        let thumb = read_image_as_data_url(&path_str, 340).unwrap();
        let thumb_json = serde_json::to_vec(&thumb).unwrap();
        assert!(
            thumb_json.len() < sideweave_plugin_api::MAX_MESSAGE_BYTES,
            "thumbnail exceeded 256KB IPC limit: {} bytes",
            thumb_json.len()
        );

        // 2. Chunked high-res read_image_chunk must reassemble into a valid image
        let first = read_image_chunk(&path_str, 1600, 0, 120_000).unwrap();
        let total = first["totalChunks"].as_u64().unwrap() as usize;
        assert!(total >= 1);
        let mut stitched = first["chunk"].as_str().unwrap().to_string();
        for idx in 1..total {
            let part = read_image_chunk(&path_str, 1600, idx, 120_000).unwrap();
            let part_json = serde_json::to_vec(&part).unwrap();
            assert!(part_json.len() < sideweave_plugin_api::MAX_MESSAGE_BYTES);
            stitched.push_str(part["chunk"].as_str().unwrap());
        }
        let decoded_bytes = base64::engine::general_purpose::STANDARD
            .decode(&stitched)
            .unwrap();
        let decoded_img = image::load_from_memory(&decoded_bytes).unwrap();
        assert_eq!(decoded_img.dimensions(), (900, 600));

        // 3. Chunked temp upload must reassemble identically on disk
        let upload_id = format!("test_up_{}", std::process::id());
        let mut uploaded_path = String::new();
        for (i, chunk) in stitched.as_bytes().chunks(120_000).enumerate() {
            let chunk_str = std::str::from_utf8(chunk).unwrap();
            uploaded_path = write_temp_upload_chunk(&upload_id, chunk_str, i > 0, "jpg").unwrap();
        }
        let disk_bytes = std::fs::read(&uploaded_path).unwrap();
        assert_eq!(disk_bytes, decoded_bytes);

        let _ = std::fs::remove_file(&uploaded_path);
        let _ = std::fs::remove_file(&img_path);
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
