use sideweave_plugin_api::{
    permissions, ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind,
    PluginResponse, PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION,
    MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{
    success_response, DesktopNotificationKind, DesktopNotificationOptions,
    NotificationOptions, Plugin, PluginContext,
};
use serde_json::{json, Value};

const PLUGIN_ID: &str = "notification-lab";

pub struct NotificationLabPlugin {
    manifest: PluginManifest,
}

impl NotificationLabPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "Notification Lab".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "Test Windows, host desktop, and sidebar notifications".into(),
                author: "SideWeave".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec![
                    permissions::NOTIFICATION_SYSTEM.into(),
                    permissions::NOTIFICATION_DESKTOP.into(),
                    permissions::NOTIFICATION_SIDEBAR.into(),
                ],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Vue),
                    asset_root: "ui".into(),
                    initial_width: Some(620),
                    initial_height: Some(800),
                    min_width: Some(520),
                    min_height: Some(640),
                    chrome: None,
                }),
                hover_preview: None,
                auto_start: false,
                minimum_host_version: None,
            },
        }
    }

    fn send_windows(&self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        let Some(title) = request.payload.get("title").and_then(Value::as_str) else {
            return error_response(
                &request,
                ErrorCode::InvalidRequest,
                "title must be a string",
            );
        };
        let Some(body) = request.payload.get("body").and_then(Value::as_str) else {
            return error_response(&request, ErrorCode::InvalidRequest, "body must be a string");
        };

        let mut notification = NotificationOptions::new(title, body);
        if let Some(icon) = optional_hint(&request.payload, "icon") {
            notification = notification.with_icon(icon);
        }
        if let Some(sound) = optional_hint(&request.payload, "sound") {
            notification = notification.with_sound(sound);
        }

        match context.notification().notify_with(&notification) {
            Ok(()) => success_response(&request, json!({ "sent": true })),
            Err(error) => error_response(&request, ErrorCode::Internal, error.message),
        }
    }

    fn show_host_desktop(&self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        let Some(title) = request.payload.get("title").and_then(Value::as_str) else {
            return error_response(
                &request,
                ErrorCode::InvalidRequest,
                "title must be a string",
            );
        };
        let Some(body) = request.payload.get("body").and_then(Value::as_str) else {
            return error_response(&request, ErrorCode::InvalidRequest, "body must be a string");
        };
        let kind = match request
            .payload
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("info")
        {
            "info" => DesktopNotificationKind::Info,
            "success" => DesktopNotificationKind::Success,
            "warning" => DesktopNotificationKind::Warning,
            "alarm" => DesktopNotificationKind::Alarm,
            _ => {
                return error_response(
                    &request,
                    ErrorCode::InvalidRequest,
                    "unknown notification style",
                )
            }
        };
        let duration_ms = request
            .payload
            .get("durationMs")
            .and_then(Value::as_u64)
            .unwrap_or(6_000);
        let notification = DesktopNotificationOptions::new(title, body)
            .with_kind(kind)
            .with_duration_ms(duration_ms);

        match context.notification().show_desktop(&notification) {
            Ok(()) => success_response(&request, json!({ "shown": true })),
            Err(error) => error_response(&request, ErrorCode::Internal, error.message),
        }
    }

    fn show_host_sidebar(&self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        let Some(title) = request.payload.get("title").and_then(Value::as_str) else {
            return error_response(
                &request,
                ErrorCode::InvalidRequest,
                "title must be a string",
            );
        };
        let Some(body) = request.payload.get("body").and_then(Value::as_str) else {
            return error_response(&request, ErrorCode::InvalidRequest, "body must be a string");
        };
        let kind = match request
            .payload
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("info")
        {
            "info" => DesktopNotificationKind::Info,
            "success" => DesktopNotificationKind::Success,
            "warning" => DesktopNotificationKind::Warning,
            "alarm" => DesktopNotificationKind::Alarm,
            _ => {
                return error_response(
                    &request,
                    ErrorCode::InvalidRequest,
                    "unknown notification style",
                )
            }
        };
        let duration_ms = request
            .payload
            .get("durationMs")
            .and_then(Value::as_u64)
            .unwrap_or(6_000);
        let notification = DesktopNotificationOptions::new(title, body)
            .with_kind(kind)
            .with_duration_ms(duration_ms);

        match context.notification().show_sidebar(&notification) {
            Ok(()) => success_response(&request, json!({ "shown": true })),
            Err(error) => error_response(&request, ErrorCode::Internal, error.message),
        }
    }
}

impl Default for NotificationLabPlugin {
    fn default() -> Self {
        Self::new()
    }
}

sideweave_plugin_sdk::export_plugin_dylib!(NotificationLabPlugin);

impl Plugin for NotificationLabPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        match request.kind.clone() {
            PluginRequestKind::Activate
            | PluginRequestKind::Deactivate
            | PluginRequestKind::Shutdown => success_response(&request, Value::Null),
            PluginRequestKind::GetInfo => success_response(&request, json!(self.manifest())),
            PluginRequestKind::Call => match request.method.as_deref() {
                Some("notify") => self.send_windows(request, context),
                Some("show_desktop") => self.show_host_desktop(request, context),
                Some("show_sidebar") => self.show_host_sidebar(request, context),
                Some(method) => error_response(
                    &request,
                    ErrorCode::NotFound,
                    format!("unknown method: {method}"),
                ),
                None => error_response(&request, ErrorCode::InvalidRequest, "missing method"),
            },
        }
    }
}

fn optional_hint<'a>(payload: &'a Value, field: &str) -> Option<&'a str> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn error_response(
    request: &PluginRequest,
    code: ErrorCode,
    message: impl Into<String>,
) -> PluginResponse {
    PluginResponse {
        request_id: request.request_id.clone(),
        plugin_id: request.plugin_id.clone(),
        ok: false,
        result: Value::Null,
        error: Some(PluginError {
            code,
            message: message.into(),
        }),
    }
}
