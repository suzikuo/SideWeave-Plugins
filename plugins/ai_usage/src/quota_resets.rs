use std::collections::BTreeMap;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use chrono::{Local, TimeZone};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sideweave_plugin_sdk::{DesktopNotificationKind, DesktopNotificationOptions, PluginContext};
use usage_core::{QuotaOrigin, QuotaView};
use usage_index::Index;

const KEY: &str = "ai_usage_quota_reset_ledger";
const LATE_GRACE_MS: i64 = 24 * 60 * 60_000;
static CHECK_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Reminder {
    tool: String,
    label: String,
    reset_at_ms: i64,
    notified: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    pending: BTreeMap<String, Reminder>,
}

impl Ledger {
    fn due(&self, enabled: &impl Fn(&str) -> bool, now: i64) -> Vec<(String, Reminder)> {
        self.pending.iter().filter(|(key, item)| enabled(key) && !item.notified
            && item.reset_at_ms <= now && now - item.reset_at_ms <= LATE_GRACE_MS)
            .map(|(key, item)| (key.clone(), item.clone())).collect()
    }

    fn observe(&mut self, quotas: &[QuotaView], enabled: &impl Fn(&str) -> bool, now: i64) {
        self.pending.retain(|key, item| enabled(key) && item.reset_at_ms >= now - LATE_GRACE_MS);
        for quota in quotas {
            let key = super::quota_notification_identity(quota);
            if quota.origin != QuotaOrigin::Probe || !enabled(&key) || quota.resets_at_ms <= now { continue }
            let notified = self.pending.get(&key).is_some_and(|item| item.notified
                && item.reset_at_ms.abs_diff(quota.resets_at_ms) <= super::QUOTA_RESET_DRIFT_TOLERANCE_MS as u64);
            self.pending.insert(key, Reminder { tool: quota.tool.clone(), label: super::quota_notification_window(quota),
                reset_at_ms: quota.resets_at_ms, notified });
        }
    }
}

pub(super) fn check(index: &Index, context: &PluginContext, quotas: &[QuotaView]) -> Result<usize, String> {
    let _guard = CHECK_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let settings = super::read_quota_notification_settings(index);
    let enabled = |key: &str| settings.subscriptions.get(key).is_some_and(|rule| rule.reset_enabled);
    let now = usage_core::report::now_ms();
    let mut ledger: Ledger = index.meta_value(KEY).ok().flatten()
        .and_then(|value| serde_json::from_str(&value).ok()).unwrap_or_default();
    // Check the saved deadline before observing a newly rolled-over API window.
    // Otherwise a refresh at reset time would erase the reminder just as it is due.
    let due = ledger.due(&enabled, now);
    let mut sent = 0;
    for (key, item) in due {
        let tool = usage_adapter_all::builtin_adapters().into_iter().find(|adapter| adapter.id() == item.tool)
            .map(|adapter| adapter.display_name().to_string()).unwrap_or(item.tool);
        let at = Local.timestamp_millis_opt(item.reset_at_ms).single()
            .map(|at| at.format("%Y-%m-%d %H:%M:%S %:z").to_string()).unwrap_or_default();
        context.notification().show_desktop(&DesktopNotificationOptions::new("AI 订阅重置提醒",
            format!("{tool} · {} 已到计划重置时间\n{at}\n可刷新订阅确认最新额度。", item.label))
            .with_kind(DesktopNotificationKind::Info).with_duration_ms(10_000))
            .map_err(|error| error.message)?;
        if let Some(item) = ledger.pending.get_mut(&key) { item.notified = true; }
        // Persist each successful delivery before trying another window.
        save(index, &ledger)?;
        sent += 1;
    }
    ledger.observe(quotas, &enabled, now);
    save(index, &ledger)?;
    Ok(sent)
}

