use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginEvent, PluginManifest, PluginRequest, PluginRequestKind,
    PluginResponse, PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION,
    MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{
    success_response, DesktopNotificationKind, DesktopNotificationOptions, HostServices,
    Plugin, PluginContext, WindowEvent, WindowEventHandler, WindowSpec,
};
use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, Local, NaiveTime, TimeZone, Timelike,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(windows)]
mod native_clock;

const PLUGIN_ID: &str = "time";
const STATE_KEY: &str = "state";
const CLOCK_WINDOW: &str = "clock";
const ALARM_WARNING_SECS: i64 = 5 * 60;
const DEFAULT_NOTIFICATION_DURATION_SECS: u32 = 12;
const MIN_NOTIFICATION_DURATION_SECS: u32 = 3;
const MAX_NOTIFICATION_DURATION_SECS: u32 = 60;
const MAX_ALARM_INTERVAL_MINUTES: u32 = 10_080;
const DEFAULT_INTERVAL_MESSAGE: &str = "时间到了，休息一下吧。";

sideweave_plugin_sdk::export_plugin_dylib!(TimePlugin);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alarm {
    pub id: String,
    pub label: String,
    pub rule: AlarmRule,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AlarmRule {
    Time {
        hour: u8,
        minute: u8,
        weekdays: Vec<u8>,
    },
    Interval {
        interval_minutes: u32,
        next_fire_at_secs: i64,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockStyle {
    pub background_opacity: f64,
    pub content_opacity: f64,
    pub scale: f64,
    pub color: String,
    pub pos_x: Option<i32>,
    pub pos_y: Option<i32>,
    pub clock_visible: bool,
    pub skin: String,
    pub mini_time_format: String,
    pub mini_layout: String,
    pub font_weight: u16,
    pub pomo_work_min: u32,
    pub pomo_break_min: u32,
    pub pinned: bool,
}

impl Default for ClockStyle {
    fn default() -> Self {
        Self {
            background_opacity: 0.85,
            content_opacity: 1.0,
            scale: 1.0,
            color: "accent".into(),
            pos_x: None,
            pos_y: None,
            clock_visible: false,
            skin: "mini".into(),
            mini_time_format: "hm".into(),
            mini_layout: "horizontal".into(),
            font_weight: 500,
            pomo_work_min: 25,
            pomo_break_min: 5,
            pinned: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmInput {
    pub label: String,
    pub rule: AlarmRule,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationMethod {
    Native,
    Desktop,
    Sidebar,
}

impl Default for NotificationMethod {
    fn default() -> Self {
        Self::Sidebar
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NextAlarm {
    pub alarm_id: String,
    pub at_secs: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeSnapshot {
    pub now_local: String,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub weekday: u8,
    pub alarms: Vec<Alarm>,
    pub next: Option<NextAlarm>,
    pub notification_method: NotificationMethod,
    pub notification_duration_secs: u32,
    pub alarm_clock_flash_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct TimeState {
    alarms: Vec<Alarm>,
    clock_style: ClockStyle,
    notification_method: NotificationMethod,
    notification_duration_secs: u32,
    alarm_clock_flash_enabled: bool,
}

impl Default for TimeState {
    fn default() -> Self {
        Self {
            alarms: Vec::new(),
            clock_style: ClockStyle::default(),
            notification_method: NotificationMethod::default(),
            notification_duration_secs: DEFAULT_NOTIFICATION_DURATION_SECS,
            alarm_clock_flash_enabled: true,
        }
    }
}

pub struct TimePlugin {
    manifest: PluginManifest,
    stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
    state_lock: Arc<Mutex<()>>,
    alarms: Arc<Mutex<Vec<Alarm>>>,
    last_fired: Arc<Mutex<HashMap<String, String>>>,
}

impl Default for TimePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl TimePlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "时钟".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "时钟、闹钟、悬浮钟和番茄钟".into(),
                author: "SideWeave".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec![
                    "storage".into(),
                    "window.create".into(),
                    "window.manage".into(),
                    "event.publish".into(),
                    "notification.desktop".into(),
                    "notification.sidebar".into(),
                    "notification.system".into(),
                ],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Html),
                    asset_root: "ui".into(),
                    initial_width: Some(760),
                    initial_height: Some(820),
                    ..Default::default()
                }),
                hover_preview: Some(sideweave_plugin_api::PluginHoverPreview {
                    width: Some(280),
                    height: Some(180),
                    data_method: Some("get_preview".into()),
                    entry: None,
                }),
                auto_start: true,
                minimum_host_version: None,
            },
            stop: Arc::new(AtomicBool::new(true)),
            ticker: None,
            state_lock: Arc::new(Mutex::new(())),
            alarms: Arc::new(Mutex::new(Vec::new())),
            last_fired: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn activate(&mut self, context: &PluginContext) -> Result<(), String> {
        self.stop_ticker();
        self.stop.store(false, Ordering::SeqCst);
        let state = load(context);
        if let Ok(mut alarms) = self.alarms.lock() {
            *alarms = state.alarms;
        }
        restore_clock(context, &self.state_lock)?;
        let services = context.services_arc();
        let plugin_id = context.plugin_id().clone();
        let stop = Arc::clone(&self.stop);
        let state_lock = Arc::clone(&self.state_lock);
        let alarms = Arc::clone(&self.alarms);
        let last_fired = Arc::clone(&self.last_fired);
        self.ticker = Some(thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                tick(
                    &services,
                    &plugin_id,
                    &state_lock,
                    &alarms,
                    &last_fired,
                );
                thread::sleep(Duration::from_secs(1));
            }
        }));
        Ok(())
    }

    fn stop_ticker(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(ticker) = self.ticker.take() {
            let _ = ticker.join();
        }
    }

    fn replace_alarm_cache(&self, alarms: &[Alarm]) {
        if let Ok(mut cached) = self.alarms.lock() {
            *cached = alarms.to_vec();
        }
    }

    fn call(&self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        match request.method.as_deref() {
            Some("get_preview") => {
                let now = Local::now();
                let state = load(context);
                let enabled_alarms = state.alarms.iter().filter(|a| a.enabled).count();
                Ok(json!({
                    "title": "时钟与闹钟",
                    "status": "online",
                    "statusText": now.format("%H:%M:%S").to_string(),
                    "subtitle": now.format("%Y年%m月%d日").to_string(),
                    "metrics": [
                        { "label": "活跃闹钟", "value": format!("{} 个", enabled_alarms) },
                        { "label": "当前星期", "value": now.format("%A").to_string() }
                    ],
                    "actions": [
                        { "id": "open", "label": "打开面板", "primary": true }
                    ]
                }))
            }
            Some("list_alarms") => Ok(json!(load(context).alarms)),
            Some("add_alarm") => {
                let input: AlarmInput = parse(&request.payload, "alarm")?;
                validate(&input)?;
                let alarm = alarm_from_input(uuid::Uuid::new_v4().to_string(), input);
                let _guard = self.state_lock.lock().unwrap();
                let mut state = load(context);
                state.alarms.push(alarm.clone());
                save(context, &state)?;
                self.replace_alarm_cache(&state.alarms);
                emit_clock_snapshot(context);
                Ok(json!(alarm))
            }
            Some("update_alarm") => {
                let id = string_field(&request.payload, "id")?;
                let input: AlarmInput = parse(&request.payload, "alarm")?;
                validate(&input)?;
                let _guard = self.state_lock.lock().unwrap();
                let mut state = load(context);
                let alarm = state
                    .alarms
                    .iter_mut()
                    .find(|alarm| alarm.id == id)
                    .ok_or_else(|| "闹钟不存在".to_string())?;
                *alarm = alarm_from_input(id, input);
                let result = alarm.clone();
                save(context, &state)?;
                self.replace_alarm_cache(&state.alarms);
                emit_clock_snapshot(context);
                Ok(json!(result))
            }
            Some("remove_alarm") => {
                let id = string_field(&request.payload, "id")?;
                let _guard = self.state_lock.lock().unwrap();
                let mut state = load(context);
                let before = state.alarms.len();
                state.alarms.retain(|alarm| alarm.id != id);
                if state.alarms.len() == before {
                    return Err("闹钟不存在".into());
                }
                save(context, &state)?;
                self.replace_alarm_cache(&state.alarms);
                emit_clock_snapshot(context);
                Ok(Value::Null)
            }
            Some("snapshot") => Ok(json!(snapshot(context))),
            Some("save_notification_method") => {
                let method: NotificationMethod = parse(&request.payload, "method")?;
                let _guard = self.state_lock.lock().unwrap();
                let mut state = load(context);
                state.notification_method = method;
                save(context, &state)?;
                Ok(json!(method))
            }
            Some("save_notification_duration") => {
                let duration_secs = request
                    .payload
                    .get("duration_secs")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| "缺少有效的 duration_secs 字段".to_string())?;
                if !(MIN_NOTIFICATION_DURATION_SECS..=MAX_NOTIFICATION_DURATION_SECS)
                    .contains(&duration_secs)
                {
                    return Err(format!(
                        "通知时长需在 {MIN_NOTIFICATION_DURATION_SECS} 到 {MAX_NOTIFICATION_DURATION_SECS} 秒之间"
                    ));
                }
                let _guard = self.state_lock.lock().unwrap();
                let mut state = load(context);
                state.notification_duration_secs = duration_secs;
                save(context, &state)?;
                Ok(json!(duration_secs))
            }
            Some("set_alarm_clock_flash") => {
                let enabled = request
                    .payload
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| "缺少有效的 enabled 字段".to_string())?;
                let guard = self.state_lock.lock().unwrap();
                let mut state = load(context);
                state.alarm_clock_flash_enabled = enabled;
                save(context, &state)?;
                drop(guard);
                emit_clock_snapshot(context);
                Ok(json!(enabled))
            }
            Some("toggle_clock") => toggle_clock(context, &self.state_lock),
            Some("set_pinned") => set_pinned(context, request, &self.state_lock),
            Some("get_clock_style") => Ok(json!(load(context).clock_style)),
            Some("save_clock_style") => {
                let style: ClockStyle = parse(&request.payload, "style")?;
                save_clock_style(context, style, &self.state_lock)
            }
            Some("notify") => {
                let title = string_field(&request.payload, "title")?;
                let body = string_field(&request.payload, "body")?;
                notify_time(
                    context.services(),
                    context.plugin_id(),
                    &DesktopNotificationOptions::new(title, body)
                        .with_kind(DesktopNotificationKind::Alarm),
                )
                .map_err(|e| e.message)?;
                Ok(Value::Null)
            }
            Some("raise_clock") => {
                context
                    .windows()
                    .focus(CLOCK_WINDOW)
                    .map_err(|e| e.message)?;
                Ok(Value::Null)
            }
            Some("window_position") => Ok(json!(context
                .windows()
                .position(CLOCK_WINDOW)
                .map_err(|e| e.message)?)),
            Some(method) => Err(format!("不支持的时钟方法: {method}")),
            None => Err("缺少插件方法".into()),
        }
    }
}

impl Plugin for TimePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        match &request.kind {
            PluginRequestKind::Activate => match self.activate(context) {
                Ok(()) => success_response(&request, Value::Null),
                Err(message) => error_response(request, message),
            },
            PluginRequestKind::Deactivate | PluginRequestKind::Shutdown => {
                self.stop_ticker();
                success_response(&request, Value::Null)
            }
            PluginRequestKind::Call => match self.call(&request, context) {
                Ok(result) => success_response(&request, result),
                Err(message) => error_response(request, message),
            },
            PluginRequestKind::GetInfo => success_response(&request, json!(self.manifest())),
        }
    }
}

