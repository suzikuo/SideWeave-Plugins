//! Where Codex keeps its rollout logs, and how the date-partitioned tree is
//! pruned before anything is opened.
//!
//! Both layouts occur in the wild: `sessions/2026-09-23/rollout-*.jsonl` and
//! `sessions/2026/09/23/rollout-*.jsonl`. A chain of date-like directory names is
//! accumulated and collapsed into the coarsest span it can prove, so a subtree
//! that is provably outside [`DateFilter`] is skipped whole. That is what keeps
//! the first index pass bounded to the retention window instead of stat-ing
//! years of a 3.8 GB tree.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::UNIX_EPOCH;

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, OpenFlags};
use usage_core::{parse_ts_ms, DateFilter, FileKind, SourceFile};

/// Guards against an unexpected deep or symlinked tree inside `~/.codex`.
const MAX_DEPTH: usize = 6;

pub(crate) fn codex_home() -> Option<PathBuf> {
    if let Some(home) = usage_core::discovery_home_root() {
        return Some(home.join(".codex"));
    }
    if let Ok(v) = std::env::var("CODEX_HOME") {
        if !v.trim().is_empty() {
            return Some(PathBuf::from(v.trim()));
        }
    }
    usage_core::home_dir().map(|h| h.join(".codex"))
}

/// Codex homes visible to the plugin host. `CODEX_HOME` overrides the native
/// home; other WSL/Linux homes are scanned through configured external roots.
pub(crate) fn codex_homes() -> Vec<PathBuf> {
    if let Some(home) = usage_core::discovery_home_root() {
        return vec![home.join(".codex")];
    }
    let mut homes = Vec::new();
    if let Some(home) = codex_home() {
        homes.push(home);
    }

    let wsl_home = match std::env::var_os("CODEX_WSL_HOME") {
        Some(value) if value.to_string_lossy().trim().is_empty() => None,
        Some(value) => Some(PathBuf::from(value)),
        None => None,
    };
    if let Some(home) = wsl_home {
        if !homes.contains(&home) {
            homes.push(home);
        }
    }
    homes
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TitleStamp(Vec<(PathBuf, u64, u128)>);

#[derive(Clone)]
struct CachedTitles {
    stamp: TitleStamp,
    by_session: HashMap<String, SessionTitle>,
}

#[derive(Clone)]
struct SessionTitle {
    name: String,
    updated_at_ms: Option<i64>,
}

static SESSION_TITLES: OnceLock<RwLock<HashMap<PathBuf, CachedTitles>>> = OnceLock::new();

/// Refresh the title cache once per Codex metadata change. Codex keeps names in
/// an append-only index and, on newer versions, in its local threads database.
pub(crate) fn refresh_session_titles(home: &Path) {
    let home_key = home.to_path_buf();
    let stamp = title_stamp(home);
    let cache = SESSION_TITLES.get_or_init(|| RwLock::new(HashMap::new()));
    if cache.read().ok().and_then(|entries| entries.get(&home_key).map(|entry| entry.stamp == stamp)).unwrap_or(false) {
        return;
    }

    let mut by_session = HashMap::new();
    for db in [home.join("state_5.sqlite"), home.join("sqlite").join("state_5.sqlite")] {
        read_database_titles(&db, &mut by_session);
    }
    read_index_titles(&home.join("session_index.jsonl"), &mut by_session);

    if let Ok(mut entries) = cache.write() {
        entries.insert(home_key, CachedTitles { stamp, by_session });
    }
}

pub(crate) fn session_title_for_rollout(path: &Path, session: &str) -> Option<String> {
    let home = path
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "sessions"))?
        .parent()?;
    let cache = SESSION_TITLES.get()?.read().ok()?;
    cache.get(home)?.by_session.get(&session.to_ascii_lowercase()).map(|title| title.name.clone())
}

/// Use the session index's update timestamp as part of the rollout fingerprint.
/// Renaming a conversation then re-reads its rollout and refreshes stored event titles.
pub(crate) fn title_revision_for_rollout(path: &Path) -> Option<i64> {
    let home = path
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "sessions"))?
        .parent()?;
    let session = uuid_from_file_name(path)?;
    let cache = SESSION_TITLES.get()?.read().ok()?;
    cache.get(home)?.by_session.get(&session)?.updated_at_ms
}

