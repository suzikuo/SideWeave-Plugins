use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use serde::Serialize;

pub const DATA_SYNC_INTERVAL_MS: i64 = 60_000;

#[derive(Clone, Default, Serialize)]
pub struct SyncStatus {
    pub running: bool,
    pub last_finished_ms: Option<i64>,
    pub warning: Option<String>,
}

/// One background scan shared by detail and preview windows. Reads never wait
/// for parsing logs, and repeated opens cannot start overlapping scans.
#[derive(Clone, Default)]
pub struct DataSync {
    state: Arc<Mutex<SyncStatus>>,
}

impl DataSync {
    pub fn status(&self) -> SyncStatus {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }

    pub fn start(&self, force: bool, work: impl FnOnce() -> Result<(), String> + Send + 'static) -> SyncStatus {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let now = usage_core::report::now_ms();
        let fresh = state.last_finished_ms.is_some_and(|stamp| now - stamp < DATA_SYNC_INTERVAL_MS);
        if state.running || (!force && fresh) {
            return state.clone();
        }
        state.running = true;
        let shared = Arc::clone(&self.state);
        if let Err(error) = std::thread::Builder::new().name("ai-usage-data-sync".into()).spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(work))
                .unwrap_or_else(|_| Err("日志同步中断，当前继续展示已建立索引的数据。".into()));
            let mut state = shared.lock().unwrap_or_else(|error| error.into_inner());
            state.warning = result.err();
            state.last_finished_ms = Some(usage_core::report::now_ms());
            state.running = false;
        }) {
            state.running = false;
            state.warning = Some(format!("无法启动日志同步：{error}"));
        }
        state.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_are_nonblocking_deduplicated_and_throttled() {
        let sync = DataSync::default();
        let (release, wait) = std::sync::mpsc::channel();
        assert!(sync.start(false, move || { wait.recv().unwrap(); Ok(()) }).running);
        assert!(sync.start(true, || panic!("must not start a second scan")).running);
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while sync.status().running {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(!sync.start(false, || panic!("fresh scans must be reused")).running);
        assert!(sync.status().warning.is_none());
    }

    #[test]
    fn failed_scan_releases_the_slot_and_manual_refresh_can_retry() {
        let sync = DataSync::default();
        sync.start(true, || Err("unreadable log".into()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while sync.status().running {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(sync.status().warning.as_deref(), Some("unreadable log"));
        let (release, wait) = std::sync::mpsc::channel();
        assert!(sync.start(true, move || { wait.recv().unwrap(); Ok(()) }).running);
        release.send(()).unwrap();
    }
}