fn error_response(request: PluginRequest, message: String) -> PluginResponse {
    PluginResponse {
        request_id: request.request_id,
        plugin_id: request.plugin_id,
        ok: false,
        result: Value::Null,
        error: Some(PluginError {
            code: ErrorCode::Internal,
            message,
        }),
    }
}

fn load(context: &PluginContext) -> TimeState {
    let mut state = context
        .storage()
        .get(context.plugin_id(), STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<TimeState>(&value).ok())
        .unwrap_or_default();

    state.alarms = state.alarms.into_iter().map(normalize_alarm).collect();
    state.clock_style = normalize_style(state.clock_style);
    state
}

fn normalize_style(mut style: ClockStyle) -> ClockStyle {
    style.background_opacity = if style.background_opacity.is_finite() {
        style.background_opacity.clamp(0.0, 1.0)
    } else {
        0.85
    };
    style.content_opacity = if style.content_opacity.is_finite() {
        style.content_opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    style.scale = if style.scale.is_finite() {
        style.scale.clamp(0.3, 2.0)
    } else {
        1.0
    };
    style.color = if style.color.is_empty() {
        "accent".into()
    } else {
        style.color
    };
    style.skin = match style.skin.as_str() {
        "pomodoro" => "pomodoro".into(),
        _ => "mini".into(),
    };
    style.mini_time_format = match style.mini_time_format.as_str() {
        "hms" => "hms".into(),
        _ => "hm".into(),
    };
    style.mini_layout = match style.mini_layout.as_str() {
        "vertical" => "vertical".into(),
        _ => "horizontal".into(),
    };
    style.font_weight = match style.font_weight {
        400 | 500 => style.font_weight,
        700 => 700,
        _ => 500,
    };
    style.pomo_work_min = style.pomo_work_min.clamp(5, 90);
    style.pomo_break_min = style.pomo_break_min.clamp(1, 30);
    style
}

fn save(context: &PluginContext, state: &TimeState) -> Result<(), String> {
    let value = serde_json::to_string(state).map_err(|e| e.to_string())?;
    context
        .storage()
        .set(context.plugin_id(), STATE_KEY, &value)
        .map_err(|e| e.message)
}

fn parse<T: for<'de> Deserialize<'de>>(payload: &Value, key: &str) -> Result<T, String> {
    payload
        .get(key)
        .cloned()
        .ok_or_else(|| format!("缺少字段: {key}"))
        .and_then(|value| {
            serde_json::from_value(value).map_err(|e| format!("字段 {key} 无效: {e}"))
        })
}

fn string_field(payload: &Value, name: &str) -> Result<String, String> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("缺少字段: {name}"))
}

