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

//! `/debug <message_id>` command — retrieve full execution context for a
//! message by walking the session tape.
//!
//! All data (tokens, model, tools, timings) is derived from tape entry
//! metadata — the tape is the single source of truth, no parallel SQL
//! lookup needed.

use std::fmt::Write;

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
    memory::TapeService,
};

use super::session::html_escape;

/// Maximum tape entries to scan per debug request.
const MAX_ENTRIES: usize = 200;

/// Handles the `/debug` command.
pub struct DebugCommandHandler {
    tape_service: TapeService,
}

impl DebugCommandHandler {
    /// Create a new handler that reads from the given tape service.
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl CommandHandler for DebugCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![CommandDefinition {
            name:        "debug".to_owned(),
            description: "Debug a message by its ID".to_owned(),
            usage:       Some("/debug <message_id>".to_owned()),
        }]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        _context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let message_id = command.args.trim();
        if message_id.is_empty() {
            return Ok(CommandResult::Text(
                "Usage: /debug <message_id>\n\nThe message ID is shown at the bottom of each \
                 response trace (🆔 Message ID)."
                    .to_owned(),
            ));
        }

        // Cross-tape search: find any entry whose metadata.rara_message_id
        // matches. The empty tape_name argument plus all_tapes=true causes
        // TapeService::search to scan all session tapes.
        let entries = self
            .tape_service
            .search("", message_id, MAX_ENTRIES, true)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("tape search failed: {e}").into(),
            })?;

        let matched: Vec<_> = entries
            .into_iter()
            .filter(|e| {
                e.metadata.as_ref().is_some_and(|m| {
                    m.get("rara_message_id")
                        .and_then(|v| v.as_str())
                        .is_some_and(|id| id == message_id)
                })
            })
            .collect();

        let mut output = String::new();
        let _ = writeln!(
            output,
            "<b>🔍 Debug: <code>{}</code></b>\n",
            html_escape(message_id)
        );

        if matched.is_empty() {
            output.push_str(
                "<i>No tape entries found for this message ID. It may have expired or never \
                 existed.</i>",
            );
            return Ok(CommandResult::Html(output));
        }

        // Aggregate metrics from tape entry metadata.
        let mut model = String::new();
        let mut total_input = 0u64;
        let mut total_output = 0u64;
        let mut iterations = 0usize;
        let mut total_stream_ms = 0u64;
        let mut tool_calls = 0usize;
        let mut tool_failures = 0usize;

        for entry in &matched {
            let Some(meta) = entry.metadata.as_ref() else {
                continue;
            };

            // LlmEntryMetadata fields (assistant messages).
            if let Some(m) = meta.get("model").and_then(|v| v.as_str()) {
                if model.is_empty() {
                    model = m.to_owned();
                }
            }
            if let Some(usage) = meta.get("usage") {
                if let Some(t) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                    total_input += t;
                }
                if let Some(t) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                    total_output += t;
                }
            }
            if meta.get("iteration").is_some() {
                iterations += 1;
            }
            if let Some(ms) = meta.get("stream_ms").and_then(|v| v.as_u64()) {
                total_stream_ms += ms;
            }

            // ToolResultMetadata fields.
            if let Some(metrics) = meta.get("tool_metrics").and_then(|v| v.as_array()) {
                tool_calls += metrics.len();
                tool_failures += metrics
                    .iter()
                    .filter(|m| m.get("success").and_then(|v| v.as_bool()) == Some(false))
                    .count();
            }
        }

        // -- Section 1: Summary ------------------------------------------------
        let _ = writeln!(output, "<b>📊 Summary</b>");
        let _ = writeln!(output, "• Entries: {}", matched.len());
        if !model.is_empty() {
            let _ = writeln!(output, "• Model: <code>{}</code>", html_escape(&model));
        }
        if iterations > 0 {
            let _ = writeln!(output, "• Iterations: {iterations}");
        }
        if total_stream_ms > 0 {
            let _ = writeln!(output, "• Stream: {:.1}s", total_stream_ms as f64 / 1000.0);
        }
        if total_input > 0 || total_output > 0 {
            let _ = writeln!(
                output,
                "• Tokens: ↑{} ↓{}",
                format_tokens(total_input),
                format_tokens(total_output)
            );
        }
        if tool_calls > 0 {
            let _ = writeln!(
                output,
                "• Tool calls: {tool_calls} ({tool_failures} failed)"
            );
        }

        // -- Section 2: Tool execution detail ----------------------------------
        let mut tool_lines: Vec<String> = Vec::new();
        for entry in &matched {
            let Some(meta) = entry.metadata.as_ref() else {
                continue;
            };
            let Some(metrics) = meta.get("tool_metrics").and_then(|v| v.as_array()) else {
                continue;
            };
            for m in metrics {
                let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let duration = m
                    .get("duration_ms")
                    .and_then(|v| v.as_u64())
                    .map(|ms| format!("{ms}ms"))
                    .unwrap_or_else(|| "—".to_owned());
                let success = m.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
                let icon = if success { "✓" } else { "✗" };
                let mut line = format!("  {icon} <code>{}</code> ({duration})", html_escape(name));
                if let Some(err) = m.get("error").and_then(|v| v.as_str()) {
                    let preview: String = err.chars().take(150).collect();
                    let _ = write!(line, "\n    ⚠️ {}", html_escape(&preview));
                }
                tool_lines.push(line);
            }
        }
        if !tool_lines.is_empty() {
            let _ = writeln!(output, "\n<b>🔧 Tools</b>");
            for line in tool_lines {
                output.push_str(&line);
                output.push('\n');
            }
        }

        // -- Section 3: Timeline -----------------------------------------------
        let _ = writeln!(output, "\n<b>📝 Timeline</b>");
        for entry in &matched {
            let kind = entry.kind.to_string();
            let ts = entry.timestamp.strftime("%H:%M:%S").to_string();

            let detail = match kind.as_str() {
                "message" => entry
                    .payload
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| s.chars().take(100).collect::<String>())
                    .unwrap_or_default(),
                "tool_call" => {
                    let name = entry
                        .payload
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    format!("→ {name}")
                }
                "tool_result" => {
                    let name = entry
                        .payload
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let success = entry
                        .payload
                        .get("success")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let icon = if success { "✓" } else { "✗" };
                    format!("{icon} {name}")
                }
                _ => String::new(),
            };

            let _ = writeln!(
                output,
                "<code>{ts}</code> [{kind}] {}",
                html_escape(&detail)
            );
        }

        Ok(CommandResult::Html(output))
    }
}

/// Format token count for display (e.g. 15200 → "15.2k").
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}
