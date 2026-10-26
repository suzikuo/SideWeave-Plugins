use std::panic::{catch_unwind, AssertUnwindSafe};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;
use std::sync::OnceLock;

mod data_sync;
mod account_groups;
mod quota_estimates;
mod quota_refresh;
mod quota_resets;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginManifest, PluginRequest, PluginRequestKind, PluginResponse,
    PluginRuntime, PluginTrust, PluginUi, PluginUiKind, API_VERSION, MANIFEST_VERSION,
};
use sideweave_plugin_sdk::{
    success_response, DesktopNotificationKind, DesktopNotificationOptions, Plugin, PluginContext,
};
use usage_core::{
    budget::load_budgets, summarize_facts, AggregatePlan, DateFilter, MachineScope, PricingMap,
    QuotaOrigin, QuotaView, Report, ReportOptions, TokenCounts,
};
use usage_index::{retention_cutoff, AccountFilter, Index, IngestOptions};

const PLUGIN_ID: &str = "ai_usage";
const DEFAULT_REQUEST_LIMIT: usize = 120;
const MAX_REQUEST_LIMIT: usize = 300;
const MAX_SESSION_LIMIT: usize = 500;
const PREVIEW_SETTINGS_KEY: &str = "ai_usage_preview_settings";
const QUOTA_NOTIFICATION_SETTINGS_KEY: &str = "ai_usage_quota_notification_settings";
const QUOTA_NOTIFICATION_LEDGER_KEY: &str = "ai_usage_quota_notification_ledger";
const DATA_ROOTS_KEY: &str = "ai_usage_data_roots";
const ACCOUNT_NAMES_FILE: &str = "codex_account_names.json";
const QUOTA_NOTIFICATION_DAY_MS: i64 = 24 * 60 * 60 * 1_000;
const QUOTA_RESET_DRIFT_TOLERANCE_MS: i64 = 15 * 60 * 1_000;
static PRICING_REFRESH_ACTIVE: AtomicBool = AtomicBool::new(false);
static PRICING_RETRY_AFTER_MS: AtomicI64 = AtomicI64::new(0);
static DATA_SYNC: OnceLock<data_sync::DataSync> = OnceLock::new();
const SOURCE_STATUS_KEY: &str = "ai_usage_source_status";

sideweave_plugin_sdk::export_plugin_dylib!(AiUsagePlugin);

pub struct AiUsagePlugin {
    manifest: PluginManifest,
    reset_worker: Option<quota_resets::Worker>,
}

impl Default for AiUsagePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl AiUsagePlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                manifest_version: MANIFEST_VERSION,
                api_version: API_VERSION,
                id: sideweave_plugin_api::PluginId::new(PLUGIN_ID).expect("static plugin id"),
                name: "AI 用量".into(),
                icon: "ui/icon.svg".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "本机 AI 编程工具的 Token、估算费用、会话与配额仪表盘".into(),
                author: "SideWeave".into(),
                runtime: PluginRuntime::DynamicLibrary,
                backend: Some("plugin.dll".into()),
                trust: PluginTrust::TrustedLocal,
                permissions: vec![
                    "filesystem.read".into(),
                    "filesystem.write".into(),
                    "network.connect".into(),
                    "notification.desktop".into(),
                ],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    kind: Some(PluginUiKind::Html),
                    asset_root: "ui".into(),
                    initial_width: Some(1040),
                    initial_height: Some(780),
                    ..Default::default()
                }),
                hover_preview: Some(sideweave_plugin_api::PluginHoverPreview {
                    width: Some(360),
                    height: Some(260),
                    data_method: None,
                    entry: Some("ui/preview.html".into()),
                }),
                auto_start: true,
                minimum_host_version: None,
            },
            reset_worker: None,
        }
    }

    fn call(&self, request: &PluginRequest, context: &PluginContext) -> Result<Value, String> {
        match request.method.as_deref() {
            Some("dashboard") => dashboard(&request.payload, context),
            Some("sync_data") => Ok(json!(start_data_sync(request.payload.get("force").and_then(Value::as_bool).unwrap_or(false)))),
            Some("get_sync_status") => Ok(json!(data_sync().status())),
            Some("get_quotas") => quota_data(&request.payload, context),
            Some("requests") => request_rows(&request.payload),
            Some("get_account_groups") => account_groups::get(&open_index()?),
            Some("save_account_group") => account_groups::save(&request.payload),
            Some("delete_account_group") => account_groups::delete(&request.payload),
            Some("sessions") => session_rows(&request.payload),
            Some("set_data_roots") => set_data_roots(&request.payload),
            Some("set_account_name") | Some("set_codex_account_name") => set_account_name(&request.payload),
            Some("get_preview") => preview_data(context),
            Some("get_preview_settings") => get_preview_settings(),
            Some("set_preview_settings") => set_preview_settings(&request.payload),
            Some("get_quota_notification_settings") => get_quota_notification_settings(),
            Some("set_quota_notification_settings") => set_quota_notification_settings(&request.payload),
            Some("check_quota_notifications") => check_quota_notifications(context),
            Some(method) => Err(format!("未知的 AI 用量方法：{method}")),
            None => Err("缺少插件方法".into()),
        }
    }
}

