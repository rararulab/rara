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
//! Aggregation lives in [`rara_kernel::debug::MessageDebugSummary`]; this
//! handler only renders the result as Telegram HTML.

use std::fmt::Write;

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    debug::MessageDebugSummary,
    error::KernelError,
    memory::TapeService,
};

use super::session::html_escape;

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
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let message_id = command.args.trim();
        if message_id.is_empty() {
            return Ok(CommandResult::Text(
                "Usage: /debug <message_id>\n\nThe message ID is shown at the bottom of each \
                 response trace (🆔 Message ID)."
                    .to_owned(),
            ));
        }

        // Exact metadata filter on the current session's tape — returns all
        // entry kinds (messages, tool calls, tool results, events) so the
        // debug view shows the complete execution context.
        let entries = self
            .tape_service
            .entries_by_message_id(&context.session_key, message_id)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("tape lookup failed: {e}").into(),
            })?;

        let summary = MessageDebugSummary::from_entries(message_id, entries);
        Ok(CommandResult::Html(render_html(&summary)))
    }
}

/// Render a [`MessageDebugSummary`] as Telegram HTML.
fn render_html(summary: &MessageDebugSummary) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "<b>🔍 Debug: <code>{}</code></b>\n",
        html_escape(&summary.message_id)
    );

    if summary.is_empty() {
        output.push_str(
            "<i>No tape entries found for this message ID. It may have expired or never \
             existed.</i>",
        );
        return output;
    }

    // -- Section 1: Summary ----------------------------------------------------
    let _ = writeln!(output, "<b>📊 Summary</b>");
    let _ = writeln!(output, "• Entries: {}", summary.entries.len());
    if let Some(ref model) = summary.model {
        let _ = writeln!(output, "• Model: <code>{}</code>", html_escape(model));
    }
    if summary.iterations > 0 {
        let _ = writeln!(output, "• Iterations: {}", summary.iterations);
    }
    if summary.stream_ms > 0 {
        let _ = writeln!(
            output,
            "• Stream: {:.1}s",
            summary.stream_ms as f64 / 1000.0
        );
    }
    if summary.input_tokens > 0 || summary.output_tokens > 0 {
        let _ = writeln!(
            output,
            "• Tokens: ↑{} ↓{}",
            format_tokens(summary.input_tokens),
            format_tokens(summary.output_tokens)
        );
    }
    if !summary.tools.is_empty() {
        let _ = writeln!(
            output,
            "• Tool calls: {} ({} failed)",
            summary.tools.len(),
            summary.tool_failures
        );
    }

    // -- Section 2: Tool execution detail --------------------------------------
    if !summary.tools.is_empty() {
        let _ = writeln!(output, "\n<b>🔧 Tools</b>");
        for tool in &summary.tools {
            let duration = tool
                .duration_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "—".to_owned());
            let icon = if tool.success { "✓" } else { "✗" };
            let _ = writeln!(
                output,
                "  {icon} <code>{}</code> ({duration})",
                html_escape(&tool.name)
            );
            if let Some(ref err) = tool.error {
                let preview: String = err.chars().take(150).collect();
                let _ = writeln!(output, "    ⚠️ {}", html_escape(&preview));
            }
        }
    }

    // -- Section 3: Timeline ---------------------------------------------------
    let _ = writeln!(output, "\n<b>📝 Timeline</b>");
    for item in &summary.timeline {
        let _ = writeln!(
            output,
            "<code>{}</code> [{}] {}",
            item.timestamp,
            item.kind,
            html_escape(&item.detail)
        );
    }

    output
}

/// Format token count for display (e.g. 15200 → "15.2k").
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}
