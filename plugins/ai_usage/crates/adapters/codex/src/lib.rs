//! Codex adapter (CLI + desktop app).
//!
//! Source: Windows `~/.codex` and WSL `~/.codex` session rollouts, honouring
//! `CODEX_HOME` and `CODEX_WSL_HOME`.
//! `event_msg.token_count` carries `info.last_token_usage` (use that, never the
//! cumulative `total_token_usage`) and `rate_limits`.
//!
//! Owned by the `codex-opencode-adapters` workstream; public surface is frozen.

mod doctor;
mod loader;
mod parser;
mod paths;
mod ttft;

use usage_core::{
    DateFilter, DetectedSource, Error, ReadCursor, ReadOutcome, Semantics, SourceAdapter, SourceFile,
};

pub const TOOL_ID: &str = "codex";

/// The replay audit surface: `tokenme codex-doctor` and the scan log read this.
pub use doctor::sessions as replay;
pub use ttft::{read_ttft, CodexTtftCursor, CodexTtftRead, CodexTtftSample};

#[derive(Debug, Default, Clone, Copy)]
pub struct CodexAdapter;

impl SourceAdapter for CodexAdapter {
    fn id(&self) -> &'static str {
        TOOL_ID
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            usage_form: usage_core::UsageForm::PerCall,
            meter: usage_core::Meter::Tokens,
            // The model lives on `turn_context`, a different record than the
            // token counts, and must be resolved inside the adapter.
            model_attr: usage_core::ModelAttr::Joined,
            dedupes_by_id: false,
            reports_quota: true,
        }
    }

    fn probe(&self) -> Option<DetectedSource> {
        let mut roots = Vec::new();
        let mut hint = None;
        for home in paths::codex_homes() {
            let root = paths::sessions_root(&home);
            if let Some(first) = paths::first_rollout(&root) {
                if hint.is_none() {
                    hint = paths::cli_version_hint(&first).map(|v| format!("cli {v}"));
                }
                roots.push(root);
            }
        }
        if roots.is_empty() {
            return None;
        }
        Some(DetectedSource {
            id: TOOL_ID.to_string(),
            display: self.display_name().to_string(),
            roots,
            hint,
        })
    }

    fn discover(&self, filter: &DateFilter) -> Vec<SourceFile> {
        let mut files = Vec::new();
        for home in paths::codex_homes() {
            paths::refresh_session_titles(&home);
            let root = paths::sessions_root(&home);
            let mut home_files = paths::list_rollouts(&root, filter);
            for file in &mut home_files {
                // A newer session-index timestamp moves this fingerprint backwards,
                // which tells the indexer to reread just this rollout and refresh titles.
                if let Some(updated_at_ms) = paths::title_revision_for_rollout(&file.path) {
                    file.mtime_ms = updated_at_ms.max(1).saturating_neg();
                }
            }
            files.extend(home_files);
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        files
    }

    fn read(&self, file: &SourceFile, cursor: ReadCursor) -> Result<ReadOutcome, Error> {
        // Never `Err`: a log Codex is mid-rotation on must not abort the pass, and
        // there is nothing a caller could do about it.
        Ok(loader::read_rollout(&file.path, cursor, &file.key()))
    }
}