fn validate(input: &AlarmInput) -> Result<(), String> {
    if input.label.chars().count() > 100 {
        return Err("标签过长".into());
    }
    match &input.rule {
        AlarmRule::Time {
            hour,
            minute,
            weekdays,
        } => {
            if *hour > 23 || *minute > 59 {
                return Err("时间无效".into());
            }
            if weekdays.iter().any(|day| *day > 6) {
                return Err("重复日期无效".into());
            }
        }
        AlarmRule::Interval {
            interval_minutes,
            next_fire_at_secs,
            message,
        } => {
            if !(1..=MAX_ALARM_INTERVAL_MINUTES).contains(interval_minutes) {
                return Err(format!("间隔需设置为 1 到 {MAX_ALARM_INTERVAL_MINUTES} 分钟"));
            }
            if input.enabled && *next_fire_at_secs <= Local::now().timestamp() {
                return Err("下一次提醒时间必须晚于当前时间".into());
            }
            if message.chars().count() > 100 {
                return Err("提醒内容不能超过 100 个字符".into());
            }
        }
    }
    Ok(())
}

fn normalize_alarm(alarm: Alarm) -> Alarm {
    alarm_from_input(
        alarm.id,
        AlarmInput {
            label: alarm.label,
            rule: alarm.rule,
            enabled: alarm.enabled,
        },
    )
}