fn save(index: &Index, ledger: &Ledger) -> Result<(), String> {
    let value = serde_json::to_string(ledger).map_err(|error| error.to_string())?;
    if index.meta_value(KEY).ok().flatten().as_deref() == Some(value.as_str()) { return Ok(()) }
    index.conn().execute("INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![KEY, value])
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) struct Worker {
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn start(context: &PluginContext) -> Result<Self, String> {
        let context = PluginContext::new(context.plugin_id().clone(), context.services_arc());
        let (stop, receive) = mpsc::channel();
        let thread = std::thread::Builder::new().name("ai-usage-reset-reminders".into()).spawn(move || {
            let mut index = None;
            loop {
                if index.is_none() { index = super::open_index().ok(); }
                if let Some(index) = index.as_ref() {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(index, &context, &usage_quota::cached())));
                }
                match receive.recv_timeout(Duration::from_secs(15)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                    _ => break,
                }
            }
        }).map_err(|error| format!("无法启动重置提醒：{error}"))?;
        Ok(Self { stop, thread: Some(thread) })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quota(reset: i64) -> QuotaView {
        QuotaView { tool: "codex".into(), used_percent: 50.0, window_minutes: 300,
            resets_at_ms: reset, sampled_at_ms: 1, label: None, id: None, origin: QuotaOrigin::Probe }
    }

    #[test]
    fn due_reminder_survives_rollover_restart_and_only_fires_once() {
        let mut ledger = Ledger::default();
        ledger.observe(&[quota(10_000)], &|_| true, 1_000);
        assert!(ledger.due(&|_| true, 9_999).is_empty());
        let saved = serde_json::to_string(&ledger).unwrap();
        let mut ledger: Ledger = serde_json::from_str(&saved).unwrap();
        let due = ledger.due(&|_| true, 10_001);
        assert_eq!(due.len(), 1);
        ledger.pending.get_mut(&due[0].0).unwrap().notified = true;
        assert!(ledger.due(&|_| true, 10_002).is_empty());
        ledger.observe(&[quota(10_000 + 5 * 60 * 60_000)], &|_| true, 10_002);
        assert_eq!(ledger.due(&|_| true, 10_000 + 5 * 60 * 60_000).len(), 1);
    }

    #[test]
    fn disabled_unknown_and_ancient_resets_do_not_notify() {
        let mut ledger = Ledger::default();
        ledger.observe(&[quota(0), quota(100)], &|_| true, 1_000);
        assert!(ledger.pending.is_empty());
        ledger.observe(&[quota(10_000)], &|_| true, 1_000);
        assert!(ledger.due(&|_| false, 10_001).is_empty());
        assert!(ledger.due(&|_| true, 10_000 + LATE_GRACE_MS + 1).is_empty());
        ledger.observe(&[], &|_| false, 10_001);
        assert!(ledger.pending.is_empty());
    }

    #[test]
    fn delivery_is_persisted_and_reset_only_rules_work_without_threshold_alerts() {
        use sideweave_plugin_sdk::mock::MockHostServices;
        let index = Index::open_in_memory().unwrap();
        let host = std::sync::Arc::new(MockHostServices::default());
        let context = PluginContext::new(sideweave_plugin_api::PluginId::new("ai_usage").unwrap(), host.clone());
        let now = usage_core::report::now_ms();
        let quota = quota(now + 60_000);
        let key = super::super::quota_notification_identity(&quota);
        let settings = serde_json::json!({"subscriptions": {key.clone(): {
            "enabled": false, "reset_enabled": true, "remaining_thresholds_percent": [50]
        }}}).to_string();
        index.conn().execute("INSERT INTO meta(key,value) VALUES(?1,?2)", params![super::super::QUOTA_NOTIFICATION_SETTINGS_KEY, settings]).unwrap();
        assert_eq!(check(&index, &context, &[quota]).unwrap(), 0);
        let mut ledger: Ledger = serde_json::from_str(&index.meta_value(KEY).unwrap().unwrap()).unwrap();
        ledger.pending.get_mut(&key).unwrap().reset_at_ms = now - 1;
        save(&index, &ledger).unwrap();
        assert_eq!(check(&index, &context, &[]).unwrap(), 1);
        assert_eq!(check(&index, &context, &[]).unwrap(), 0);
        let notifications = host.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].0, "AI 订阅重置提醒");
        assert!(notifications[0].1.contains("计划重置时间"));
        let stored: Ledger = serde_json::from_str(&index.meta_value(KEY).unwrap().unwrap()).unwrap();
        assert!(stored.pending[&key].notified);
    }
}