fn title_stamp(home: &Path) -> TitleStamp {
    let paths = [
        home.join("session_index.jsonl"),
        home.join("state_5.sqlite"),
        home.join("state_5.sqlite-wal"),
        home.join("sqlite").join("state_5.sqlite"),
        home.join("sqlite").join("state_5.sqlite-wal"),
    ];
    let mut stamp = Vec::new();
    for path in paths {
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        let modified_ns = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        stamp.push((path, meta.len(), modified_ns));
    }
    TitleStamp(stamp)
}

fn read_index_titles(path: &Path, titles: &mut HashMap<String, SessionTitle>) {
    let Ok(file) = std::fs::File::open(path) else { return };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let Some(id) = entry.get("id").and_then(serde_json::Value::as_str) else { continue };
        let Some(name) = entry.get("thread_name").and_then(serde_json::Value::as_str) else { continue };
        if let Some(name) = clean_title(name) {
            titles.insert(
                id.to_ascii_lowercase(),
                SessionTitle {
                    name,
                    updated_at_ms: entry.get("updated_at").and_then(parse_timestamp_value),
                },
            );
        }
    }
}

fn read_database_titles(path: &Path, titles: &mut HashMap<String, SessionTitle>) {
    let Ok(db) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else { return };
    let _ = db.busy_timeout(std::time::Duration::from_millis(250));
    let columns: HashSet<String> = match db.prepare("PRAGMA table_info(threads)") {
        Ok(mut stmt) => match stmt.query_map([], |row| row.get::<_, String>(1)) {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => return,
        },
        Err(_) => return,
    };
    let name_col = columns.contains("name").then_some("name");
    let title_col = columns.contains("title").then_some("title");
    if !columns.contains("id") || (name_col.is_none() && title_col.is_none()) {
        return;
    }
    let title_expr = match (name_col, title_col) {
        (Some(name), Some(title)) => format!("COALESCE(NULLIF(trim({name}), ''), NULLIF(trim({title}), ''))"),
        (Some(name), None) => format!("NULLIF(trim({name}), '')"),
        (None, Some(title)) => format!("NULLIF(trim({title}), '')"),
        (None, None) => return,
    };
    let sql = format!("SELECT id, {title_expr} FROM threads WHERE {title_expr} IS NOT NULL");
    let Ok(mut stmt) = db.prepare(&sql) else { return };
    let Ok(rows) = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))) else { return };
    for row in rows.flatten() {
        if let (id, Some(name)) = (row.0, clean_title(&row.1)) {
            titles.entry(id.to_ascii_lowercase()).or_insert(SessionTitle { name, updated_at_ms: None });
        }
    }
}

fn parse_timestamp_value(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::String(text) => parse_ts_ms(text),
        serde_json::Value::Number(number) => parse_ts_ms(&number.to_string()),
        _ => None,
    }
}

fn clean_title(value: &str) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = normalized.chars().take(240).collect();
    (!trimmed.is_empty()).then_some(trimmed)
}

pub(crate) fn sessions_root(home: &Path) -> PathBuf {
    home.join("sessions")
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `rollout-2026-09-23T19-25-11-01a0ce03-33ce-7172-848c-7199f1589e2e` and the
/// `_`-separated spelling both end in the 36-char session uuid, which is the
/// fallback session id when a file has no `session_meta`.
pub(crate) fn uuid_from_file_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let tail = match stem.rsplit_once('_') {
        Some((_, t)) if t.len() == 36 => t,
        _ if stem.len() >= 36 => &stem[stem.len() - 36..],
        _ => return None,
    };
    let is_uuid = tail.len() == 36
        && tail.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        });
    is_uuid.then(|| tail.to_ascii_lowercase())
}

fn local_midnight(y: i32, m: u32, d: u32) -> Option<i64> {
    let naive = NaiveDate::from_ymd_opt(y, m, d)?.and_hms_opt(0, 0, 0)?;
    // A DST-skipped midnight returns `None`: no span, hence no pruning, which is
    // the safe direction (never drop real data over a boundary ambiguity).
    Local.from_local_datetime(&naive).single().map(|dt| dt.timestamp_millis())
}