fn alarm_from_input(id: String, input: AlarmInput) -> Alarm {
    let rule = normalize_alarm_rule(input.rule);
    Alarm {
        id,
        label: normalize_alarm_label(&input.label),
        rule,
        enabled: input.enabled,
    }
}

fn normalize_alarm_rule(rule: AlarmRule) -> AlarmRule {
    match rule {
        AlarmRule::Time {
            hour,
            minute,
            mut weekdays,
        } => {
            weekdays.retain(|day| *day <= 6);
            AlarmRule::Time {
                hour: hour.min(23),
                minute: minute.min(59),
                weekdays: normalize_weekdays(weekdays),
            }
        }
        AlarmRule::Interval {
            interval_minutes,
            next_fire_at_secs,
            message,
        } => {
            let message = message.trim().chars().take(100).collect::<String>();
            AlarmRule::Interval {
                interval_minutes: interval_minutes.clamp(1, MAX_ALARM_INTERVAL_MINUTES),
                next_fire_at_secs,
                message: if message.is_empty() {
                    DEFAULT_INTERVAL_MESSAGE.into()
                } else {
                    message
                },
            }
        }
    }
}

fn normalize_alarm_label(label: &str) -> String {
    let label = label.trim();
    if label.is_empty() {
        "闹钟".into()
    } else {
        label.into()
    }
}

fn normalize_weekdays(mut days: Vec<u8>) -> Vec<u8> {
    days.retain(|day| *day <= 6);
    days.sort_unstable();
    days.dedup();
    days
}

fn snapshot(context: &PluginContext) -> TimeSnapshot {
    let now = Local::now();
    let state = load(context);
    make_snapshot(
        now,
        state.alarms,
        state.notification_method,
        state.notification_duration_secs,
        state.alarm_clock_flash_enabled,
    )
}

fn make_snapshot(
    now: DateTime<Local>,
    alarms: Vec<Alarm>,
    notification_method: NotificationMethod,
    notification_duration_secs: u32,
    alarm_clock_flash_enabled: bool,
) -> TimeSnapshot {
    let next = alarms
        .iter()
        .filter_map(|alarm| {
            next_trigger_at(alarm, now).map(|at| (alarm.id.clone(), at.timestamp()))
        })
        .min_by_key(|(_, at)| *at)
        .map(|(alarm_id, at_secs)| NextAlarm { alarm_id, at_secs });
    TimeSnapshot {
        now_local: now.to_rfc3339(),
        hour: now.hour() as u8,
        minute: now.minute() as u8,
        second: now.second() as u8,
        weekday: now.weekday().num_days_from_sunday() as u8,
        alarms,
        next,
        notification_method,
        notification_duration_secs,
        alarm_clock_flash_enabled,
    }
}

fn emit_clock_snapshot(context: &PluginContext) {
    sync_clock_snapshot(context.services(), &snapshot(context));
}

fn sync_clock_snapshot(services: &dyn HostServices, snapshot: &TimeSnapshot) {
    #[cfg(windows)]
    let _ = services;
    #[cfg(windows)]
    native_clock::update_snapshot(CLOCK_WINDOW, snapshot);
    #[cfg(not(windows))]
    let _ = services
        .windows()
        .emit(CLOCK_WINDOW, "time://clock-snapshot", json!(snapshot));
}

fn sync_clock_style(services: &dyn HostServices, style: &ClockStyle) {
    #[cfg(windows)]
    let _ = services;
    #[cfg(windows)]
    native_clock::update_style(CLOCK_WINDOW, style);
    #[cfg(not(windows))]
    let _ = services
        .windows()
        .emit(CLOCK_WINDOW, "time://clock-style", json!(style));
}

fn sync_clock_visibility(services: &dyn HostServices, visible: bool) {
    #[cfg(not(windows))]
    let _ = services
        .windows()
        .emit(CLOCK_WINDOW, "time://clock-visibility", json!(visible));
    #[cfg(windows)]
    let _ = (services, visible);
}

fn sync_clickthrough(services: &dyn HostServices, enabled: bool) {
    #[cfg(not(windows))]
    let _ = services
        .windows()
        .emit(CLOCK_WINDOW, "time://clickthrough", json!(enabled));
    #[cfg(windows)]
    let _ = (services, enabled);
}

