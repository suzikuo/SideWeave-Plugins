//! Sequential JSONL reader: byte cursor, tolerant parsing, and Codex's model
//! attribution.
//!
//! `ReadCursor.0` is a byte offset. Only whole lines are consumed, so a file the
//! app is still writing never yields a half record; the trailing partial line is
//! picked up on the next pass.

use std::io::{BufRead, BufReader, Read, Seek as _, SeekFrom};
use std::collections::HashMap;
use std::path::Path;

use usage_core::{Meter, QuotaSample, ReadCursor, ReadOutcome, TokenCounts, UsageEvent};

use crate::parser::{self, Record};
use crate::paths;
use crate::TOOL_ID;

/// Per-file attribution state, seeded by `session_meta` and updated per turn.
#[derive(Debug, Default)]
struct Attribution {
    session: String,
    account_id: Option<String>,
    project: Option<String>,
    meta_model: Option<String>,
    turn_model: Option<String>,
    session_title: Option<String>,
    current_turn_id: Option<String>,
    task_started_ms: Option<i64>,
    completed_duration: Option<(String, i64)>,
    /// Identity of the previous `token_count`: the session accumulator plus the
    /// exact usage. Codex re-reports some calls verbatim (measured 2.03% of all
    /// tokens on this machine), and only a byte-identical repeat is one — 294
    /// real records leave the accumulator flat while reporting a different call,
    /// so a looser test would silently drop them.
    previous_usage: Option<UsageSignature>,
}

type UsageSignature = (f64, TokenCounts, f64);

/// The billable payload of one `token_count`.
struct Usage {
    ts_ms: i64,
    turn_id: Option<String>,
    duration_ms: Option<i64>,
    counts: TokenCounts,
    quota: Option<QuotaSample>,
}

impl Attribution {
    fn new(path: &Path) -> Self {
        Self {
            session: paths::uuid_from_file_name(path).unwrap_or_else(|| path.to_string_lossy().into_owned()),
            ..Self::default()
        }
    }

    /// `token_count` carries no model, so the most recent `turn_context.model`
    /// wins; if none precedes it (sub-agent rollouts, a session resumed mid-file)
    /// fall back to any model on `session_meta`, else `None`.
    fn model(&self) -> Option<String> {
        self.turn_model.clone().or_else(|| self.meta_model.clone())
    }

    /// Applies one line's record and reports the usage it carried, if any.
    fn apply(&mut self, raw: &str) -> Option<Usage> {
        // Pre-gate on substrings: most bytes of a rollout are `response_item`
        // payloads, and parsing them just to learn they carry no usage is the one
        // thing a multi-GB log cannot survive.
        if !parser::worth_parsing(raw) {
            return None;
        }
        match parser::parse_record(raw)? {
            Record::SessionMeta { session, account_id, cwd, model, title } => {
                // Codex can repeat the SAME session header just after a
                // turn_context, without a model. It must not erase that turn's
                // attribution. Only an explicitly different session is a boundary.
                if session.as_deref().is_some_and(|id| id != self.session) {
                    *self = Self { session: self.session.clone(), ..Self::default() };
                }
                if let Some(s) = session {
                    self.session = s;
                }
                if account_id.is_some() { self.account_id = account_id; }
                if cwd.is_some() {
                    self.project = cwd;
                }
                if model.is_some() {
                    self.meta_model = model;
                }
                if title.is_some() { self.session_title = title; }
                None
            }
            Record::TurnContext { turn_id, model, cwd } => {
                if turn_id.is_some() {
                    self.current_turn_id = turn_id;
                }
                if model.is_some() {
                    self.turn_model = model;
                }
                if self.project.is_none() {
                    self.project = cwd;
                }
                None
            }
            Record::TaskStarted { turn_id, ts_ms } => {
                if turn_id.is_some() {
                    self.current_turn_id = turn_id;
                }
                self.task_started_ms = Some(ts_ms);
                None
            }
            Record::TaskComplete { turn_id, duration_ms } => {
                if let (Some(turn_id), Some(duration_ms)) =
                    (turn_id.or_else(|| self.current_turn_id.clone()), duration_ms)
                {
                    self.completed_duration = Some((turn_id, duration_ms));
                }
                None
            }
            Record::Usage { ts_ms, counts, reported_total, quota, accumulator } => {
                if counts.is_zero() {
                    // Context-size echo: emitting it would inflate the request count
                    // for a turn that billed nothing.
                    return None;
                }
                let signature = (accumulator, counts, reported_total);
                if accumulator > 0.0 && self.previous_usage == Some(signature) {
                    return None;
                }
                self.previous_usage = Some(signature);
                let duration_ms = self
                    .task_started_ms
                    .and_then(|started| ts_ms.checked_sub(started))
                    .filter(|duration| (0..=86_400_000).contains(duration));
                Some(Usage {
                    ts_ms,
                    turn_id: self.current_turn_id.clone(),
                    duration_ms,
                    counts,
                    quota,
                })
            }
        }
    }

