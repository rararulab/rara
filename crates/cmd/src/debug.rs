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
//! Reads tape JSONL files directly via [`TapeService`] and renders the
//! shared [`MessageDebugSummary`] as plain text suitable for terminals
//! and log triage.

use std::fmt::Write;

use clap::Args;
use rara_kernel::{
    debug::MessageDebugSummary,
    memory::{FileTapeStore, TapeService},
};
use snafu::{ResultExt, Whatever};

/// Maximum tape entries scanned per debug request — same cap as the
/// Telegram handler so behavior matches.
const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, Args)]
#[command(about = "Inspect a message by its rara_message_id")]
#[command(
    long_about = "Inspect a message by its rara_message_id.\n\nReads tape entries from disk (no \
                  kernel boot required) and prints execution metrics, tool calls, and a \
                  chronological timeline.\n\nExamples:\n  rara debug 01J4M8VW9XYZAB..."
)]
pub struct DebugCmd {
    /// The rara_message_id to inspect.
    pub message_id: String,
}

impl DebugCmd {
    pub async fn run(self) -> Result<(), Whatever> {
        let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let store = FileTapeStore::new(rara_paths::memory_dir(), &workspace)
            .await
            .whatever_context("failed to open tape store")?;
        let tape_service = TapeService::new(store);

        // Cross-tape search — empty tape_name + all_tapes=true scans every
        // session JSONL on disk.
        let entries = tape_service
            .search("", &self.message_id, MAX_ENTRIES, true)
            .await
            .whatever_context("tape search failed")?;

        let summary = MessageDebugSummary::from_entries(&self.message_id, entries);
        println!("{}", render_text(&summary));
        Ok(())
    }
}

/// Render a [`MessageDebugSummary`] as plain text for terminal output.
fn render_text(summary: &MessageDebugSummary) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "🔍 Debug: {}", summary.message_id);
    let _ = writeln!(out, "{}", "─".repeat(60));

    if summary.is_empty() {
        out.push_str("No tape entries found for this message ID.\n");
        out.push_str("It may have expired or never existed.\n");
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