fn sync_clock_alarm(services: &dyn HostServices, payload: Value, flash_enabled: bool) {
    #[cfg(windows)]
    if flash_enabled {
        native_clock::alarm(CLOCK_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let mut payload = payload;
        if let Value::Object(fields) = &mut payload {
            fields.insert("flash".into(), json!(flash_enabled));
        }
        let _ = services
            .windows()
            .emit(CLOCK_WINDOW, "time://alarm", payload);
    }
    #[cfg(windows)]
    let _ = (services, payload, flash_enabled);
}

fn next_trigger_at<T: TimeZone>(alarm: &Alarm, now: DateTime<T>) -> Option<DateTime<T>> {
    if !alarm.enabled {
        return None;
    }
    match &alarm.rule {
        AlarmRule::Time {
            hour,
            minute,
            weekdays,
        } => {
            let target = NaiveTime::from_hms_opt(*hour as u32, *minute as u32, 0)?;
            for offset in 0..=7i64 {
                let date = now.date_naive() + ChronoDuration::days(offset);
                if !weekdays.is_empty()
                    && !weekdays.contains(&(date.weekday().num_days_from_sunday() as u8))
                {
                    continue;
                }
                let naive = date.and_time(target);
                let Some(at) = now.timezone().from_local_datetime(&naive).earliest() else {
                    continue;
                };
                if at >= now {
                    return Some(at);
                }
            }
            None
        }
        AlarmRule::Interval {
            next_fire_at_secs,
            ..
        } => now.timezone().timestamp_opt(*next_fire_at_secs, 0).single(),
    }
}

fn alarm_due_key(alarm: &Alarm, now: DateTime<Local>) -> Option<String> {
    if !alarm.enabled {
        return None;
    }
    match &alarm.rule {
        AlarmRule::Time {
            hour,
            minute,
            weekdays,
        } if *hour as u32 == now.hour()
            && *minute as u32 == now.minute()
            && (weekdays.is_empty()
                || weekdays.contains(&(now.weekday().num_days_from_sunday() as u8))) =>
        {
            Some(format!("time|{}", now.format("%Y%m%d-%H%M")))
        }
        AlarmRule::Interval {
            next_fire_at_secs,
            ..
        } if *next_fire_at_secs <= now.timestamp() => {
            Some(format!("interval|{next_fire_at_secs}"))
        }
        _ => None,
    }
}

fn tick(
    services: &Arc<dyn HostServices>,
    plugin_id: &sideweave_plugin_api::PluginId,
    state_lock: &Arc<Mutex<()>>,
    alarms: &Arc<Mutex<Vec<Alarm>>>,
    last_fired: &Arc<Mutex<HashMap<String, String>>>,
) {
    let now = Local::now();
    if matches!(now.second(), 0 | 30) {
        let current_alarms = alarms
            .lock()
            .map(|cached| cached.clone())
            .unwrap_or_default();
        sync_clock_snapshot(
            services.as_ref(),
            &make_snapshot(
                now,
                current_alarms,
                notification_method(services.as_ref(), plugin_id),
                notification_duration_secs(services.as_ref(), plugin_id),
                alarm_clock_flash_enabled(services.as_ref(), plugin_id),
            ),
        );
    }
    let due = {
        let _guard = state_lock.lock().unwrap();
        alarms
            .lock()
            .map(|cached| {
                cached
                    .iter()
                    .filter_map(|alarm| {
                        alarm_due_key(alarm, now).map(|key| (alarm.clone(), key))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let mut fired_now = false;
    for (alarm, occurrence_key) in due {
        if last_fired
            .lock()
            .map(|fired| fired.get(&alarm.id) == Some(&occurrence_key))
            .unwrap_or(true)
        {
            continue;
        }

        let Some(alarm) = record_alarm_occurrence(
            services.as_ref(),
            plugin_id,
            state_lock,
            alarms,
            &alarm.id,
            &occurrence_key,
            now,
        ) else {
            continue;
        };
        if let Ok(mut fired) = last_fired.lock() {
            fired.insert(alarm.id.clone(), occurrence_key);
        }
        fired_now = true;
        let (title, body) = alarm_notification_content(&alarm, now);
        let notification = DesktopNotificationOptions::new(title, body)
            .with_kind(DesktopNotificationKind::Alarm);
        if let Err(error) = notify_time(services.as_ref(), plugin_id, &notification) {
            eprintln!(
                "[time] failed to show alarm notification: {}",
                error.message
            );
        }
        let payload = json!({ "id": alarm.id, "label": alarm.label });
        let _ = services.events().publish(PluginEvent {
            plugin_id: plugin_id.clone(),
            topic: "time.alarm".into(),
            payload: payload.clone(),
        });
        sync_clock_alarm(
            services.as_ref(),
            payload,
            alarm_clock_flash_enabled(services.as_ref(), plugin_id),
        );
    }
    if fired_now {
        let current_alarms = alarms
            .lock()
            .map(|cached| cached.clone())
            .unwrap_or_default();
        sync_clock_snapshot(
            services.as_ref(),
            &make_snapshot(
                now,
                current_alarms,
                notification_method(services.as_ref(), plugin_id),
                notification_duration_secs(services.as_ref(), plugin_id),
                alarm_clock_flash_enabled(services.as_ref(), plugin_id),
            ),
        );
    }
}

fn record_alarm_occurrence(
    services: &dyn HostServices,
    plugin_id: &sideweave_plugin_api::PluginId,
    state_lock: &Mutex<()>,
    alarms: &Mutex<Vec<Alarm>>,
    alarm_id: &str,
    occurrence_key: &str,
    now: DateTime<Local>,
) -> Option<Alarm> {
    let _guard = state_lock.lock().ok()?;
    let value = services
        .storage()
        .get(plugin_id, STATE_KEY)
        .ok()
        .flatten()?;
    let mut state = serde_json::from_str::<TimeState>(&value).ok()?;
    let alarm = state
        .alarms
        .iter_mut()
        .find(|alarm| alarm.id == alarm_id)?;
    *alarm = normalize_alarm(alarm.clone());
    if alarm_due_key(alarm, now).as_deref() != Some(occurrence_key) {
        return None;
    }

    match &mut alarm.rule {
        AlarmRule::Time { weekdays, .. } if weekdays.is_empty() => alarm.enabled = false,
        AlarmRule::Interval {
            interval_minutes,
            next_fire_at_secs,
            ..
        } => {
            let interval_secs = i64::from((*interval_minutes).max(1)) * 60;
            let elapsed = now.timestamp().saturating_sub(*next_fire_at_secs);
            let steps = elapsed.div_euclid(interval_secs).saturating_add(1);
            *next_fire_at_secs =
                (*next_fire_at_secs).saturating_add(steps.saturating_mul(interval_secs));
        }
        _ => {}
    }

    let result = alarm.clone();
    if let Ok(serialized) = serde_json::to_string(&state) {
        if let Err(error) = services.storage().set(plugin_id, STATE_KEY, &serialized) {
            eprintln!("[time] failed to save next alarm occurrence: {}", error.message);
        }
    }
    if let Ok(mut cached) = alarms.lock() {
        *cached = state.alarms;
    }
    Some(result)
}

fn notification_method(
    services: &dyn HostServices,
    plugin_id: &sideweave_plugin_api::PluginId,
) -> NotificationMethod {
    services
        .storage()
        .get(plugin_id, STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<TimeState>(&value).ok())
        .map(|state| state.notification_method)
        .unwrap_or_default()
}

fn notification_duration_secs(
    services: &dyn HostServices,
    plugin_id: &sideweave_plugin_api::PluginId,
) -> u32 {
    services
        .storage()
        .get(plugin_id, STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<TimeState>(&value).ok())
        .map(|state| {
            state
                .notification_duration_secs
                .clamp(MIN_NOTIFICATION_DURATION_SECS, MAX_NOTIFICATION_DURATION_SECS)
        })
        .unwrap_or(DEFAULT_NOTIFICATION_DURATION_SECS)
}

fn alarm_clock_flash_enabled(
    services: &dyn HostServices,
    plugin_id: &sideweave_plugin_api::PluginId,
) -> bool {
    services
        .storage()
        .get(plugin_id, STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<TimeState>(&value).ok())
        .map(|state| state.alarm_clock_flash_enabled)
        .unwrap_or(true)
}

fn notify_time(
    services: &dyn HostServices,
    plugin_id: &sideweave_plugin_api::PluginId,
    notification: &DesktopNotificationOptions,
) -> Result<(), sideweave_plugin_sdk::HostError> {
    let duration_ms = u64::from(notification_duration_secs(services, plugin_id)) * 1_000;
    let notification = notification.clone().with_duration_ms(duration_ms);
    match notification_method(services, plugin_id) {
        NotificationMethod::Native => services
            .notification()
            .notify(&notification.title, &notification.body),
        NotificationMethod::Desktop => services
            .notification()
            .show_desktop_for_plugin(plugin_id, &notification),
        NotificationMethod::Sidebar => services
            .notification()
            .show_sidebar_for_plugin(plugin_id, &notification),
    }
}

fn alarm_notification_content(alarm: &Alarm, now: DateTime<Local>) -> (String, String) {
    match &alarm.rule {
        AlarmRule::Interval { message, .. } => (
            alarm.label.clone(),
            message.replace("{time}", &now.format("%H:%M").to_string()),
        ),
        AlarmRule::Time { hour, minute, .. } => {
            let time = format!("{hour:02}:{minute:02}");
            let label = alarm.label.trim();
            let body = if label.is_empty() || label == "闹钟" {
                format!("{time} 的闹钟时间到了。")
            } else {
                format!("{label}（{time}）时间到了。")
            };
            ("闹钟提醒".into(), body)
        }
    }
}

fn toggle_clock(context: &PluginContext, state_lock: &Arc<Mutex<()>>) -> Result<Value, String> {
    let (visible, style) = {
        let _guard = state_lock.lock().unwrap();
        let mut state = load(context);
        state.clock_style.clock_visible = !state.clock_style.clock_visible;
        let visible = state.clock_style.clock_visible;
        if !visible {
            state.clock_style.pinned = false;
        }
        save(context, &state)?;
        (visible, state.clock_style)
    };
    if visible {
        create_clock(context, &style, state_lock)?;
        let _ = context
            .windows()
            .set_clickthrough(CLOCK_WINDOW, style.pinned);
        sync_clock_visibility(context.services(), true);
        sync_clock_style(context.services(), &style);
        emit_clock_snapshot(context);
        sync_clickthrough(context.services(), style.pinned);
    } else {
        let _ = context.windows().set_clickthrough(CLOCK_WINDOW, false);
        sync_clock_visibility(context.services(), false);
        sync_clickthrough(context.services(), false);
        context
            .windows()
            .close(CLOCK_WINDOW)
            .map_err(|e| e.message)?;
    }
    Ok(Value::Null)
}

fn set_pinned(
    context: &PluginContext,
    request: &PluginRequest,
    state_lock: &Arc<Mutex<()>>,
) -> Result<Value, String> {
    let enabled = request
        .payload
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let style = {
        let _guard = state_lock.lock().unwrap();
        let mut state = load(context);
        state.clock_style.pinned = enabled;
        save(context, &state)?;
        state.clock_style
    };
    context
        .windows()
        .set_clickthrough(CLOCK_WINDOW, enabled)
        .map_err(|e| e.message)?;
    sync_clickthrough(context.services(), enabled);
    sync_clock_style(context.services(), &style);
    publish_clock_style(context, &style);
    Ok(Value::Null)
}

fn restore_clock(context: &PluginContext, state_lock: &Arc<Mutex<()>>) -> Result<(), String> {
    let style = {
        let _guard = state_lock.lock().unwrap();
        load(context).clock_style
    };
    if style.clock_visible {
        create_clock(context, &style, state_lock)?;
        let _ = context
            .windows()
            .set_clickthrough(CLOCK_WINDOW, style.pinned);
        sync_clock_visibility(context.services(), true);
        sync_clock_style(context.services(), &style);
        emit_clock_snapshot(context);
        sync_clickthrough(context.services(), style.pinned);
    }
    Ok(())
}

fn create_clock(
    context: &PluginContext,
    style: &ClockStyle,
    state_lock: &Arc<Mutex<()>>,
) -> Result<(), String> {
    let (width, height) = clock_size(style);
    let initial_snapshot = snapshot(context);
    let handler = ClockWindowHandler {
        services: context.services_arc(),
        plugin_id: context.plugin_id().clone(),
        state_lock: Arc::clone(state_lock),
        style: style.clone(),
        initial_snapshot: initial_snapshot.clone(),
    };
    context
        .windows()
        .create(
            WindowSpec {
                id: CLOCK_WINDOW.into(),
                title: "SideWeave 时钟".into(),
                url: clock_window_url_with_snapshot(style, Some(&initial_snapshot)),
                width,
                height,
                position_px: style.pos_x.zip(style.pos_y),
                always_on_top: true,
                skip_taskbar: true,
                resizable: false,
                transparent: !cfg!(windows),
                decorations: false,
                maximizable: false,
            },
            Some(Arc::new(handler)),
        )
        .map_err(|e| e.message)
}

#[cfg(test)]
fn clock_window_url(style: &ClockStyle) -> String {
    clock_window_url_with_snapshot(style, None)
}

fn clock_window_url_with_snapshot(style: &ClockStyle, snapshot: Option<&TimeSnapshot>) -> String {
    #[cfg(windows)]
    {
        let _ = (style, snapshot);
        return "agile-native://localhost/time/clock".into();
    }
    #[cfg(not(windows))]
    {
        let encode = |serialized: &str| {
            let mut encoded = String::with_capacity(serialized.len() * 3);
            for byte in serialized.bytes() {
                if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                    encoded.push(char::from(byte));
                } else {
                    encoded.push_str(&format!("%{byte:02X}"));
                }
            }
            encoded
        };
        let serialized_style = serde_json::to_string(style).unwrap_or_else(|_| "{}".into());
        let mut query = format!("style={}", encode(&serialized_style));
        if let Some(snapshot) = snapshot {
            let serialized_snapshot =
                serde_json::to_string(snapshot).unwrap_or_else(|_| "{}".into());
            query.push_str("&snapshot=");
            query.push_str(&encode(&serialized_snapshot));
        }
        format!("ui/clock.html?{query}")
    }
}

fn save_clock_style(
    context: &PluginContext,
    mut style: ClockStyle,
    state_lock: &Arc<Mutex<()>>,
) -> Result<Value, String> {
    let style = {
        let _guard = state_lock.lock().unwrap();
        let old = load(context).clock_style;
        style = normalize_style(style);
        style.pos_x = old.pos_x;
        style.pos_y = old.pos_y;
        style.clock_visible = old.clock_visible;
        style.pinned = old.pinned;
        let mut state = load(context);
        state.clock_style = style.clone();
        save(context, &state)?;
        style
    };
    sync_clock_style(context.services(), &style);
    publish_clock_style(context, &style);
    let (width, height) = clock_size(&style);
    let _ = context.windows().set_size(CLOCK_WINDOW, width, height);
    Ok(Value::Null)
}

fn publish_clock_style(context: &PluginContext, style: &ClockStyle) {
    let _ = context.events().publish(PluginEvent {
        plugin_id: context.plugin_id().clone(),
        topic: "time.clock_style".into(),
        payload: json!(style),
    });
}

fn clock_size(style: &ClockStyle) -> (f64, f64) {
    let scale = style.scale.clamp(0.3, 2.0);
    if style.skin == "mini" {
        let (width, height) = match (style.mini_layout.as_str(), style.mini_time_format.as_str()) {
            ("vertical", "hms") => (100.0, 188.0),
            ("vertical", _) => (100.0, 122.0),
            (_, "hms") => (264.0, 72.0),
            _ => (200.0, 72.0),
        };
        return (width * scale, height * scale);
    }
    if style.skin == "pomodoro" {
        return ((340.0 * scale).max(180.0), (150.0 * scale).max(100.0));
    }
    (340.0 * scale, 150.0 * scale)
}

struct ClockWindowHandler {
    services: Arc<dyn HostServices>,
    plugin_id: sideweave_plugin_api::PluginId,
    state_lock: Arc<Mutex<()>>,
    style: ClockStyle,
    initial_snapshot: TimeSnapshot,
}

impl WindowEventHandler for ClockWindowHandler {
    fn on_event(&self, event: WindowEvent) {
        match event {
            WindowEvent::NativeHandleAvailable { handle } => {
                #[cfg(windows)]
                {
                    let pomo_services = Arc::clone(&self.services);
                    let pomo_plugin_id = self.plugin_id.clone();
                    let callbacks = native_clock::NativeClockCallbacks {
                        pomo_finished: Arc::new(move |phase| {
                            let message = if phase == "work" {
                                "\u{4e13}\u{6ce8}\u{65f6}\u{95f4}\u{7ed3}\u{675f}\u{ff0c}\u{5f00}\u{59cb}\u{4f11}\u{606f}\u{5427}\u{3002}"
                            } else {
                                "\u{4f11}\u{606f}\u{65f6}\u{95f4}\u{7ed3}\u{675f}\u{ff0c}\u{5f00}\u{59cb}\u{4e0b}\u{4e00}\u{8f6e}\u{4e13}\u{6ce8}\u{5427}\u{3002}"
                            };
                            let _ = notify_time(
                                pomo_services.as_ref(),
                                &pomo_plugin_id,
                                &DesktopNotificationOptions::new(
                                    "\u{756a}\u{8304}\u{949f}",
                                    message,
                                )
                                .with_kind(DesktopNotificationKind::Alarm),
                            );
                        }),
                    };
                    let _ = native_clock::attach(
                        CLOCK_WINDOW,
                        handle,
                        &self.style,
                        &self.initial_snapshot,
                        callbacks,
                    );
                }
                return;
            }
            WindowEvent::VisibilityChanged { visible } => {
                #[cfg(windows)]
                native_clock::set_visible(CLOCK_WINDOW, visible);
                return;
            }
            _ => {}
        }
        let _guard = self.state_lock.lock().unwrap();
        let Some(value) = self
            .services
            .storage()
            .get(&self.plugin_id, STATE_KEY)
            .ok()
            .flatten()
        else {
            return;
        };
        let Ok(mut state) = serde_json::from_str::<TimeState>(&value) else {
            return;
        };
        match event {
            WindowEvent::Moved { x, y } => {
                state.clock_style.pos_x = Some(x);
                state.clock_style.pos_y = Some(y);
            }
            WindowEvent::Closed => {
                state.clock_style.clock_visible = false;
            }
            WindowEvent::Resized { .. }
            | WindowEvent::NativeHandleAvailable { .. }
            | WindowEvent::VisibilityChanged { .. } => return,
        }
        if let Ok(serialized) = serde_json::to_string(&state) {
            let _ = self
                .services
                .storage()
                .set(&self.plugin_id, STATE_KEY, &serialized);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{clock_window_url, normalize_style, ClockStyle};

    #[test]
    fn default_clock_style_has_renderable_values() {
        let style = ClockStyle::default();
        assert_eq!(style.skin, "mini");
        assert_eq!(style.scale, 1.0);
        assert_eq!(style.pomo_work_min, 25);
        assert_eq!(style.pomo_break_min, 5);
        assert_eq!(style.font_weight, 500);
    }

    #[test]
    fn invalid_style_values_are_normalized() {
        let style = normalize_style(ClockStyle {
            background_opacity: 0.0,
            content_opacity: 1.0,
            scale: 0.0,
            color: String::new(),
            pos_x: None,
            pos_y: None,
            clock_visible: true,
            skin: String::new(),
            mini_time_format: String::new(),
            mini_layout: String::new(),
            font_weight: 900,
            pomo_work_min: 0,
            pomo_break_min: 0,
            pinned: true,
        });
        assert_eq!(style.skin, "mini");
        assert_eq!(style.mini_time_format, "hm");
        assert_eq!(style.mini_layout, "horizontal");
        assert_eq!(style.scale, 0.3);
        assert_eq!(style.background_opacity, 0.0);
        assert_eq!(style.font_weight, 500);
        assert!(style.clock_visible);
        assert!(style.pinned);
    }

    #[test]
    fn current_saved_style_uses_expected_values() {
        let style: ClockStyle = serde_json::from_str(
            r#"{"background_opacity":0.85,"content_opacity":1.0,"scale":1.0,"color":"accent","skin":"mini","mini_time_format":"hm","mini_layout":"horizontal","font_weight":500,"pomo_work_min":25,"pomo_break_min":5,"pinned":false}"#,
        )
        .unwrap();
        assert_eq!(style.font_weight, 500);
        assert_eq!(style.background_opacity, 0.85);
    }

    #[test]
    fn clock_window_url_embeds_the_saved_skin() {
        let style = ClockStyle {
            skin: "mini".into(),
            mini_time_format: "hms".into(),
            mini_layout: "vertical".into(),
            ..ClockStyle::default()
        };

        let url = clock_window_url(&style);

        #[cfg(windows)]
        assert_eq!(url, "agile-native://localhost/time/clock");
        #[cfg(not(windows))]
        {
            assert!(url.starts_with("ui/clock.html?style="));
            assert!(url.contains("%22skin%22%3A%22mini%22"));
            assert!(url.contains("%22mini_time_format%22%3A%22hms%22"));
            assert!(url.contains("%22mini_layout%22%3A%22vertical%22"));
        }
    }
}