#[derive(Serialize)]
struct PreviewTool {
    tool: String,
    total_tokens: f64,
    requests: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct PreviewSettings {
    widgets: Vec<String>,
    display_mode: String,
    #[serde(default)]
    schema_version: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct QuotaNotificationSettings {
    // Retained for migrating the original all-subscriptions setting.
    enabled: bool,
    #[serde(
        alias = "remaining_threshold_percent",
        deserialize_with = "deserialize_quota_thresholds"
    )]
    remaining_thresholds_percent: Vec<u8>,
    subscriptions: HashMap<String, QuotaSubscriptionNotificationRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct QuotaSubscriptionNotificationRule {
    enabled: bool,
    reset_enabled: bool,
    remaining_thresholds_percent: Vec<u8>,
}

impl Default for QuotaSubscriptionNotificationRule {
    fn default() -> Self {
        Self {
            enabled: false,
            reset_enabled: false,
            remaining_thresholds_percent: vec![50, 10, 0],
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum QuotaThresholdValues {
    Single(u8),
    Multiple(Vec<u8>),
}

fn deserialize_quota_thresholds<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = QuotaThresholdValues::deserialize(deserializer)?;
    Ok(match values {
        QuotaThresholdValues::Single(value) => vec![value],
        QuotaThresholdValues::Multiple(values) => values,
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct QuotaNotificationLedger {
    windows: HashMap<String, QuotaNotificationWindowState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct QuotaNotificationWindowState {
    period_anchor_ms: i64,
    expires_at_ms: i64,
    #[serde(
        default,
        alias = "alerted",
        deserialize_with = "deserialize_alerted_thresholds"
    )]
    alerted_thresholds_percent: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AlertedThresholdValues {
    Legacy(bool),
    Current(Vec<u8>),
}

fn deserialize_alerted_thresholds<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = AlertedThresholdValues::deserialize(deserializer)?;
    Ok(match values {
        AlertedThresholdValues::Legacy(true) => (0..=100).collect(),
        AlertedThresholdValues::Legacy(false) => Vec::new(),
        AlertedThresholdValues::Current(values) => values,
    })
}

impl Default for QuotaNotificationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            remaining_thresholds_percent: vec![50, 10, 0],
            subscriptions: HashMap::new(),
        }
    }
}

fn read_quota_notification_settings(index: &Index) -> QuotaNotificationSettings {
    index
        .meta_value(QUOTA_NOTIFICATION_SETTINGS_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn get_quota_notification_settings() -> Result<Value, String> {
    let index = open_index()?;
    let settings = read_quota_notification_settings(&index);
    let quotas = usage_quota::cached();
    let subscriptions = quotas
        .iter()
        .filter(|quota| quota.origin == QuotaOrigin::Probe)
        .map(|quota| json!({ "key": quota_notification_identity(quota), "quota": quota }))
        .collect::<Vec<_>>();
    let mut value = serde_json::to_value(settings)
        .map_err(|error| format!("Could not read quota notification settings: {error}"))?;
    value["available_subscriptions"] = json!(subscriptions);
    Ok(value)
}

fn set_quota_notification_settings(payload: &Value) -> Result<Value, String> {
    let mut settings: QuotaNotificationSettings = serde_json::from_value(payload.clone())
        .map_err(|error| format!("Invalid quota notification settings: {error}"))?;
    if settings.remaining_thresholds_percent.iter().any(|threshold| *threshold > 100) {
        return Err("Legacy quota thresholds must be from 0 to 100 percent.".into());
    }
    settings.remaining_thresholds_percent.sort_unstable_by(|a, b| b.cmp(a));
    settings.remaining_thresholds_percent.dedup();
    for (identity, rule) in &mut settings.subscriptions {
        if rule.remaining_thresholds_percent.is_empty()
            || rule.remaining_thresholds_percent.len() > 10
            || rule.remaining_thresholds_percent.iter().any(|threshold| *threshold > 100)
        {
            return Err(format!(
                "Subscription {identity} needs between 1 and 10 thresholds from 0 to 100 percent."
            ));
        }
        rule.remaining_thresholds_percent.sort_unstable_by(|a, b| b.cmp(a));
        rule.remaining_thresholds_percent.dedup();
    }
    if payload.get("subscriptions").is_some() {
        settings.enabled = false;
    }
    let index = open_index()?;
    let serialized = serde_json::to_string(&settings)
        .map_err(|error| format!("Could not serialize quota notification settings: {error}"))?;
    index
        .conn()
        .execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES(?1, ?2)",
            params![QUOTA_NOTIFICATION_SETTINGS_KEY, serialized],
        )
        .map_err(|error| format!("Could not save quota notification settings: {error}"))?;
    serde_json::to_value(settings)
        .map_err(|error| format!("Could not return quota notification settings: {error}"))
}

fn quota_notification_identity(quota: &QuotaView) -> String {
    let bucket = quota
        .id
        .as_deref()
        .or(quota.label.as_deref())
        .unwrap_or("default");
    format!("{}|{}|{}", quota.tool, quota.window_minutes, bucket)
}

fn quota_notification_window(quota: &QuotaView) -> String {
    if let Some(label) = quota.label.as_deref().filter(|label| !label.trim().is_empty()) {
        return label.to_string();
    }
    if quota.window_minutes >= 10_080 {
        format!("{} \u{5468}\u{7a97}\u{53e3}", (quota.window_minutes + 10_079) / 10_080)
    } else if quota.window_minutes >= 1_440 {
        format!("{} \u{5929}\u{7a97}\u{53e3}", (quota.window_minutes + 1_439) / 1_440)
    } else {
        format!("{} \u{5c0f}\u{65f6}\u{7a97}\u{53e3}", (quota.window_minutes.max(60) + 59) / 60)
    }
}

fn notify_quota_threshold(
    index: &Index,
    quotas: &[QuotaView],
    context: &PluginContext,
) -> Result<usize, String> {
    // Detail, preview and the background watcher share one notification ledger.
    static CHECK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = CHECK_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let settings = read_quota_notification_settings(index);
    if !quotas.iter().any(|quota| quota.origin == QuotaOrigin::Probe) {
        return Ok(0);
    }

    let now = usage_core::report::now_ms();
    let stored = index
        .meta_value(QUOTA_NOTIFICATION_LEDGER_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<QuotaNotificationLedger>(&value).ok())
        .unwrap_or_default();
    let mut ledger = stored;
    ledger.windows.retain(|_, state| state.expires_at_ms > now);
    let mut alerts = Vec::new();

    for quota in quotas {
        if quota.origin != QuotaOrigin::Probe {
            continue;
        }
        let identity = quota_notification_identity(quota);
        let thresholds = match settings.subscriptions.get(&identity) {
            Some(rule) if rule.enabled => &rule.remaining_thresholds_percent,
            Some(_) => continue,
            None if settings.enabled => &settings.remaining_thresholds_percent,
            None => continue,
        };
        if thresholds.is_empty() {
            continue;
        }
        let remaining = (100.0 - quota.used_percent).clamp(0.0, 100.0);
        let (period_anchor_ms, expires_at_ms) = if quota.resets_at_ms > now {
            (quota.resets_at_ms, quota.resets_at_ms.saturating_add(60_000))
        } else {
            let day_start = now.div_euclid(QUOTA_NOTIFICATION_DAY_MS) * QUOTA_NOTIFICATION_DAY_MS;
            (day_start, day_start.saturating_add(QUOTA_NOTIFICATION_DAY_MS))
        };
        let state = ledger
            .windows
            .entry(identity)
            .or_default();
        let same_period = state.expires_at_ms > now
            && state.period_anchor_ms.abs_diff(period_anchor_ms)
                <= QUOTA_RESET_DRIFT_TOLERANCE_MS as u64;
        if !same_period {
            *state = QuotaNotificationWindowState {
                period_anchor_ms,
                expires_at_ms,
                alerted_thresholds_percent: Vec::new(),
            };
        } else {
            state.period_anchor_ms = period_anchor_ms;
            state.expires_at_ms = expires_at_ms;
        }

        let newly_reached = thresholds
            .iter()
            .copied()
            .filter(|threshold| {
                remaining <= f64::from(*threshold)
                    && !state.alerted_thresholds_percent.contains(threshold)
            })
            .collect::<Vec<_>>();
        if newly_reached.is_empty() {
            continue;
        }
        state
            .alerted_thresholds_percent
            .extend(newly_reached.iter().copied());
        let threshold_labels = newly_reached
            .iter()
            .map(|threshold| format!("{threshold}%"))
            .collect::<Vec<_>>()
            .join("\u{3001}");

        let tool_name = usage_adapter_all::builtin_adapters()
            .into_iter()
            .find(|adapter| adapter.id() == quota.tool.as_str())
            .map(|adapter| adapter.display_name().to_string())
            .unwrap_or_else(|| quota.tool.clone());
        alerts.push(format!(
            "{} \u{00b7} {}\u{ff1a}\u{5269}\u{4f59} {:.0}%\u{ff08}\u{9608}\u{503c} {}\u{ff09}",
            tool_name,
            quota_notification_window(quota),
            remaining,
            threshold_labels
        ));
    }

    if alerts.is_empty() {
        let serialized = serde_json::to_string(&ledger)
            .map_err(|error| format!("Could not serialize quota notification history: {error}"))?;
        index
            .conn()
            .execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES(?1, ?2)",
                params![QUOTA_NOTIFICATION_LEDGER_KEY, serialized],
            )
            .map_err(|error| format!("Could not save quota notification history: {error}"))?;
        return Ok(0);
    }

    let mut body = alerts.iter().take(4).cloned().collect::<Vec<_>>();
    if alerts.len() > body.len() {
        body.push(format!("\u{53e6}\u{6709} {} \u{4e2a}\u{989d}\u{5ea6}\u{7a97}\u{53e3}\u{8fbe}\u{5230}\u{9608}\u{503c}", alerts.len() - body.len()));
    }
    context
        .notification()
        .show_desktop(
            &DesktopNotificationOptions::new("AI \u{8ba2}\u{9605}\u{989d}\u{5ea6}\u{63d0}\u{9192}", body.join("\n"))
                .with_kind(DesktopNotificationKind::Warning)
                .with_duration_ms(8_000),
        )
        .map_err(|error| error.message)?;

    let serialized = serde_json::to_string(&ledger)
        .map_err(|error| format!("Could not serialize quota notification history: {error}"))?;
    index
        .conn()
        .execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES(?1, ?2)",
            params![QUOTA_NOTIFICATION_LEDGER_KEY, serialized],
        )
        .map_err(|error| format!("Could not save quota notification history: {error}"))?;
    Ok(1)
}

fn check_quota_notifications(context: &PluginContext) -> Result<Value, String> {
    let index = open_index()?;
    let quotas = usage_quota::cached();
    let notifications_sent = notify_quota_threshold(&index, &quotas, context)?
        + quota_resets::check(&index, context, &quotas)?;
    Ok(json!({ "notifications_sent": notifications_sent }))
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            widgets: vec![
                "tokens".into(),
                "cost".into(),
                "requests".into(),
                "quotas".into(),
                "tools".into(),
                "split".into(),
                "updated".into(),
            ],
            display_mode: "cards".into(),
            schema_version: 1,
        }
    }
}