    fn event(&self, usage: Usage, source_key: &str, path: &Path, line_offset: u64) -> UsageEvent {
        let mut ev = UsageEvent::new(TOOL_ID, usage.ts_ms, self.session.clone());
        ev.session_title = paths::session_title_for_rollout(path, &self.session).or_else(|| self.session_title.clone());
        ev.account_id = self.account_id.clone();
        ev.duration_ms = usage.duration_ms;
        ev.turn_id = usage.turn_id.clone();
        ev.project = self.project.clone();
        ev.model = self.model();
        ev.meter = Meter::Tokens;
        ev.counts = usage.counts;
        ev.quota = usage.quota;
        // A rollout line offset is stable across incremental reads. This also
        // lets a later task_complete update the matching token_count duration.
        ev.dedupe_key = Some(format!("codex#{}#{line_offset}", self.session));
        ev.source = source_key.to_string();
        ev
    }
}

/// Reads `[cursor, last_complete_line)` from `path`.
pub(crate) fn read_rollout(path: &Path, cursor: ReadCursor, source_key: &str) -> ReadOutcome {
    try_read(path, cursor, source_key).unwrap_or_else(|_| ReadOutcome {
        // Unreadable file: hand back the position we were given. Resetting to 0
        // would make the next pass re-ingest the whole file, and this adapter has
        // no `dedupe_key` to absorb the duplicates.
        events: Vec::new(),
        cursor,
    })
}