/// A directory name that is a whole date, one numeric date component, or noise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DatePart {
    Day(NaiveDate),
    Num(u32),
    NotDate,
}

fn date_part(name: &str) -> DatePart {
    let n = name.trim();
    if let Ok(d) = NaiveDate::parse_from_str(n, "%Y-%m-%d") {
        return DatePart::Day(d);
    }
    if !n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(v) = n.parse::<u32>() {
            return DatePart::Num(v);
        }
    }
    DatePart::NotDate
}

fn day_span(y: i32, m: u32, d: u32) -> Option<(i64, i64)> {
    let start = local_midnight(y, m, d)?;
    let end = match (m, d) {
        (12, 31) => local_midnight(y + 1, 1, 1)?,
        (_, 31) => local_midnight(y, m + 1, 1)?,
        (_, d) => local_midnight(y, m, d + 1)?,
    };
    Some((start, end))
}

/// Inclusive-start / exclusive-end local span proved by a chain of directory
/// names, or `None` when the chain proves no date (then nothing gets pruned).
fn span_ms(parts: &[DatePart]) -> Option<(i64, i64)> {
    if let Some(DatePart::Day(d)) = parts.first() {
        return day_span(d.year(), d.month(), d.day());
    }
    let nums: Vec<u32> = parts
        .iter()
        .map(|p| match p {
            DatePart::Num(v) => Some(*v),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let year = |y: u32| (1990..=2200).contains(&y);
    match nums.as_slice() {
        [y] if year(*y) => Some((local_midnight(*y as i32, 1, 1)?, local_midnight(*y as i32 + 1, 1, 1)?)),
        [y, mo] if year(*y) && (1..=12).contains(mo) => Some((
            local_midnight(*y as i32, *mo, 1)?,
            if *mo == 12 { local_midnight(*y as i32 + 1, 1, 1)? } else { local_midnight(*y as i32, mo + 1, 1)? },
        )),
        [y, mo, d] if year(*y) && (1..=12).contains(mo) && (1..=31).contains(d) => {
            day_span(*y as i32, *mo, *d)
        }
        _ => None,
    }
}

fn overlaps(filter: &DateFilter, start_ms: i64, end_ms: i64) -> bool {
    filter.since_ms.is_none_or(|s| end_ms > s) && filter.until_ms.is_none_or(|u| start_ms <= u)
}

/// `false` only when the whole subtree is provably outside `filter`.
fn dir_in_filter(filter: &DateFilter, parts: &[DatePart]) -> bool {
    match span_ms(parts) {
        Some((s, e)) => overlaps(filter, s, e),
        None => true,
    }
}

fn is_rollout(name: &str) -> bool {
    name.ends_with(".jsonl")
}

fn walk(dir: &Path, parts: &[DatePart], filter: &DateFilter, depth: usize, out: &mut Vec<SourceFile>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            let part = date_part(name);
            // Noise in the chain breaks the date path: restart the accumulation
            // so `sessions/backup/2026-09-23` is still understood as a day.
            let mut child: Vec<DatePart> = if part == DatePart::NotDate {
                Vec::new()
            } else if matches!(part, DatePart::Day(_)) {
                vec![part]
            } else {
                let mut v = parts.to_vec();
                v.push(part);
                v
            };
            if child.len() > 3 {
                child.truncate(3);
            }
            if !dir_in_filter(filter, &child) {
                continue;
            }
            walk(&path, &child, filter, depth + 1, out);
            continue;
        }
        if !is_rollout(name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        out.push(SourceFile {
            kind: FileKind::Jsonl,
            size: meta.len(),
            mtime_ms: mtime_ms(&meta),
            path,
        });
    }
}

pub(crate) fn list_rollouts(root: &Path, filter: &DateFilter) -> Vec<SourceFile> {
    let mut out = Vec::new();
    walk(root, &[], filter, 0, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Sub-directories in ascending name order, i.e. oldest date first.
fn child_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<(String, PathBuf)> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let name = p.file_name()?.to_str()?.to_string();
                e.file_type().ok().filter(|ft| ft.is_dir()).map(|_| (name, p))
            })
            .collect(),
        Err(_) => return Vec::new(),
    };
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    dirs.into_iter().map(|(_, p)| p).collect()
}