fn read_preview_settings(index: &Index) -> PreviewSettings {
    let stored = index
        .meta_value(PREVIEW_SETTINGS_KEY)
        .ok()
        .flatten();
    let mut settings = stored
        .as_deref()
        .and_then(|value| serde_json::from_str::<PreviewSettings>(value).ok())
        .unwrap_or_default();
    let is_legacy = stored
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .and_then(|value| value.get("schema_version").and_then(Value::as_u64))
        .unwrap_or(0)
        == 0;
    if is_legacy && !settings.widgets.iter().any(|widget| widget == "quotas") {
        settings.widgets.insert(0, "quotas".into());
    }
    settings.schema_version = 1;
    settings
}

fn get_preview_settings() -> Result<Value, String> {
    let index = open_index()?;
    serde_json::to_value(read_preview_settings(&index))
        .map_err(|error| format!("无法读取预览设置：{error}"))
}

fn set_preview_settings(payload: &Value) -> Result<Value, String> {
    const ALLOWED_WIDGETS: &[&str] = &["tokens", "cost", "requests", "quotas", "tools", "split", "updated"];
    let index = open_index()?;
    let mut settings: PreviewSettings = serde_json::from_value(payload.clone())
        .map_err(|error| format!("预览设置格式无效：{error}"))?;
    let mut seen = HashSet::new();
    settings.widgets.retain(|widget| {
        ALLOWED_WIDGETS.contains(&widget.as_str()) && seen.insert(widget.clone())
    });
    settings.display_mode = match settings.display_mode.as_str() {
        "compact" => "compact".into(),
        _ => "cards".into(),
    };
    settings.schema_version = 1;
    let serialized = serde_json::to_string(&settings)
        .map_err(|error| format!("无法保存预览设置：{error}"))?;
    index
        .conn()
        .execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES(?1, ?2)",
            params![PREVIEW_SETTINGS_KEY, serialized],
        )
        .map_err(|error| format!("无法保存预览设置：{error}"))?;
    serde_json::to_value(settings).map_err(|error| format!("无法返回预览设置：{error}"))
}

fn read_account_names() -> HashMap<String, String> {
    std::fs::read(plugin_data_dir().join(ACCOUNT_NAMES_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
fn set_account_name(payload: &Value) -> Result<Value, String> {
    let selection = payload.get("selection").and_then(Value::as_str).unwrap_or("");
    let filter = AccountFilter::from_selection(selection);
    if !matches!(&filter, AccountFilter::Account { .. }) {
        return Err("Please select a tagged account or profile.".into());
    }
    let index = open_index()?;
    let known = index
        .accounts()
        .map_err(|error| format!("Could not read account list: {error}"))?
        .iter()
        .any(|account| make_account_selection(&account.tool, account.account_id.as_deref()) == selection);
    if !known {
        return Err("That account or profile is no longer indexed. Refresh and try again.".into());
    }

    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(|character| !character.is_control())
        .take(48)
        .collect::<String>()
        .trim()
        .to_string();
    let mut names = read_account_names();
    let saved_name = if name.is_empty() {
        names.remove(selection);
        if let AccountFilter::Account { tool, id } = &filter {
            if tool == "codex" { names.remove(id); }
        }
        None
    } else {
        names.insert(selection.to_string(), name.clone());
        Some(name)
    };

    let directory = plugin_data_dir();
    std::fs::create_dir_all(&directory).map_err(|error| format!("无法创建插件数据目录：{error}"))?;
    let path = directory.join(ACCOUNT_NAMES_FILE);
    let temporary = path.with_extension("json.tmp");
    let serialized = serde_json::to_vec_pretty(&names).map_err(|error| format!("无法序列化账号名称：{error}"))?;
    std::fs::write(&temporary, serialized).map_err(|error| format!("无法写入账号名称：{error}"))?;
    if let Err(error) = usage_core::replace_file(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("无法保存账号名称：{error}"));
    }
    serde_json::to_value(json!({ "name": saved_name }))
        .map_err(|error| format!("无法返回账号名称：{error}"))
}

fn preview_data(context: &PluginContext) -> Result<Value, String> {
    let index = open_index()?;
    let now = usage_core::report::now_ms();
    let since = now - 24 * 60 * 60 * 1000;
    let settings = read_preview_settings(&index);
    let polled_quotas = quotas_for_selection(&json!({}), false);
    let _ = notify_quota_threshold(&index, &polled_quotas, context);
    let last_ingest_ms = index
        .meta_value("last_ingest_ms")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<i64>().ok());

    let (requests, total_tokens, input_tokens, output_tokens, latest_ms):
        (i64, f64, f64, f64, Option<i64>) = index
        .conn()
        .query_row(
            "SELECT COUNT(*), \
                    COALESCE(SUM(COALESCE(in_tok, 0) + COALESCE(cc_tok, 0) + COALESCE(cr_tok, 0) + COALESCE(out_tok, 0)), 0), \
                    COALESCE(SUM(COALESCE(in_tok, 0) + COALESCE(cc_tok, 0) + COALESCE(cr_tok, 0)), 0), \
                    COALESCE(SUM(COALESCE(out_tok, 0)), 0), \
                    MAX(ts_ms) \
             FROM event WHERE ts_ms >= ?1 AND ts_ms <= ?2",
            params![since, now],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|error| format!("无法读取预览用量：{error}"))?;

    let mut statement = index
        .conn()
        .prepare(
            "SELECT tool, \
                    COALESCE(SUM(COALESCE(in_tok, 0) + COALESCE(cc_tok, 0) + COALESCE(cr_tok, 0) + COALESCE(out_tok, 0)), 0) AS total_tokens, \
                    COUNT(*) AS requests \
             FROM event WHERE ts_ms >= ?1 AND ts_ms <= ?2 \
             GROUP BY tool ORDER BY total_tokens DESC LIMIT 3",
        )
        .map_err(|error| format!("无法读取预览排行：{error}"))?;
    let tools = statement
        .query_map(params![since, now], |row| {
            Ok(PreviewTool {
                tool: row.get(0)?,
                total_tokens: row.get(1)?,
                requests: row.get(2)?,
            })
        })
        .map_err(|error| format!("无法读取预览排行：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法读取预览排行：{error}"))?;

    let pricing_options = usage_core::pricing::PricingOptions {
        offline: true,
        cache_dir: usage_core::pricing::PricingOptions::default_cache_dir(),
        overrides: Default::default(),
    };
    let prices = catch_unwind(AssertUnwindSafe(|| PricingMap::load(&pricing_options)))
        .unwrap_or_else(|_| PricingMap::default());
    let mut cost_usd = 0.0;
    let mut credit_cost_usd = 0.0;
    let mut unpriced_models = HashSet::new();
    let mut model_statement = index
        .conn()
        .prepare(
            "SELECT tool, model, COALESCE(SUM(in_tok), 0), COALESCE(SUM(cc_tok), 0), \
                    COALESCE(SUM(cr_tok), 0), COALESCE(SUM(out_tok), 0), COALESCE(SUM(credits), 0) \
             FROM event WHERE ts_ms >= ?1 AND ts_ms <= ?2 GROUP BY tool, model",
        )
        .map_err(|error| format!("无法读取预览费用：{error}"))?;
    let model_rows = model_statement
        .query_map(params![since, now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                TokenCounts {
                    input: row.get(2)?,
                    cache_creation: row.get(3)?,
                    cache_read: row.get(4)?,
                    output: row.get(5)?,
                    reasoning: 0.0,
                    credits: row.get(6)?,
                },
            ))
        })
        .map_err(|error| format!("无法读取预览费用：{error}"))?;
    for model_row in model_rows {
        let (tool, model, counts) = model_row.map_err(|error| format!("无法读取预览费用：{error}"))?;
        if let Some(cost) = prices.cost(model.as_deref(), &counts) {
            cost_usd += cost;
        } else if counts.total() > 0.0 {
            unpriced_models.insert(model.unwrap_or_else(|| "未知模型".into()));
        }
        if let Some((rate, _)) = usage_core::pricing::credit_rate(&tool) {
            credit_cost_usd += counts.credits * rate;
        }
    }

    let quotas = polled_quotas;
    let quota_estimates = quota_estimates::collect(&index, &quotas, "all", &prices);

    serde_json::to_value(json!({
        "generated_at_ms": now,
        "requests": requests,
        "total_tokens": total_tokens,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "latest_ms": latest_ms,
        "last_ingest_ms": last_ingest_ms,
        "sync_status": data_sync().status(),
        "tools": tools,
        "quotas": quotas,
        "quota_estimates": quota_estimates,
        "cost_usd": cost_usd,
        "credit_cost_usd": credit_cost_usd,
        "unpriced_models": unpriced_models.len(),
        "settings": settings,
    }))
    .map_err(|error| format!("无法序列化预览用量：{error}"))
}