fn try_read(path: &Path, cursor: ReadCursor, source_key: &str) -> std::io::Result<ReadOutcome> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    // Cursor past EOF means the log was rotated or truncated: re-read from the top
    // and let the indexer purge this `source` key's stale events.
    let start = if cursor.0 > len { 0 } else { cursor.0 };
    if start == len {
        // Nothing appended: skip even the attribution probe below.
        return Ok(ReadOutcome { events: Vec::new(), cursor: ReadCursor(start) });
    }
    let mut state = Attribution::new(path);
    if start > 0 {
        // A resumed pass has not read the record that names the current turn's
        // model, so recover it from the bytes just behind the cursor.
        let (turn_id, model, cwd) = recover_turn_context(&mut file, start).unwrap_or((None, None, None));
        state.turn_model = model;
        state.project = cwd;
        state.current_turn_id = turn_id;
        state.task_started_ms = recover_task_started(&mut file, start);
        state.account_id = recover_account_id(&mut file);
    }
    let mut events = Vec::new();
    let mut turn_events: HashMap<String, Vec<usize>> = HashMap::new();
    let mut pos = start;
    if start < len {
        file.seek(SeekFrom::Start(start))?;
        let mut reader = BufReader::with_capacity(128 * 1024, (&mut file).take(len - start));
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        loop {
            buf.clear();
            let n = match reader.read_until(b'\n', &mut buf) {
                Ok(n) => n,
                // Torn chunk: stop here and resume at the last good line.
                Err(_) => break,
            };
            if n == 0 || !buf.ends_with(b"\n") {
                // EOF, or a mid-write partial line left for the next pass.
                break;
            }
            let line_start = pos;
            pos = pos.saturating_add(n as u64);
            if let Ok(line) = std::str::from_utf8(&buf) {
                if let Some(usage) = state.apply(line) {
                    let turn_id = usage.turn_id.clone();
                    let event = state.event(usage, source_key, path, line_start);
                    if let Some(turn_id) = turn_id {
                        turn_events.entry(turn_id).or_default().push(events.len());
                    }
                    events.push(event);
                }
                if let Some((turn_id, duration_ms)) = state.completed_duration.take() {
                    if let Some(indices) = turn_events.get(&turn_id) {
                        for index in indices {
                            events[*index].duration_ms = Some(duration_ms);
                        }
                    }
                    if let Some(task_start) = find_task_started(path, line_start, &turn_id) {
                        if task_start < start {
                            events.extend(read_duration_patches(
                                path,
                                task_start,
                                start,
                                &turn_id,
                                duration_ms,
                                state.account_id.clone(),
                                source_key,
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(ReadOutcome { events, cursor: ReadCursor(pos) })
}

/// Bytes read per step, and the total budget, of the backwards attribution scan.
const SEED_CHUNK: u64 = 256 * 1024;
/// A turn whose records exceed the budget loses its model label rather than
/// making every pass re-read megabytes.
const SEED_CAP: u64 = 8 * 1024 * 1024;
const MARKER: &[u8] = b"turn_context";
const START_MARKER: &[u8] = b"task_started";

/// Finds the current turn's start line near the cursor. A `task_complete` can
/// arrive in a later append after its token_count was already indexed.
fn find_task_started(path: &Path, before: u64, target_turn: &str) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    let base = before.saturating_sub(SEED_CAP);
    let mut bytes = vec![0u8; (before - base) as usize];
    file.seek(SeekFrom::Start(base)).ok()?;
    file.read_exact(&mut bytes).ok()?;

    let mut offset = base;
    let mut found = None;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let line_offset = offset;
        offset = offset.saturating_add(line.len() as u64);
        // The first line may start before the bounded scan window.
        if base > 0 && line_offset == base {
            continue;
        }
        let Ok(raw) = std::str::from_utf8(line) else { continue };
        if let Some(Record::TaskStarted { turn_id, .. }) = parser::parse_record(raw) {
            if turn_id.as_deref().is_none_or(|id| id == target_turn) {
                found = Some(line_offset);
            }
        }
    }
    found
}

/// Replays only the completed turn's pre-cursor usage lines. Stable line keys
/// make these duration-only updates idempotent in the usage index.
fn read_duration_patches(
    path: &Path,
    from: u64,
    until: u64,
    target_turn: &str,
    duration_ms: i64,
    account_id: Option<String>,
    source_key: &str,
) -> Vec<UsageEvent> {
    if from >= until {
        return Vec::new();
    }
    let Ok(mut file) = std::fs::File::open(path) else { return Vec::new() };
    if file.seek(SeekFrom::Start(from)).is_err() {
        return Vec::new();
    }
    let mut reader = BufReader::with_capacity(64 * 1024, (&mut file).take(until - from));
    let mut state = Attribution::new(path);
    state.current_turn_id = Some(target_turn.to_string());
    state.account_id = account_id;
    let mut events = Vec::new();
    let mut offset = from;
    let mut buf = Vec::with_capacity(16 * 1024);
    loop {
        buf.clear();
        let Ok(size) = reader.read_until(b'\n', &mut buf) else { break };
        if size == 0 || !buf.ends_with(b"\n") {
            break;
        }
        let line_offset = offset;
        offset = offset.saturating_add(size as u64);
        let Ok(raw) = std::str::from_utf8(&buf) else { continue };
        if let Some(usage) = state.apply(raw) {
            if usage.turn_id.as_deref() == Some(target_turn) {
                let mut event = state.event(usage, source_key, path, line_offset);
                event.duration_ms = Some(duration_ms);
                events.push(event);
            }
        }
    }
    events
}

/// The newest `turn_context` line before `cursor`, as (turn id, model, cwd).
///
/// Reads backwards in chunks and stops at the first hit, so a steady-state pass
/// costs one chunk of I/O (on real logs that distance is ~90 KiB median) instead
/// of a re-read of the file. Scan failures are not fatal: an unlabelled model is
/// a pricing gap, whereas dropping the events would lose usage outright.
fn recover_turn_context(
    file: &mut std::fs::File,
    cursor: u64,
) -> Option<(Option<String>, Option<String>, Option<String>)> {
    let mut base = cursor;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let start = base.saturating_sub(SEED_CHUNK);
        let mut chunk = vec![0u8; (base - start) as usize];
        if file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut chunk).is_err() {
            return None;
        }
        chunk.extend_from_slice(&buf);
        buf = chunk;
        base = start;
        match scan_for_turn_context(&buf, base == 0) {
            Scan::Found(hit) => return Some(hit),
            Scan::Truncated | Scan::NotFound if base == 0 || cursor - base >= SEED_CAP => return None,
            Scan::Truncated | Scan::NotFound => {}
        }
    }
}

fn recover_task_started(file: &mut std::fs::File, cursor: u64) -> Option<i64> {
    let mut base = cursor;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let start = base.saturating_sub(SEED_CHUNK);
        let mut chunk = vec![0u8; (base - start) as usize];
        if file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut chunk).is_err() {
            return None;
        }
        chunk.extend_from_slice(&buf);
        buf = chunk;
        base = start;
        match scan_for_task_started(&buf, base == 0) {
            TaskScan::Found(ts_ms) => return Some(ts_ms),
            TaskScan::Truncated | TaskScan::NotFound if base == 0 || cursor - base >= SEED_CAP => return None,
            TaskScan::Truncated | TaskScan::NotFound => {}
        }
    }
}

/// Recover the rollout's creator account when an incremental read resumes
/// after the initial `session_meta` record. Codex writes this near the file
/// header, so the probe stays bounded even for multi-megabyte conversations.
fn recover_account_id(file: &mut std::fs::File) -> Option<String> {
    const HEADER_PROBE: usize = 64 * 1024;
    let len = file.metadata().ok()?.len().min(HEADER_PROBE as u64) as usize;
    let mut bytes = vec![0u8; len];
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(raw) = std::str::from_utf8(line) else { continue };
        if let Some(Record::SessionMeta { account_id, .. }) = parser::parse_record(raw) {
            if account_id.is_some() {
                return account_id;
            }
        }
    }
    None
}