/// Newest rollout file that can actually be opened. Visits a bounded number of
/// directory entries: `probe` runs at every startup and must not walk 3.8 GB.
pub(crate) fn first_rollout(root: &Path) -> Option<PathBuf> {
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if visited > 400 || depth > MAX_DEPTH {
            continue;
        }
        visited += 1;
        // Pushed oldest-first so the LIFO pop reaches the newest dates first.
        // `read_dir`'s `file_type` does not follow symlinks, so no cycle is possible.
        for child in child_dirs(&dir) {
            stack.push((child, depth + 1));
        }
        let mut files: Vec<String> = match std::fs::read_dir(&dir) {
            Ok(rd) => rd
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let n = p.file_name()?.to_str()?.to_string();
                    (is_rollout(&n) && e.file_type().map(|ft| ft.is_file()).unwrap_or(false)).then_some(n)
                })
                .collect(),
            Err(_) => continue,
        };
        files.sort();
        if let Some(name) = files.into_iter().rev().find(|n| {
            let p = dir.join(n);
            std::fs::File::open(&p).is_ok()
        }) {
            return Some(dir.join(name));
        }
    }
    None
}

/// `cli_version` off the first `session_meta` line, used as the sources-list hint.
pub(crate) fn cli_version_hint(path: &Path) -> Option<String> {
    use std::io::{BufRead, Read};
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file.take(1024 * 1024));
    for line in reader.lines().map_while(Result::ok).take(8) {
        if !line.contains("session_meta") {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&line).ok()?;
        if let Some(ver) = v.get("payload").and_then(|p| p.get("cli_version")).and_then(|c| c.as_str()) {
            return Some(ver.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(names: &[&str]) -> Vec<DatePart> {
        names.iter().map(|n| date_part(n)).collect()
    }

    #[test]
    fn flat_and_nested_date_dirs_both_prove_a_day() {
        let flat = span_ms(&parts(&["2026-09-23"])).expect("flat day");
        let nested = span_ms(&parts(&["2026", "09", "23"])).expect("nested day");
        assert_eq!(flat, nested, "both spellings mean the same local day");
        assert_eq!(nested.1 - nested.0, 86_400_000);
    }

    #[test]
    fn year_and_month_chains_prune_whole_subtrees() {
        let until = DateFilter::new(None, Some(local_midnight(2026, 8, 31).unwrap() + 1_000));
        let since = DateFilter::new(Some(local_midnight(2026, 9, 20).unwrap()), None);
        assert!(!dir_in_filter(&until, &parts(&["2026", "09"])), "September is out of range");
        assert!(dir_in_filter(&until, &parts(&["2026", "08"])));
        assert!(dir_in_filter(&until, &parts(&["2026"])), "2026 still contains August");
        assert!(dir_in_filter(&until, &parts(&["2025"])), "an older year is inside `until`");
        assert!(!dir_in_filter(&since, &parts(&["2025"])), "and outside `since`");
        assert!(!dir_in_filter(&since, &parts(&["2024"])));
    }

    #[test]
    fn non_date_dirs_are_never_pruned() {
        let f = DateFilter::new(Some(local_midnight(2030, 1, 1).unwrap()), None);
        assert!(dir_in_filter(&f, &parts(&["archived"])));
        assert!(dir_in_filter(&f, &[]));
        assert!(span_ms(&parts(&["2026", "13"])).is_none(), "month 13 proves nothing");
    }

    #[test]
    fn uuid_survives_both_file_name_spellings() {
        let u = "01a0ce03-33ce-7172-848c-7199f1589e2e";
        assert_eq!(uuid_from_file_name(Path::new(&format!("rollout-2026-09-23T19-25-11-{u}"))).as_deref(), Some(u));
        assert_eq!(uuid_from_file_name(Path::new(&format!("rollout-2026-09-23T19-25-11_{u}"))).as_deref(), Some(u));
        assert_eq!(uuid_from_file_name(Path::new("notes")), None);
        assert_eq!(uuid_from_file_name(Path::new("rollout-not-a-uuid")), None);
    }
}
