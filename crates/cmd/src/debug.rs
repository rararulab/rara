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

//! `rara debug <turn_id>` — print the full execution context for a
//! message without booting the chat UI or kernel runtime.
//!
//! The `execution_traces` SQLite table is the single source of truth: each
//! turn writes a fully aggregated [`ExecutionTrace`] (model, tokens,
//! iterations, thinking, tools, plan, rationale) keyed by
//! `rara_turn_id`.  We render that directly.  The on-disk tape is opened
//! only as a *supplementary* timeline — one fd, streamed line-by-line.

use std::{
    fmt::Write as _,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use clap::Args;
use rara_kernel::{
    memory::{TapEntry, find_tape_file},
    trace::{ExecutionTrace, TraceService},
};
use snafu::{ResultExt, Whatever};
use yunara_store::diesel_pool::{DieselPoolConfig, DieselSqlitePools, build_sqlite_pools};

#[derive(Debug, Clone, Args)]
#[command(about = "Inspect a message by its rara_turn_id")]
#[command(
    long_about = "Inspect a message by its rara_turn_id.\n\nLooks up the execution trace in the \
                  SQLite index, then attaches a chronological timeline from the on-disk tape.  \
                  Does not boot the kernel.\n\nExamples:\n  rara debug 01J4M8VW9XYZAB..."
)]
pub struct DebugCmd {
    /// The rara_turn_id to inspect.
    pub turn_id: String,
}

impl DebugCmd {
    pub async fn run(self) -> Result<(), Whatever> {
        let pool = open_db()
            .await
            .whatever_context("failed to open trace database")?;
        let trace_service = TraceService::new(pool);

        let lookup = trace_service
            .find_trace_by_turn_id(&self.turn_id)
            .await
            .whatever_context("trace lookup failed")?;

        let Some((session_id, trace)) = lookup else {
            println!("🔍 Debug: {}", self.turn_id);
            println!("{}", "─".repeat(60));
            println!(
                "No execution trace found for this message ID.\nThe trace may have expired (30 \
                 day retention), the turn may have failed before persistence, or the ID is for a \
                 slash command (which does not produce a turn)."
            );
            return Ok(());
        };

        // Walk the one matching tape file for the timeline. The trace
        // already has all aggregated stats; the tape only contributes a
        // chronological event view.
        let tape_path = find_tape_file(rara_paths::memory_dir(), &session_id);
        let timeline = match tape_path.as_deref() {
            Some(path) => {
                collect_timeline(path, &self.turn_id).whatever_context("failed to read tape")?
            }
            None => Vec::new(),
        };

        println!(
            "{}",
            render(
                &self.turn_id,
                &session_id,
                tape_path.as_deref(),
                &trace,
                &timeline
            )
        );
        Ok(())
    }
}

/// Open the rara SQLite database in read-only mode. The CLI must not run
/// migrations or hold a write lock — the running daemon may be active.
async fn open_db() -> Result<DieselSqlitePools, yunara_store::error::Error> {
    let db_path = rara_paths::database_dir().join("rara.db");
    let url = format!("sqlite:{}?mode=ro", db_path.display());
    build_sqlite_pools(
        &DieselPoolConfig::builder()
            .database_url(url)
            .max_connections(1)
            .build(),
    )
    .await
}

/// Single chronological event extracted from the tape.
struct TimelineEvent {
    timestamp: String,
    kind:      String,
    detail:    String,
}

/// Stream a single tape file and pull entries that mention `turn_id`.
/// Substring filtering is the hot path; we only invoke `serde_json` on
/// lines that already match.
fn collect_timeline(path: &Path, turn_id: &str) -> std::io::Result<Vec<TimelineEvent>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut events = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if !line.contains(turn_id) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<TapEntry>(&line) else {
            continue;
        };

        let kind = entry.kind.to_string();
        let detail = match kind.as_str() {
            "message" => {
                let role = entry
                    .payload
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let content = entry
                    .payload
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| s.chars().take(200).collect::<String>())
                    .unwrap_or_default();
                format!("[{role}] {content}")
            }
            "tool_call" => {
                let names: Vec<&str> = entry
                    .payload
                    .get("calls")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|c| c.get("name").and_then(|n| n.as_str()))
                            .collect()
                    })
                    .unwrap_or_default();
                if names.is_empty() {
                    "→ ?".to_owned()
                } else {
                    format!("→ {}", names.join(", "))
                }
            }
            "tool_result" => {
                let metrics: Vec<String> = entry
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("tool_metrics"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|m| {
                                let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                let success =
                                    m.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
                                let icon = if success { "✓" } else { "✗" };
                                format!("{icon} {name}")
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if metrics.is_empty() {
                    "✓ ?".to_owned()
                } else {
                    metrics.join(", ")
                }
            }
            _ => String::new(),
        };

        events.push(TimelineEvent {
            timestamp: entry.timestamp.strftime("%H:%M:%S").to_string(),
            kind,
            detail,
        });
    }
    Ok(events)
}