impl Plugin for AiUsagePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
        let result = match &request.kind {
            PluginRequestKind::Activate => {
                if self.reset_worker.is_none() {
                    match quota_resets::Worker::start(context) {
                        Ok(worker) => { self.reset_worker = Some(worker); Ok(Value::Null) },
                        Err(error) => Err(error),
                    }
                } else { Ok(Value::Null) }
            }
            PluginRequestKind::Deactivate | PluginRequestKind::Shutdown => {
                self.reset_worker = None;
                Ok(Value::Null)
            }
            PluginRequestKind::Call => self.call(&request, context),
            _ => Ok(Value::Null),
        };
        match result {
            Ok(value) => success_response(&request, value),
            Err(message) => PluginResponse {
                request_id: request.request_id,
                plugin_id: request.plugin_id,
                ok: false,
                result: Value::Null,
                error: Some(PluginError {
                    code: ErrorCode::Internal,
                    message,
                }),
            },
        }
    }
}

#[derive(Serialize)]
struct DashboardPayload {
    report: Report,
    quota_estimates: Vec<quota_estimates::Estimate>,
    indexed_events: u64,
    last_ingest_ms: Option<i64>,
    refresh_warning: Option<String>,
    supported_tools: Vec<SupportedTool>,
    data_roots: Vec<DataRoot>,
    accounts: Vec<AccountOption>,
}

fn make_account_selection(tool: &str, account_id: Option<&str>) -> String {
    match account_id {
        Some(id) => format!("account:{tool}|{id}"),
        None => format!("unknown:{tool}"),
    }
}

#[derive(Serialize)]
struct AccountOption {
    tool: String,
    selection: String,
    label: String,
    name: Option<String>,
    requests: u64,
    sessions: u64,
}

#[derive(Serialize)]
struct SupportedTool {
    id: String,
    display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataRoot {
    id: String,
    path: String,
}

struct RootedAdapter {
    inner: Box<dyn usage_core::SourceAdapter>,
    root: DataRoot,
}

impl usage_core::SourceAdapter for RootedAdapter {
    fn id(&self) -> &'static str { self.inner.id() }
    fn display_name(&self) -> &'static str { self.inner.display_name() }
    fn semantics(&self) -> usage_core::Semantics { self.inner.semantics() }
    fn probe(&self) -> Option<usage_core::DetectedSource> {
        usage_core::with_discovery_home(std::path::Path::new(&self.root.path), || self.inner.probe())
    }
    fn discover(&self, filter: &DateFilter) -> Vec<usage_core::SourceFile> {
        self.discover_in_home(std::path::Path::new(&self.root.path), filter)
    }
    fn discover_in_home(&self, home: &std::path::Path, filter: &DateFilter) -> Vec<usage_core::SourceFile> {
        self.inner.discover_in_home(home, filter)
    }
    fn source_key(&self, file: &usage_core::SourceFile) -> String {
        format!("linux:{}:{}", self.root.id, file.key())
    }
    fn read(&self, file: &usage_core::SourceFile, cursor: usage_core::ReadCursor) -> usage_core::Result<usage_core::ReadOutcome> {
        usage_core::with_discovery_home(std::path::Path::new(&self.root.path), || self.inner.read(file, cursor))
    }
    fn quota(&self) -> Vec<usage_core::QuotaSample> {
        usage_core::with_discovery_home(std::path::Path::new(&self.root.path), || self.inner.quota())
    }
}

fn read_data_roots(index: &Index) -> Vec<DataRoot> {
    index.meta_value(DATA_ROOTS_KEY).ok().flatten()
        .and_then(|value| serde_json::from_str::<Vec<DataRoot>>(&value).ok())
        .unwrap_or_default()
}

fn root_id(path: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in path.trim_end_matches(['\\', '/']).to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("root-{hash:016x}")
}

fn adapters_for_roots(roots: &[DataRoot]) -> Vec<Box<dyn usage_core::SourceAdapter>> {
    let base = usage_adapter_all::builtin_adapters();
    let mut adapters = base;
    for root in roots {
        adapters.extend(usage_adapter_all::builtin_adapters().into_iter().map(|inner| {
            Box::new(RootedAdapter { inner, root: root.clone() }) as Box<dyn usage_core::SourceAdapter>
        }));
    }
    adapters
}

fn migrate_legacy_root(index: &Index, root: &DataRoot) -> Result<(), String> {
    let path = std::path::Path::new(&root.path);
    let mut prefix = path.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
    prefix.push(std::path::MAIN_SEPARATOR);
    let escaped = prefix.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_") + "%";
    let conn = index.conn();
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|error| error.to_string())?;
    let migrated = (|| -> Result<(), String> {
        let mut stmt = conn.prepare("SELECT source_key FROM file_state WHERE source_key LIKE ?1 ESCAPE '\\' COLLATE NOCASE AND source_key NOT LIKE 'linux:%'")
            .map_err(|error| error.to_string())?;
        let rows = stmt.query_map([&escaped], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let keys = rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
        drop(stmt);
        for key in &keys {
            let rooted_key = format!("linux:{}:{}", root.id, key);
            conn.execute("UPDATE event SET source = ?1 WHERE source = ?2", params![rooted_key, key])
                .map_err(|error| error.to_string())?;
            conn.execute("UPDATE file_state SET source_key = ?1 WHERE source_key = ?2", params![rooted_key, key])
                .map_err(|error| error.to_string())?;
        }
        if !keys.is_empty() {
            conn.execute("DELETE FROM event_rollup", []).map_err(|error| error.to_string())?;
        }
        Ok(())
    })();
    match migrated {
        Ok(()) => conn.execute_batch("COMMIT").map_err(|error| error.to_string()),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn set_data_roots(payload: &Value) -> Result<Value, String> {
    let submitted = payload.get("roots").and_then(Value::as_array).ok_or("roots must be an array")?;
    let mut roots = Vec::<DataRoot>::new();
    let mut seen = HashSet::new();
    for value in submitted {
        let path = value.get("path").and_then(Value::as_str).unwrap_or_default().trim();
        if path.is_empty() { continue; }
        let normalized = path.trim_end_matches(['\\', '/']).to_lowercase();
        if !seen.insert(normalized) { continue; }
        if !std::path::Path::new(path).is_absolute() {
            return Err(format!("数据目录必须是绝对路径：{path}"));
        }
        roots.push(DataRoot { id: root_id(path), path: path.to_string() });
    }
    let index = open_index()?;
    let previous = read_data_roots(&index);
    for root in &roots {
        if !previous.iter().any(|old| old.id == root.id) {
            migrate_legacy_root(&index, root)?;
        }
    }
    let serialized = serde_json::to_string(&roots).map_err(|error| error.to_string())?;
    index.conn().execute("INSERT OR REPLACE INTO meta(key, value) VALUES(?1, ?2)", params![DATA_ROOTS_KEY, serialized])
        .map_err(|error| error.to_string())?;
    serde_json::to_value(roots).map_err(|error| error.to_string())
}

fn plugin_data_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        PathBuf::from(appdata)
            .join("com.agiletiles.app")
            .join("plugin-data")
            .join(PLUGIN_ID)
    } else {
        PathBuf::from(".agile-data")
            .join("plugin-data")
            .join(PLUGIN_ID)
    }
}

fn open_index() -> Result<Index, String> {
    let directory = plugin_data_dir();
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("无法创建插件数据目录：{error}"))?;
    let path = directory.join("index.db");
    Index::open(path).map_err(|error| format!("无法打开本地用量索引：{error}"))
}