enum Scan {
    Found((Option<String>, Option<String>, Option<String>)),
    /// No usable record in view; more bytes might hold one.
    NotFound,
    /// The leftmost candidate's line starts before the buffer: read more bytes.
    Truncated,
}

fn scan_for_turn_context(buf: &[u8], at_bof: bool) -> Scan {
    let mut limit = buf.len();
    loop {
        let Some(rel) = rfind_marker(&buf[..limit]) else { return Scan::NotFound };
        let line_start = match buf[..rel].iter().rposition(|b| *b == b'\n') {
            Some(i) => i + 1,
            None if !at_bof => return Scan::Truncated,
            None => 0,
        };
        let Some(nl) = buf[rel..].iter().position(|b| *b == b'\n') else {
            // The candidate runs past the cursor: that is the mid-write tail, so
            // discard it and keep looking further left.
            limit = line_start;
            continue;
        };
        let line_end = rel + nl;
        if let Ok(line) = std::str::from_utf8(&buf[line_start..line_end]) {
            if let Some(Record::TurnContext { turn_id, model, cwd }) = parser::parse_record(line) {
                return Scan::Found((turn_id, model, cwd));
            }
        }
        // The marker appeared inside some other record's text: step left of it.
        limit = line_start;
    }
}

fn rfind_marker(hay: &[u8]) -> Option<usize> {
    if hay.len() < MARKER.len() {
        return None;
    }
    (0..=hay.len() - MARKER.len()).rev().find(|&i| &hay[i..i + MARKER.len()] == MARKER)
}

enum TaskScan {
    Found(i64),
    NotFound,
    Truncated,
}

fn scan_for_task_started(buf: &[u8], at_bof: bool) -> TaskScan {
    let mut limit = buf.len();
    loop {
        let Some(rel) = rfind_bytes(&buf[..limit], START_MARKER) else { return TaskScan::NotFound };
        let line_start = match buf[..rel].iter().rposition(|b| *b == b'\n') {
            Some(i) => i + 1,
            None if !at_bof => return TaskScan::Truncated,
            None => 0,
        };
        let Some(nl) = buf[rel..].iter().position(|b| *b == b'\n') else {
            limit = line_start;
            continue;
        };
        let line_end = rel + nl;
        if let Ok(line) = std::str::from_utf8(&buf[line_start..line_end]) {
            if let Some(Record::TaskStarted { ts_ms, .. }) = parser::parse_record(line) {
                return TaskScan::Found(ts_ms);
            }
        }
        limit = line_start;
    }
}

