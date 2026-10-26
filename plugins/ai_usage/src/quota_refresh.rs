use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Owns the subscription cadence independently of webview visibility or log scans.
pub(super) struct Worker {
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn start() -> Result<Self, String> {
        Self::start_with(usage_quota::TTL, || {
            // Probes deduplicate in-flight requests and update the shared cache
            // when they finish. Never wait for vendor HTTPS on plugin shutdown.
            // Force this scheduled pass: a prior request may have finished a
            // few seconds after the last tick, so a TTL check here would skip
            // every other five-minute tick and stretch refreshes to ten minutes.
            let _ = usage_quota::refresh_within(Duration::ZERO);
            super::account_groups::refresh_background();
        })
    }

    fn start_with(interval: Duration, mut refresh: impl FnMut() + Send + 'static) -> Result<Self, String> {
        let (stop, receive) = mpsc::channel();
        let thread = std::thread::Builder::new().name("ai-usage-quota-refresh".into()).spawn(move || {
            loop {
                match receive.try_recv() {
                    Err(mpsc::TryRecvError::Empty) => {},
                    _ => break,
                }
                let started = Instant::now();
                let _ = catch_unwind(AssertUnwindSafe(&mut refresh));
                match receive.recv_timeout(interval.saturating_sub(started.elapsed())) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                    _ => break,
                }
            }
        }).map_err(|error| format!("无法启动订阅后台刷新：{error}"))?;
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

    #[test]
    fn starts_without_a_webview_repeats_and_stops_on_drop() {
        let (send, receive) = mpsc::channel();
        let worker = Worker::start_with(Duration::from_millis(20), move || { let _ = send.send(()); }).unwrap();
        receive.recv_timeout(Duration::from_secs(2)).expect("initial background refresh");
        receive.recv_timeout(Duration::from_secs(2)).expect("periodic background refresh");
        drop(worker);
        while receive.try_recv().is_ok() {}
        assert!(matches!(receive.recv_timeout(Duration::from_millis(40)), Err(mpsc::RecvTimeoutError::Disconnected)));
    }

    #[test]
    fn a_failed_tick_retries_and_shutdown_interrupts_a_long_interval() {
        let (send, receive) = mpsc::channel();
        let mut first = true;
        let worker = Worker::start_with(Duration::from_millis(20), move || {
            if first { first = false; panic!("failed probe pass"); }
            let _ = send.send(());
        }).unwrap();
        receive.recv_timeout(Duration::from_secs(2)).expect("refresh retries after failure");
        drop(worker);

        let (send, receive) = mpsc::channel();
        let worker = Worker::start_with(Duration::from_secs(300), move || { let _ = send.send(()); }).unwrap();
        receive.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        drop(worker);
        assert!(started.elapsed() < Duration::from_secs(1), "shutdown must wake the timer immediately");
    }
}