fn ingest(index: &mut Index, adapters: &[Box<dyn usage_core::SourceAdapter>]) -> Option<String> {
    let now = usage_core::report::now_ms();
    let filter = DateFilter::new(Some(retention_cutoff(now)), None);
    let opts = IngestOptions {
        force: false,
        claim_wait: Duration::from_millis(500),
    };
    match index.ingest_with(adapters, &filter, &opts) {
        Ok(_) => None,
        Err(_) => Some("读取日志时遇到问题，当前展示已建立索引中的数据。".into()),
    }
}

fn data_sync() -> &'static data_sync::DataSync {
    DATA_SYNC.get_or_init(data_sync::DataSync::default)
}

fn start_data_sync(force: bool) -> data_sync::SyncStatus {
    data_sync().start(force, || {
        let mut index = open_index()?;
        let adapters = adapters_for_roots(&read_data_roots(&index));
        if let Some(warning) = ingest(&mut index, &adapters) {
            return Err(warning);
        }
        let detected = catch_unwind(AssertUnwindSafe(usage_adapter_all::detect_all)).unwrap_or_default();
        let sources = index.source_statuses(&detected).map_err(|error| error.to_string())?;
        let value = serde_json::to_string(&sources).map_err(|error| error.to_string())?;
        index.conn().execute("INSERT OR REPLACE INTO meta(key, value) VALUES(?1, ?2)", params![SOURCE_STATUS_KEY, value])
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

fn quotas_for_selection(payload: &Value, refresh: bool) -> Vec<QuotaView> {
    let selection=payload.get("account").and_then(Value::as_str).unwrap_or("all");
    if let Ok(index)=open_index() {
        if let Some(quotas)=account_groups::quotas(&index,selection,refresh,payload.get("force").and_then(Value::as_bool).unwrap_or(false)) {return quotas}
    }
    if matches!(parse_scope(payload.get("scope").and_then(Value::as_str).unwrap_or("all")), MachineScope::Origin { .. }) {
        return Vec::new();
    }
    let force = payload.get("force").and_then(Value::as_bool).unwrap_or(false);
    let account = AccountFilter::from_selection(payload.get("account").and_then(Value::as_str).unwrap_or("all"));
    catch_unwind(AssertUnwindSafe(|| match account {
        AccountFilter::All if !refresh => usage_quota::cached(),
        AccountFilter::All if force => usage_quota::refresh_within(Duration::from_secs(2)),
        AccountFilter::All => usage_quota::collect_within(Duration::from_secs(2)),
        AccountFilter::Account { tool, id } if tool == "codex" => {
            if !refresh { usage_quota::cached_codex_for_account(&id) }
            else if force { usage_quota::refresh_codex_for_account(&id, Duration::from_secs(2)) }
            else { usage_quota::collect_codex_for_account(&id, Duration::from_secs(2)) }
        }
        _ => Vec::new(),
    })).unwrap_or_default()
}

fn quota_data(payload: &Value, context: &PluginContext) -> Result<Value, String> {
    let refresh = payload.get("refresh").and_then(Value::as_bool).unwrap_or(false);
    let quotas = quotas_for_selection(payload, refresh);
    let index = open_index()?;
    let _ = notify_quota_threshold(&index, &quotas, context);
    quota_resets::check(&index, context, &quotas)?;
    let estimates = quota_estimates::collect(&index, &quotas,
        payload.get("account").and_then(Value::as_str).unwrap_or("all"), &pricing());
    Ok(json!({ "quotas": quotas, "quota_estimates": estimates }))
}

fn pricing() -> PricingMap {
    let mut options = usage_core::pricing::PricingOptions {
        offline: true,
        cache_dir: usage_core::pricing::PricingOptions::default_cache_dir(),
        overrides: Default::default(),
    };
    let prices = catch_unwind(AssertUnwindSafe(|| PricingMap::load(&options)))
        .unwrap_or_else(|_| PricingMap::default());

    // Price feeds are ancillary to usage totals. Never make a dashboard or
    // detail query wait on a stale-cache network request; serve the cached
    // table now, then let the next request pick up the refreshed cache file.
    if prices.meta().stale && options.cache_dir.is_some() {
        let now = usage_core::report::now_ms();
        let retry_after = PRICING_RETRY_AFTER_MS.load(Ordering::Relaxed);
        if now >= retry_after
            && PRICING_REFRESH_ACTIVE
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        {
            PRICING_RETRY_AFTER_MS.store(now + 30 * 60 * 1000, Ordering::Relaxed);
            options.offline = false;
            if std::thread::Builder::new()
                .name("ai-usage-pricing-refresh".into())
                .spawn(move || {
                    let _ = catch_unwind(AssertUnwindSafe(|| PricingMap::load(&options)));
                    PRICING_REFRESH_ACTIVE.store(false, Ordering::Release);
                })
                .is_err()
            {
                PRICING_REFRESH_ACTIVE.store(false, Ordering::Release);
            }
        }
    }
    prices
}

fn parse_scope(value: &str) -> MachineScope {
    match value {
        "local" => MachineScope::Local,
        "all" | "" => MachineScope::All,
        value if value.starts_with("origin:") => MachineScope::Origin { name: value[7..].to_string() },
        _ => MachineScope::All,
    }
}

fn dashboard(payload: &Value, _context: &PluginContext) -> Result<Value, String> {
    let mut index = open_index()?;
    let data_roots = read_data_roots(&index);
    let supported_tools = usage_adapter_all::builtin_adapters()
        .iter()
        .map(|adapter| SupportedTool {
            id: adapter.id().to_string(),
            display: adapter.display_name().to_string(),
        })
        .collect();
    let warning = data_sync().status().warning;
    let cached_sources = index.meta_value(SOURCE_STATUS_KEY).ok().flatten()
        .and_then(|value| serde_json::from_str::<Vec<usage_core::SourceStatus>>(&value).ok());
    let sources = match cached_sources {
        Some(sources) => sources,
        None => index.source_statuses(&[]).map_err(|error| error.to_string())?,
    };
    let scope = parse_scope(payload.get("scope").and_then(Value::as_str).unwrap_or("all"));
    let account_selection = payload.get("account").and_then(Value::as_str).unwrap_or("all");
    let account_filter = AccountFilter::from_selection(account_selection);
    let quotas = quotas_for_selection(payload, false);
    let now = usage_core::report::now_ms();
    let plan = AggregatePlan::build(now, 200);
    let facts = index
        .report_facts_for_account(&plan, &scope, &account_filter)
        .map_err(|error| format!("无法汇总用量数据：{error}"))?;
    let account_names = read_account_names();
    let mut accounts: Vec<AccountOption> = index
        .accounts()
        .map_err(|error| format!("无法读取 账号：{error}"))?
        .into_iter()
        .map(|account| {
            let selection = make_account_selection(&account.tool, account.account_id.as_deref());
            let name = account_names
                .get(&selection)
                .or_else(|| account.account_id.as_ref().and_then(|id| account_names.get(id)))
                .cloned();
            let label = match account.account_id.as_deref() {
                Some(id) => {
                    let suffix: String = id.chars().rev().take(6).collect::<String>().chars().rev().collect();
                    if id.starts_with("profile:") {
                        format!("配置 · {suffix}")
                    } else {
                        format!("账号 · {suffix}")
                    }
                }
                None => "未标记账号".to_string(),
            };
            AccountOption {
                tool: account.tool,
                selection,
                label,
                name,
                requests: account.events,
                sessions: account.sessions,
            }
        })
        .collect();
    for group in index.account_groups()? {
        let clause=usage_index::account_groups::group_clause("'group'","?1","e.tool","extra.account_id","CASE WHEN substr(e.source,1,6)='linux:' AND instr(substr(e.source,7),':')>0 THEN substr(e.source,7,instr(substr(e.source,7),':')-1) ELSE '' END");
        let (requests,sessions):(i64,i64)=index.conn().query_row(&format!("SELECT COUNT(*),COUNT(DISTINCT e.session) FROM event e LEFT JOIN event_extra extra ON extra.event_id=e.id WHERE {clause}"),[&group.id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        accounts.push(AccountOption {tool:group.tool,selection:format!("group:{}",group.id),label:format!("统一账号 · {}",group.name),name:Some(group.name),requests:requests.max(0) as u64,sessions:sessions.max(0) as u64});
    }
    let prices = pricing();
    let mut options = ReportOptions::new(&prices);
    options.now_ms = now;
    options.scope = scope;
    options.sources = sources;
    options.polled_quota = quotas;
    options.budgets = load_budgets();
    options.recent_session_limit = 200;
    let report = summarize_facts(&facts, &options);
    let quota_estimates = quota_estimates::collect(&index, &report.quotas, account_selection, &prices);

    let indexed_events = index
        .event_count()
        .map_err(|error| format!("无法读取索引统计：{error}"))?;
    let last_ingest_ms = index
        .meta_value("last_ingest_ms")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<i64>().ok());
    serde_json::to_value(DashboardPayload {
        report,
        quota_estimates,
        indexed_events,
        last_ingest_ms,
        refresh_warning: warning,
        supported_tools,
        data_roots,
        accounts,
    })
    .map_err(|error| format!("无法序列化用量报告：{error}"))
}

#[derive(Serialize)]
struct RequestRow {
    tool: String,
    timestamp_ms: i64,
    session: String,
    session_title: Option<String>,
    account_id: Option<String>,
    duration_ms: Option<i64>,
    project: Option<String>,
    model: Option<String>,
    counts: TokenCounts,
    total_tokens: f64,
    cost_usd: Option<f64>,
    pricing_alias: Option<String>,
    credits_cost_usd: Option<f64>,
}

#[derive(Serialize)]
struct SessionUsageRow {
    tool: String,
    session: String,
    session_title: Option<String>,
    account_id: Option<String>,
    group_id: Option<String>,
    project: Option<String>,
    model: String,
    first_ms: i64,
    last_ms: i64,
    total_tokens: f64,
    cost: f64,
    requests: u64,
}

#[derive(Default)]
struct SessionUsageAccumulator {
    row: Option<SessionUsageRow>,
    latest_model_ms: i64,
}

fn session_rows(payload: &Value) -> Result<Value, String> {
    let index = open_index()?;
    let warning = data_sync().status().warning;
    session_rows_from_index(&index,payload,warning,&pricing())
}

fn session_rows_from_index(index:&Index,payload:&Value,warning:Option<String>,prices:&PricingMap)->Result<Value,String> {
    let now = usage_core::report::now_ms();
    let since_ms = payload
        .get("since_ms")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| now - 7 * 86_400_000)
        .max(retention_cutoff(now));
    let until_ms = payload
        .get("until_ms")
        .and_then(Value::as_i64)
        .unwrap_or(now)
        .min(now);
    if since_ms > until_ms {
        return Err("开始日期必须早于结束日期".into());
    }
    let scope = payload.get("scope").and_then(Value::as_str).unwrap_or("all");
    let account = payload.get("account").and_then(Value::as_str).unwrap_or("all");
    let account_filter = AccountFilter::from_selection(account);
    let (account_kind, account_key) = account_filter.sql_parts();
    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(200)
        .clamp(1, MAX_SESSION_LIMIT);

    // Aggregate inside the selected time range by model and meter, then fold
    // model groups into sessions so pricing remains correct for mixed-model
    // sessions. Conversation contents are never selected.
    let mut statement = index
        .conn()
        .prepare(
            "SELECT e.tool, e.session, MAX(NULLIF(e.session_title, '')), MAX(extra.account_id), MAX(NULLIF(e.project, '')), \
                    e.model, e.meter, MIN(e.ts_ms), MAX(e.ts_ms), \
                    COALESCE(SUM(e.in_tok), 0), COALESCE(SUM(e.cc_tok), 0), COALESCE(SUM(e.cr_tok), 0), \
                    COALESCE(SUM(e.out_tok), 0), COALESCE(SUM(e.reason_tok), 0), COALESCE(SUM(e.credits), 0), COUNT(*) \
             FROM event e LEFT JOIN event_extra extra ON extra.event_id = e.id \
             WHERE e.ts_ms >= ?1 AND e.ts_ms <= ?2 \
               AND (?3 = 'all' OR (?3 = 'local' AND e.source NOT LIKE 'linux:%') \
                    OR (substr(?3, 1, 7) = 'origin:' AND e.source LIKE 'linux:' || substr(?3, 8) || ':%')) \
               AND (?4 = 'all' OR (?4 = 'account' AND e.tool || '|' || extra.account_id = ?5) \
                    OR (?4 = 'group' AND EXISTS(SELECT 1 FROM account_group_member gm WHERE gm.group_id=?5 AND gm.tool=e.tool AND gm.origin=(CASE WHEN substr(e.source,1,6)='linux:' AND instr(substr(e.source,7),':')>0 THEN substr(e.source,7,instr(substr(e.source,7),':')-1) ELSE '' END) AND gm.account_id=extra.account_id)) OR (?4 = 'unknown' AND e.tool || '|' || COALESCE(extra.account_id, '') = ?5)) \
             GROUP BY e.tool, e.session, e.model, e.meter, e.source, extra.account_id \
             ORDER BY MAX(e.ts_ms) DESC",
        )
        .map_err(|error| format!("无法读取会话排行：{error}"))?;
    let groups = statement
        .query_map(params![since_ms, until_ms, scope, account_kind, account_key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                TokenCounts {
                    input: row.get(9)?,
                    cache_creation: row.get(10)?,
                    cache_read: row.get(11)?,
                    output: row.get(12)?,
                    reasoning: row.get(13)?,
                    credits: row.get(14)?,
                },
                row.get::<_, i64>(15)?.max(0) as u64,
            ))
        })
        .map_err(|error| format!("无法读取会话排行：{error}"))?;
    let grouped = groups
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法读取会话排行：{error}"))?;
    drop(statement);

    let mut sessions = std::collections::HashMap::<(String, String, Option<String>), SessionUsageAccumulator>::new();
    for (tool, session, title, account_id, project, model, meter, first_ms, last_ms, counts, requests) in grouped {
        let group_id=if let AccountFilter::Group(id)=&account_filter {Some(id.clone())} else {None};
        let key = (tool.clone(), session.clone(), group_id.as_ref().map(|id|format!("group:{id}")).or_else(||account_id.clone()));
        let accumulator = sessions.entry(key).or_default();
        let row = accumulator.row.get_or_insert_with(|| SessionUsageRow {
            tool: tool.clone(),
            session: session.clone(),
            session_title: title.clone(),
            account_id: if group_id.is_some(){None}else{account_id.clone()},
            group_id,
            project: project.clone(),
            model: model.clone().unwrap_or_else(|| "unknown".into()),
            first_ms,
            last_ms,
            total_tokens: 0.0,
            cost: 0.0,
            requests: 0,
        });
        row.first_ms = row.first_ms.min(first_ms);
        row.last_ms = row.last_ms.max(last_ms);
        if row.session_title.as_deref().unwrap_or("").is_empty() {
            row.session_title = title;
        }
        if row.project.as_deref().unwrap_or("").is_empty() {
            row.project = project;
        }
        if last_ms >= accumulator.latest_model_ms {
            row.model = model.clone().unwrap_or_else(|| "unknown".into());
            accumulator.latest_model_ms = last_ms;
        }
        row.total_tokens += counts.total();
        row.requests += requests;
        row.cost += if meter == "credits" {
            usage_core::pricing::credit_rate(&tool)
                .map(|(rate, _)| counts.credits * rate)
                .unwrap_or(0.0)
        } else {
            prices.cost(model.as_deref(), &counts).unwrap_or(0.0)
        };
    }
    let mut rows: Vec<_> = sessions.into_values().filter_map(|entry| entry.row).collect();
    rows.sort_by(|a, b| b.cost.total_cmp(&a.cost).then_with(|| b.last_ms.cmp(&a.last_ms)));
    rows.truncate(limit);

    Ok(json!({
        "rows": rows,
        "since_ms": since_ms,
        "until_ms": until_ms,
        "refresh_warning": warning,
        "scope": scope,
    }))
}

fn request_rows(payload: &Value) -> Result<Value, String> {
    let index = open_index()?;
    let warning = data_sync().status().warning;
    let now = usage_core::report::now_ms();
    request_rows_from_index(&index, payload, now, &pricing(), warning)
}

fn request_rows_from_index(index: &Index, payload: &Value, now: i64, prices: &PricingMap, warning: Option<String>) -> Result<Value, String> {
    let retention_start = retention_cutoff(now);
    let since_ms = payload
        .get("since_ms")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| now - 7 * 86_400_000)
        .max(retention_start);
    let until_ms = payload
        .get("until_ms")
        .and_then(Value::as_i64)
        .unwrap_or(now)
        .min(now);
    if since_ms > until_ms {
        return Err("开始日期必须早于结束日期".into());
    }
    let tool = payload.get("tool").and_then(Value::as_str).unwrap_or("");
    let search = payload.get("search").and_then(Value::as_str).unwrap_or("");
    let scope = payload.get("scope").and_then(Value::as_str).unwrap_or("all");
    let account = payload.get("account").and_then(Value::as_str).unwrap_or("all");
    let account_filter = AccountFilter::from_selection(account);
    let (account_kind, account_key) = account_filter.sql_parts();
    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_REQUEST_LIMIT)
        .clamp(1, MAX_REQUEST_LIMIT);

    // Count and fetch from the same SQLite read snapshot. Keep the highest
    // event ID across pages so an ingest cannot shift new rows into that set.
    let transaction = index.conn().unchecked_transaction().map_err(|error| error.to_string())?;
    let snapshot_max_id = match payload.get("snapshot_max_id").and_then(Value::as_i64).filter(|id| *id >= 0) {
        Some(id) => id,
        None => transaction.query_row("SELECT COALESCE(MAX(id),0) FROM event", [], |row| row.get::<_, i64>(0))
            .map_err(|error| error.to_string())?,
    };
    const FILTER: &str = "FROM event e LEFT JOIN event_extra extra ON extra.event_id = e.id \
        WHERE e.ts_ms >= ?1 AND e.ts_ms <= ?2 AND e.id <= ?8 \
          AND (?3 = '' OR e.tool = ?3) \
          AND (?4 = 'all' OR (?4 = 'local' AND e.source NOT LIKE 'linux:%') \
               OR (substr(?4, 1, 7) = 'origin:' AND e.source LIKE 'linux:' || substr(?4, 8) || ':%')) \
          AND (?5 = '' OR instr(lower(COALESCE(e.session_title, '') || ' ' || COALESCE(e.project, '') || ' ' || COALESCE(e.model, '')), lower(?5)) > 0) \
          AND (?6 = 'all' OR (?6 = 'account' AND e.tool || '|' || extra.account_id = ?7) \
               OR (?6 = 'group' AND EXISTS(SELECT 1 FROM account_group_member gm WHERE gm.group_id=?7 AND gm.tool=e.tool AND gm.origin=(CASE WHEN substr(e.source,1,6)='linux:' AND instr(substr(e.source,7),':')>0 THEN substr(e.source,7,instr(substr(e.source,7),':')-1) ELSE '' END) AND gm.account_id=extra.account_id)) OR (?6 = 'unknown' AND e.tool || '|' || COALESCE(extra.account_id, '') = ?7))";
    let total = transaction.query_row(&format!("SELECT COUNT(*) {FILTER}"),
        params![since_ms, until_ms, tool, scope, search, account_kind, account_key, snapshot_max_id],
        |row| row.get::<_, i64>(0)).map_err(|error| format!("无法统计请求明细：{error}"))? as u64;
    let total_pages = total.div_ceil(limit as u64).max(1);
    let page = payload.get("page").and_then(Value::as_u64).unwrap_or(1).clamp(1, total_pages);
    let offset = (page - 1) * limit as u64;

    // This query reads only event metadata and source-provided session titles
    // already in the local index; prompts and responses are never returned.
    let mut statement = transaction
        .prepare(
            &format!("SELECT e.tool, e.ts_ms, e.session, e.session_title, e.project, e.model, extra.account_id, extra.duration_ms, \
                    COALESCE(e.in_tok, 0), COALESCE(e.cc_tok, 0), COALESCE(e.cr_tok, 0), \
                    COALESCE(e.out_tok, 0), COALESCE(e.reason_tok, 0), COALESCE(e.credits, 0) \
             {FILTER} ORDER BY e.ts_ms DESC, e.id DESC LIMIT ?9 OFFSET ?10"),
        )
        .map_err(|error| format!("无法读取请求明细：{error}"))?;
    let records = statement
        .query_map(
            params![since_ms, until_ms, tool, scope, search, account_kind, account_key, snapshot_max_id, limit as i64, offset as i64],
            |row| {
                Ok(RequestRow {
                    tool: row.get(0)?,
                    timestamp_ms: row.get(1)?,
                    session: row.get(2)?,
                    session_title: row.get(3)?,
                    account_id: row.get(6)?,
                    project: row.get(4)?,
                    model: row.get(5)?,
                    duration_ms: row.get(7)?,
                    counts: TokenCounts {
                        input: row.get(8)?,
                        cache_creation: row.get(9)?,
                        cache_read: row.get(10)?,
                        output: row.get(11)?,
                        reasoning: row.get(12)?,
                        credits: row.get(13)?,
                    },
                    total_tokens: 0.0,
                    cost_usd: None,
                    pricing_alias: None,
                    credits_cost_usd: None,
                })
            },
        )
        .map_err(|error| format!("无法读取请求明细：{error}"))?;
    let mut rows = records
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法读取请求明细：{error}"))?;
    drop(statement);
    transaction.commit().map_err(|error| error.to_string())?;

    for row in &mut rows {
        // Codex only records duration for a whole task, so it cannot be shown
        // as a per-request duration in this event-level list.
        if row.tool == "codex" {
            row.duration_ms = None;
        }
        row.total_tokens = row.counts.total();
        row.cost_usd = prices.cost(row.model.as_deref(), &row.counts);
        row.pricing_alias = row.model.as_deref().and_then(|model| prices.explain(model))
            .and_then(|explanation| explanation.matched_alias).map(str::to_string);
        row.credits_cost_usd = usage_core::pricing::credit_rate(&row.tool)
            .map(|(rate, _)| row.counts.credits * rate);
    }

    Ok(json!({
        "rows": rows,
        "total": total,
        "page": page,
        "page_size": limit,
        "total_pages": total_pages,
        "snapshot_max_id": snapshot_max_id,
        "since_ms": since_ms,
        "until_ms": until_ms,
        "refresh_warning": warning,
        "scope": scope,
    }))
}

