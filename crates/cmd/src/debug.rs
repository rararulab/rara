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

//! `rara debug <message_id>` — print the full execution context for a
//! message without booting the chat UI or kernel runtime.
//!
//! Two-stage lookup keeps fd usage at O(1):
//! 1. **Index** — query the `execution_traces` SQLite table for the
//!    `session_id` that produced the turn (single indexed row read).
//! 2. **Content** — open exactly one tape JSONL file, stream-grep it for the
//!    message ID, then close the fd.
//!
//! The previous implementation walked every tape file via the
//! `FileTapeStore` cache and tripped macOS' 256-fd ulimit (EMFILE).

use std::{
    fmt::Write as _,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use clap::Args;
use rara_kernel::{
    debug::MessageDebugSummary,
    memory::{TapEntry, find_tape_file},
    trace::TraceService,
};
use snafu::{ResultExt, Whatever};
use sqlx::sqlite::SqlitePoolOptions;

#[derive(Debug, Clone, Args)]
#[command(about = "Inspect a message by its rara_message_id")]
#[command(
    long_about = "Inspect a message by its rara_message_id.\n\nUses the execution_traces SQLite \
                  index to locate the originating session, then streams that one tape JSONL file \
                  to print execution metrics, tool calls, and a chronological timeline. Does not \
                  boot the kernel.\n\nExamples:\n  rara debug 01J4M8VW9XYZAB..."
)]
pub struct DebugCmd {
    /// The rara_message_id to inspect.
    pub message_id: String,
}

impl DebugCmd {
    pub async fn run(self) -> Result<(), Whatever> {
        // Stage 1: SQL index lookup → session_id.
        let pool = open_db()
            .await
            .whatever_context("failed to open trace database")?;
        let trace_service = TraceService::new(pool);
        let session_id = trace_service
            .find_session_for_message(&self.message_id)
            .await
            .whatever_context("trace index lookup failed")?;

        let Some(session_id) = session_id else {
            println!("🔍 Debug: {}", self.message_id);
            println!("{}", "─".repeat(60));
            println!(
                "No execution trace found for this message ID.\nThe trace may have expired (30 \
                 day retention), the turn may have failed before persistence, or the ID is \
                 incorrect."
            );
            return Ok(());
        };

        // Stage 2: resolve tape path and stream the one matching file.
        let Some(tape_path) = find_tape_file(rara_paths::memory_dir(), &session_id) else {
            return Err(snafu::FromString::without_source(format!(
                "session {session_id} found in trace index but tape file is missing on disk",
            )));
        };

        let entries = scan_tape_for_message(&tape_path, &self.message_id)
            .whatever_context("failed to read tape file")?;

        let summary = MessageDebugSummary::from_entries(&self.message_id, entries);
        println!("{}", render_text(&summary, &session_id, &tape_path));
        Ok(())
    }
}

/// Open the rara SQLite database in read-only mode. The CLI must not run
/// migrations or hold a write lock — the running daemon may be active.
async fn open_db() -> Result<sqlx::SqlitePool, sqlx::Error> {
    let db_path = rara_paths::database_dir().join("rara.db");
    let url = format!("sqlite:{}?mode=ro", db_path.display());
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
}

/// Stream a single tape file line-by-line. The substring check is the hot
/// path — we only invoke `serde_json` on lines that already contain the
/// message ID, which avoids parsing the rest of the session history.
fn scan_tape_for_message(path: &Path, message_id: &str) -> std::io::Result<Vec<TapEntry>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if !line.contains(message_id) {
            continue;
        }
        match serde_json::from_str::<TapEntry>(&line) {
            Ok(entry) => out.push(entry),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "skipping malformed tape entry"
                );
            }
        }
    }
    Ok(out)
}

/// Render a [`MessageDebugSummary`] as plain text for terminal output.
fn render_text(summary: &MessageDebugSummary, session_id: &str, tape_path: &Path) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "🔍 Debug: {}", summary.message_id);
    let _ = writeln!(out, "  Session: {session_id}");
    let _ = writeln!(out, "  Tape:    {}", tape_path.display());
    let _ = writeln!(out, "{}", "─".repeat(60));

    if summary.is_empty() {
        out.push_str(
            "Trace index pointed at this session but no matching tape entries were found.\nThe \
             tape may have been compacted/folded since the trace was written.\n",
        );
        return out;
    }

    // -- Summary ---------------------------------------------------------------
    let _ = writeln!(out, "\n📊 Summary");
    let _ = writeln!(out, "  Entries:    {}", summary.entries.len());
    if let Some(ref model) = summary.model {
        let _ = writeln!(out, "  Model:      {model}");
    }
    if summary.iterations > 0 {
        let _ = writeln!(out, "  Iterations: {}", summary.iterations);
    }
    if summary.stream_ms > 0 {
        let _ = writeln!(
            out,
            "  Stream:     {:.1}s",
            summary.stream_ms as f64 / 1000.0
        );
    }
    if summary.input_tokens > 0 || summary.output_tokens > 0 {
        let _ = writeln!(
            out,
            "  Tokens:     ↑{} ↓{}",
            format_tokens(summary.input_tokens),
            format_tokens(summary.output_tokens)
        );
    }
    if !summary.tools.is_empty() {
        let _ = writeln!(
            out,
            "  Tool calls: {} ({} failed)",
            summary.tools.len(),
            summary.tool_failures
        );
    }

    // -- Tools -----------------------------------------------------------------
    if !summary.tools.is_empty() {
        let _ = writeln!(out, "\n🔧 Tools");
        for tool in &summary.tools {
            let duration = tool
                .duration_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "—".to_owned());
            let icon = if tool.success { "✓" } else { "✗" };
            let _ = writeln!(out, "  {icon} {} ({duration})", tool.name);
            if let Some(ref err) = tool.error {
                let preview: String = err.chars().take(150).collect();
                let _ = writeln!(out, "    ⚠ {preview}");
            }
        }
    }

    // -- Timeline --------------------------------------------------------------
    let _ = writeln!(out, "\n📝 Timeline");
    for item in &summary.timeline {
        let _ = writeln!(out, "  {} [{}] {}", item.timestamp, item.kind, item.detail);
    }

    out
}

/// Format token count for display (e.g. 15200 → "15.2k").
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}