/// Render the full debug view as plain text for terminal output.
fn render(
    turn_id: &str,
    session_id: &str,
    tape_path: Option<&Path>,
    trace: &ExecutionTrace,
    timeline: &[TimelineEvent],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "🔍 Debug: {turn_id}");
    let _ = writeln!(out, "  Session: {session_id}");
    if let Some(path) = tape_path {
        let _ = writeln!(out, "  Tape:    {}", path.display());
    } else {
        let _ = writeln!(out, "  Tape:    [missing on disk]");
    }
    let _ = writeln!(out, "{}", "─".repeat(60));

    // -- Summary ---------------------------------------------------------------
    let _ = writeln!(out, "\n📊 Summary");
    let _ = writeln!(out, "  Duration:   {}s", trace.duration_secs);
    if !trace.model.is_empty() {
        let _ = writeln!(out, "  Model:      {}", trace.model);
    }
    if trace.iterations > 0 {
        let _ = writeln!(out, "  Iterations: {}", trace.iterations);
    }
    if trace.thinking_ms > 0 {
        let _ = writeln!(out, "  Thinking:   {}s", trace.thinking_ms / 1000);
    }
    if trace.input_tokens > 0 || trace.output_tokens > 0 {
        let _ = writeln!(
            out,
            "  Tokens:     ↑{} ↓{}",
            format_tokens(u64::from(trace.input_tokens)),
            format_tokens(u64::from(trace.output_tokens))
        );
    }
    if !trace.tools.is_empty() {
        let failures = trace.tools.iter().filter(|t| !t.success).count();
        let _ = writeln!(
            out,
            "  Tool calls: {} ({failures} failed)",
            trace.tools.len()
        );
    }

    // -- Rationale -------------------------------------------------------------
    if let Some(ref rationale) = trace.turn_rationale {
        let _ = writeln!(out, "\n💭 Rationale");
        for line in rationale.lines() {
            let _ = writeln!(out, "  {line}");
        }
    }

    // -- Thinking preview ------------------------------------------------------
    if !trace.thinking_preview.is_empty() {
        let _ = writeln!(out, "\n🧠 Thinking");
        for line in trace.thinking_preview.lines() {
            let _ = writeln!(out, "  {line}");
        }
    }

    // -- Plan steps ------------------------------------------------------------
    if !trace.plan_steps.is_empty() {
        let _ = writeln!(out, "\n📋 Plan");
        for step in &trace.plan_steps {
            let _ = writeln!(out, "  • {step}");
        }
    }

    // -- Tools -----------------------------------------------------------------
    if !trace.tools.is_empty() {
        let _ = writeln!(out, "\n🔧 Tools");
        for tool in &trace.tools {
            let duration = tool
                .duration_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "—".to_owned());
            let icon = if tool.success { "✓" } else { "✗" };
            let _ = writeln!(out, "  {icon} {} ({duration})", tool.name);
            if !tool.summary.is_empty() {
                let preview: String = tool.summary.chars().take(150).collect();
                let _ = writeln!(out, "    → {preview}");
            }
            if let Some(ref err) = tool.error {
                let preview: String = err.chars().take(200).collect();
                let _ = writeln!(out, "    ⚠ {preview}");
            }
        }
    }

    // -- Timeline --------------------------------------------------------------
    if !timeline.is_empty() {
        let _ = writeln!(out, "\n📝 Timeline ({} entries)", timeline.len());
        for ev in timeline {
            let _ = writeln!(out, "  {} [{}] {}", ev.timestamp, ev.kind, ev.detail);
        }
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