#[cfg(test)]
mod request_pagination_tests {
    use super::*;
    const NOW: i64 = 1_790_000_000_000;

    #[test]
    fn group_request_filters_include_only_member_sources_with_no_duplicate_rows() {
        let index=Index::open_in_memory().unwrap();
        add(&index,"native","local","a","Native");
        add(&index,"wsl","linux:wsl:file","a","WSL");
        add(&index,"other","linux:other:file","a","Other");
        add(&index,"unknown","local","","Unknown");
        let members=index.source_accounts().unwrap().into_iter().filter(|i|i.origin!="other").map(|i|i.id).collect();
        index.save_account_group(&usage_index::account_groups::AccountGroup{id:"g".into(),tool:"codex".into(),name:"Unified".into(),members,quota_identity_id:None}).unwrap();
        let prices=PricingMap::default();
        let result=request_rows_from_index(&index,&json!({"account":"group:g"}),NOW,&prices,None).unwrap();
        assert_eq!(result["total"],2);
        let local=request_rows_from_index(&index,&json!({"account":"group:g","scope":"local"}),NOW,&prices,None).unwrap();
        assert_eq!(local["total"],1);
        index.delete_account_group("g").unwrap();
        assert_eq!(request_rows_from_index(&index,&json!({"account":"group:g"}),NOW,&prices,None).unwrap()["total"],0);
        assert_eq!(index.event_count().unwrap(),4);
    }

