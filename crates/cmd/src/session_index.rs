// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! `rara session-index <subcommand>` — the rescue toolbox for the
//! SQLite-backed session index introduced in issue #2025.
//!
//! The only operator-facing command today is `rebuild`, which scans the
//! on-disk tape for one (or all) sessions and overwrites the
//! derived-state row(s) in `sessions`. Use it after a crash, or after
//! manually editing a tape file out-of-band, to bring the index back in
//! sync with reality.

use std::sync::Arc;

use clap::{Args, Subcommand};
use rara_kernel::{memory::TapeService, session::SessionKey};
use rara_sessions::sqlite_index::{ReconcileTape, SqliteSessionIndex, TapeReport};
use snafu::{ResultExt, Whatever, whatever};

#[derive(Debug, Clone, Args)]
#[command(about = "Maintain the SQLite-backed session index")]
pub struct SessionIndexCmd {
    #[command(subcommand)]
    pub action: SessionIndexAction,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SessionIndexAction {
    /// Rebuild derived-state rows from on-disk tapes.
    Rebuild(RebuildArgs),
}

#[derive(Debug, Clone, Args)]
pub struct RebuildArgs {
    /// Rebuild only this session. Without `--key`, every session in the
    /// index is rebuilt.
    #[arg(long)]
    pub key: Option<String>,
}

impl SessionIndexCmd {
    pub async fn run(self) -> Result<(), Whatever> {
        match self.action {
            SessionIndexAction::Rebuild(args) => rebuild(args).await,
        }
    }
}

async fn rebuild(args: RebuildArgs) -> Result<(), Whatever> {
    let config = rara_app::AppConfig::new().whatever_context("Failed to load config")?;
    let pools = rara_app::open_pools_for_cli(&config)
        .await
        .whatever_context("Failed to open SQLite pools")?;

    let index = Arc::new(SqliteSessionIndex::new(pools.clone()));

    let workspace_path = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let store = rara_kernel::memory::FileTapeStore::new(rara_paths::memory_dir(), &workspace_path)
        .await
        .whatever_context("Failed to initialize FileTapeStore")?;
    let tape = TapeService::new(store);
    let reconciler = TapeReconciler { tape };

    match args.key {
        Some(raw) => {
            let key = SessionKey::try_from_raw(&raw).map_err(|e| {
                snafu::FromString::without_source(format!("invalid session key '{raw}': {e}"))
            })?;
            let Some(report) = reconciler.read_tape(&key).await else {
                whatever!("no on-disk tape found for session {raw}");
            };
            index
                .rebuild_session_with_report(&key, &report)
                .await
                .whatever_context("rebuild failed")?;
            println!("rebuilt session {raw}");
        }
        None => {
            let repaired = index
                .reconcile_all(reconciler)
                .await
                .whatever_context("reconcile_all failed")?;
            println!("rebuild complete: {repaired} session(s) repaired");
        }
    }
    Ok(())
}

/// Adapter wiring the kernel's [`TapeService`] into the
/// [`ReconcileTape`] trait. Same shape as the boot-time copy in
/// `rara_app::boot` — kept inline to keep the CLI binary's dependency
/// tree minimal (no extra crate-public surface).
struct TapeReconciler {
    tape: TapeService,
}

#[async_trait::async_trait]
impl ReconcileTape for TapeReconciler {
    async fn read_tape(&self, key: &SessionKey) -> Option<TapeReport> {
        let tape_name = key.to_string();
        let info = self.tape.info(&tape_name).await.ok()?;
        let entries = self.tape.entries(&tape_name).await.ok()?;
        let preview = entries.iter().find_map(|e| {
            if e.kind != rara_kernel::memory::TapEntryKind::Message {
                return None;
            }
            if e.payload.get("role").and_then(|v| v.as_str()) != Some("user") {
                return None;
            }
            extract_preview(&e.payload)
        });
        let anchors = derive_anchors_from_disk(&tape_name)
            .await
            .unwrap_or_default();
        let updated_at = entries
            .last()
            .map(|e| jiff_to_chrono(e.timestamp))
            .unwrap_or_else(chrono::Utc::now);
        Some(TapeReport {
            total_entries: info.entries as i64,
            updated_at,
            last_token_usage: info.last_token_usage.map(|x| x as i64),
            estimated_context_tokens: info.estimated_context_tokens as i64,
            entries_since_last_anchor: info.entries_since_last_anchor as i64,
            anchors,
            preview,
        })
    }
}

fn extract_preview(payload: &serde_json::Value) -> Option<String> {
    const MAX: usize = 200;
    let s = payload
        .get("content")
        .and_then(|c| match c {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(arr) => arr
                .iter()
                .find_map(|seg| seg.get("text").and_then(|v| v.as_str()).map(str::to_owned)),
            _ => None,
        })
        .or_else(|| {
            payload
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX).collect())
}

fn jiff_to_chrono(ts: jiff::Timestamp) -> chrono::DateTime<chrono::Utc> {
    let mut second = ts.as_second();
    let mut nanosecond = ts.subsec_nanosecond();
    if nanosecond < 0 {
        second = second.saturating_sub(1);
        nanosecond = nanosecond.saturating_add(1_000_000_000);
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp(second, nanosecond as u32)
        .unwrap_or_else(chrono::Utc::now)
}

async fn derive_anchors_from_disk(tape_name: &str) -> Option<Vec<rara_kernel::session::AnchorRef>> {
    use tokio::io::AsyncBufReadExt;
    let path = rara_kernel::memory::find_tape_file(rara_paths::memory_dir(), tape_name)?;
    let file = tokio::fs::File::open(&path).await.ok()?;
    let mut reader = tokio::io::BufReader::new(file);
    let mut anchors = Vec::new();
    let mut offset: u64 = 0;
    let mut line = String::new();
    let mut entries_in_segment: i64 = 0;
    loop {
        line.clear();
        let read = reader.read_line(&mut line).await.ok()?;
        if read == 0 {
            break;
        }
        let line_start = offset;
        offset = offset.saturating_add(read as u64);

        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed.is_empty() {
            continue;
        }
        let entry: rara_kernel::memory::TapEntry = match serde_json::from_str(trimmed) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.kind == rara_kernel::memory::TapEntryKind::Anchor {
            entries_in_segment += 1;
            let name = entry
                .payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_owned();
            anchors.push(rara_kernel::session::AnchorRef {
                anchor_id: entry.id,
                byte_offset: line_start,
                name,
                timestamp: jiff_to_chrono(entry.timestamp),
                entry_count_in_segment: entries_in_segment,
            });
            entries_in_segment = 0;
        } else {
            entries_in_segment += 1;
        }
    }
    Some(anchors)
}