fn rfind_bytes(hay: &[u8], marker: &[u8]) -> Option<usize> {
    if hay.len() < marker.len() {
        return None;
    }
    (0..=hay.len() - marker.len()).rev().find(|&i| &hay[i..i + marker.len()] == marker)
}



#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn token_count(total: f64, last_total: f64) -> String {
        json!({
            "timestamp": "2026-09-23T10:00:00.000Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "last_token_usage": {
                        "input_tokens": last_total, "cached_input_tokens": 0.0,
                        "output_tokens": 0.0, "total_tokens": last_total
                    },
                    "total_token_usage": { "total_tokens": total }
                }
            }
        })
        .to_string()
    }

    fn meta(session: &str) -> String {
        json!({ "type": "session_meta", "payload": { "session_id": session } }).to_string()
    }

    fn new_state() -> Attribution {
        Attribution::new(&PathBuf::from("/tmp/rollout-x.jsonl"))
    }

    #[test]
    fn a_verbatim_re_report_is_dropped_once() {
        let mut a = new_state();
        assert!(a.apply(&meta("s1")).is_none());
        assert!(a.apply(&token_count(1000.0, 1000.0)).is_some(), "first call bills");
        assert!(a.apply(&token_count(1000.0, 1000.0)).is_none(), "identical repeat re-bills");
        assert!(a.apply(&token_count(2000.0, 1000.0)).is_some(), "accumulator moved, so a new call");
    }

    #[test]
    fn a_flat_accumulator_with_different_usage_is_still_a_call() {
        // Real shape from the fixtures: `total_token_usage` parked at one value
        // while three distinct calls report different usage.
        let mut a = new_state();
        assert!(a.apply(&token_count(930_000.0, 15_687.0)).is_some());
        assert!(a.apply(&token_count(930_000.0, 19_847.0)).is_some());
        assert!(a.apply(&token_count(930_000.0, 8_402.0)).is_some());
    }

    #[test]
    fn a_new_session_boundary_clears_the_repeat_memory() {
        let mut a = new_state();
        assert!(a.apply(&token_count(500.0, 500.0)).is_some());
        a.apply(&meta("s2"));
        assert!(a.apply(&token_count(500.0, 500.0)).is_some());
    }

    #[test]
    fn repeated_meta_keeps_dedupe_and_new_session_clears_all_attribution() {
        let mut state = new_state();
        state.apply(&meta("s1"));
        state.apply(r#"{"type":"session_meta","payload":{"id":"s1","model":"meta-model","creator_account_id":"account-a","title":"Title"}}"#);
        state.apply(r#"{"type":"turn_context","payload":{"turn_id":"turn-a","model":"gpt-6-luna"}}"#);
        assert!(state.apply(&token_count(500.0, 500.0)).is_some());
        state.apply(&meta("s1"));
        assert_eq!(state.model().as_deref(), Some("gpt-6-luna"));
        assert_eq!(state.account_id.as_deref(), Some("account-a"));
        assert_eq!(state.current_turn_id.as_deref(), Some("turn-a"));
        assert_eq!(state.session_title.as_deref(), Some("Title"));
        assert!(state.apply(&token_count(500.0, 500.0)).is_none(), "repeated metadata must not bill repeated usage");
        state.apply(&meta("s2"));
        assert_eq!(state.model(), None, "a different session must not inherit either model");
        assert_eq!(state.account_id, None);
        assert_eq!(state.current_turn_id, None);
        assert_eq!(state.session_title, None);
        assert!(state.apply(&token_count(500.0, 500.0)).is_some());
    }

    #[test]
    fn a_record_without_an_accumulator_is_never_dropped() {
        let mut a = new_state();
        let bare = json!({
            "timestamp": "2026-09-23T10:00:00.000Z", "type": "event_msg",
            "payload": { "type": "token_count", "info": { "last_token_usage": {
                "input_tokens": 75.0, "cached_input_tokens": 0.0,
                "output_tokens": 0.0, "total_tokens": 75.0 } } }
        })
        .to_string();
        assert!(a.apply(&bare).is_some());
        assert!(a.apply(&bare).is_some(), "no accumulator to compare against");
    }
}