    #[test]
    fn unified_session_ranking_merges_member_aliases_without_changing_raw_accounts() {
        let index=Index::open_in_memory().unwrap();
        add(&index,"shared","local","a","Title");
        add(&index,"shared","linux:wsl:file","b","Title");
        add(&index,"unrelated","linux:other:file","c","Other");
        let members=index.source_accounts().unwrap().into_iter().filter(|i|i.origin!="other").map(|i|i.id).collect();
        index.save_account_group(&usage_index::account_groups::AccountGroup{id:"g".into(),tool:"codex".into(),name:"Unified".into(),members,quota_identity_id:None}).unwrap();
        let result=session_rows_from_index(&index,&json!({"account":"group:g","since_ms":NOW-100,"until_ms":NOW}),None,&PricingMap::default()).unwrap();
        let rows=result["rows"].as_array().unwrap();
        assert_eq!(rows.len(),1);
        assert_eq!(rows[0]["requests"],2);
        assert_eq!(rows[0]["group_id"],"g");
        let ids:i64=index.conn().query_row("SELECT COUNT(DISTINCT account_id) FROM event_extra",[],|r|r.get(0)).unwrap();
        assert_eq!(ids,3);
    }

    #[test]
    fn antigravity_requests_keep_titles_and_explain_internal_model_price_estimates() {
        let index = Index::open_in_memory().unwrap();
        index.conn().execute("INSERT INTO event(tool,ts_ms,session,session_title,model,meter,in_tok,cr_tok,out_tok,source) \
            VALUES('antigravity',?1,'fixture','Fixture session','gemini-3.8-flash-n','tokens',1000,2000,500,'local')", [NOW - 1]).unwrap();
        let prices = PricingMap::default();
        let result = request_rows_from_index(&index, &json!({"tool": "antigravity"}), NOW, &prices, None).unwrap();
        let row = &result["rows"][0];
        assert_eq!(row["session_title"], "Fixture session");
        assert_eq!(row["model"], "gemini-3.8-flash-n", "keep the original model id");
        assert_eq!(row["pricing_alias"], "gemini-3.8-flash");
        let expected = prices.cost(Some("gemini-3.8-flash"), &TokenCounts { input: 1000.0, cache_read: 2000.0, output: 500.0, ..Default::default() }).unwrap();
        assert!((row["cost_usd"].as_f64().unwrap() - expected).abs() < 1e-12);
    }

    fn add(index: &Index, session: &str, source: &str, account: &str, title: &str) {
        index.conn().execute("INSERT INTO event(tool,ts_ms,session,session_title,model,meter,in_tok,source) VALUES('codex',?1,?2,?3,'gpt-5','tokens',10,?4)",
            params![NOW - 1, session, title, source]).unwrap();
        let id = index.conn().last_insert_rowid();
        index.conn().execute("INSERT INTO event_extra(event_id,account_id) VALUES(?1,?2)", params![id, account]).unwrap();
    }

    #[test]
    fn pages_reach_every_record_past_200_without_new_ingests_shifting_them() {
        let index = Index::open_in_memory().unwrap();
        for id in 0..425 { add(&index, &format!("s-{id}"), "local", "a", "Task"); }
        let prices = PricingMap::default();
        let first = request_rows_from_index(&index, &json!({"limit": 200}), NOW, &prices, None).unwrap();
        assert_eq!(first["total"], 425);
        assert_eq!(first["total_pages"], 3);
        let mut sessions: HashSet<String> = first["rows"].as_array().unwrap().iter().map(|row| row["session"].as_str().unwrap().into()).collect();
        assert_eq!(sessions.len(), 200);
        add(&index, "new-after-first-page", "local", "a", "Task");
        for page in [2, 3] {
            let result = request_rows_from_index(&index, &json!({"limit": 200, "page": page, "snapshot_max_id": first["snapshot_max_id"]}), NOW, &prices, None).unwrap();
            assert_eq!(result["total"], 425);
            assert_eq!(result["rows"].as_array().unwrap().len(), if page == 2 { 200 } else { 25 });
            for row in result["rows"].as_array().unwrap() { assert!(sessions.insert(row["session"].as_str().unwrap().into())); }
        }
        assert_eq!(sessions.len(), 425);
        assert!(!sessions.contains("new-after-first-page"));
        let fresh = request_rows_from_index(&index, &json!({"limit": 100}), NOW, &prices, None).unwrap();
        assert_eq!(fresh["total"], 426);
        assert_eq!(fresh["rows"][0]["session"], "new-after-first-page");
    }

    #[test]
    fn count_and_page_rows_share_search_source_account_and_date_filters() {
        let index = Index::open_in_memory().unwrap();
        add(&index, "match-1", "local", "a", "Match title");
        add(&index, "match-2", "local", "a", "Match title");
        add(&index, "other-account", "local", "b", "Match title");
        add(&index, "other-source", "linux:remote:file", "a", "Match title");
        add(&index, "other-title", "local", "a", "Other title");
        let prices = PricingMap::default();
        let query = json!({"limit": 1, "page": u64::MAX, "scope": "local", "account": "account:codex|a", "search": "match", "since_ms": NOW - 60_000, "until_ms": NOW});
        let result = request_rows_from_index(&index, &query, NOW, &prices, None).unwrap();
        assert_eq!(result["total"], 2);
        assert_eq!(result["total_pages"], 2);
        assert_eq!(result["page"], 2);
        assert_eq!(result["rows"][0]["session"], "match-1");
        let mut no_match = query.clone();
        no_match["tool"] = json!("claude");
        assert_eq!(request_rows_from_index(&index, &no_match, NOW, &prices, None).unwrap()["total"], 0);
        no_match = query;
        no_match["since_ms"] = json!(NOW);
        assert_eq!(request_rows_from_index(&index, &no_match, NOW, &prices, None).unwrap()["total"], 0);
    }

    #[test]
    fn empty_queries_and_invalid_page_sizes_are_bounded() {
        let index = Index::open_in_memory().unwrap();
        let prices = PricingMap::default();
        let result = request_rows_from_index(&index, &json!({"page": u64::MAX, "limit": 0}), NOW, &prices, None).unwrap();
        assert_eq!(result["total"], 0);
        assert_eq!(result["page"], 1);
        assert_eq!(result["total_pages"], 1);
        assert_eq!(result["page_size"], 1);
        let result = request_rows_from_index(&index, &json!({"limit": u64::MAX}), NOW, &prices, None).unwrap();
        assert_eq!(result["page_size"], MAX_REQUEST_LIMIT);
    }
}
