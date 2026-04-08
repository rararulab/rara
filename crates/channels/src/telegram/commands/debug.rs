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
//! message, including tape entries, token usage, tool calls, and errors.

use std::fmt::Write;

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
    memory::TapeService,
    trace::TraceService,
};

use super::session::html_escape;

/// Handles the `/debug` command.
pub struct DebugCommandHandler {
    tape_service:  TapeService,
    trace_service: TraceService,
}

impl DebugCommandHandler {
    /// Create a new handler with tape and trace services.
    pub fn new(tape_service: TapeService, trace_service: TraceService) -> Self {
        Self {
            tape_service,
            trace_service,
        }
    }
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

        let mut output = String::new();
        let _ = writeln!(output, "<b>🔍 Debug: {}</b>\n", html_escape(message_id));

        // -- Section 1: ExecutionTrace from SQLite -----------------------------
        match self.trace_service.find_by_rara_message_id(message_id).await {
            Ok(Some(trace)) => {
                let _ = writeln!(output, "<b>📊 Execution Summary</b>");
                let _ = writeln!(output, "• Duration: {}s", trace.duration_secs);
                let _ = writeln!(
                    output,
                    "• Model: <code>{}</code>",
                    html_escape(&trace.model)
                );
                let _ = writeln!(output, "• Iterations: {}", trace.iterations);
                let _ = writeln!(
                    output,
                    "• Tokens: ↑{} ↓{}",
                    format_tokens(trace.input_tokens),
                    format_tokens(trace.output_tokens)
                );
                if trace.thinking_ms > 0 {
                    let _ = writeln!(output, "• Thinking: {}s", trace.thinking_ms / 1000);
                }
                if let Some(ref rationale) = trace.turn_rationale {
                    let preview: String = rationale.chars().take(300).collect();
                    let _ = writeln!(
                        output,
                        "\n<b>💭 Rationale</b>\n<i>{}</i>",
                        html_escape(&preview)
                    );
                }
                if !trace.thinking_preview.is_empty() {
                    let _ = writeln!(
                        output,
                        "\n<b>🧠 Thinking</b>\n<i>{}</i>",
                        html_escape(&trace.thinking_preview)
                    );
                }

                // Tools
                if !trace.tools.is_empty() {
                    let _ = writeln!(output, "\n<b>🔧 Tools ({} calls)</b>", trace.tools.len());
                    for tool in &trace.tools {
                        let duration = tool
                            .duration_ms
                            .map(|ms| format!("{}ms", ms))
                            .unwrap_or_else(|| "—".to_owned());
                        let status = if tool.success { "✓" } else { "✗" };
                        let _ = writeln!(
                            output,
                            "  {status} <code>{}</code> ({duration})",
                            html_escape(&tool.name)
                        );
                        if let Some(ref err) = tool.error {
                            let preview: String = err.chars().take(200).collect();
                            let _ = writeln!(output, "    ⚠️ {}", html_escape(&preview));
                        }
                        if !tool.summary.is_empty() {
                            let preview: String = tool.summary.chars().take(100).collect();
                            let _ = writeln!(output, "    → {}", html_escape(&preview));
                        }
                    }
                }
            }
            Ok(None) => {
                let _ = writeln!(
                    output,
                    "<i>No execution trace found (may have expired or ID is incorrect).</i>"
                );
            }
            Err(e) => {
                let _ = writeln!(
                    output,
                    "<i>Trace lookup failed: {}</i>",
                    html_escape(&e.to_string())
                );
            }
        }

        // -- Section 2: Tape entries -------------------------------------------
        let _ = writeln!(output, "\n<b>📝 Tape Entries</b>");
        match self.tape_service.search("", message_id, 50, true).await {
            Ok(entries) => {
                let matched: Vec<_> = entries
                    .into_iter()
                    .filter(|e| {
                        e.metadata.as_ref().map_or(false, |m| {
                            m.get("rara_message_id")
                                .and_then(|v| v.as_str())
                                .map_or(false, |id| id == message_id)
                        })
                    })
                    .collect();

                if matched.is_empty() {
                    let _ = writeln!(output, "<i>No tape entries found.</i>");
                } else {
                    let _ = writeln!(output, "Found {} entries:\n", matched.len());
                    for entry in &matched {
                        let kind = entry.kind.to_string();
                        let ts = entry.timestamp.strftime("%H:%M:%S").to_string();

                        // Extract key info based on entry kind.
                        let detail = match kind.as_str() {
                            "message" => entry
                                .payload
                                .get("content")
                                .and_then(|v| v.as_str())
                                .map(|s| {
                                    let preview: String = s.chars().take(100).collect();
                                    preview
                                })
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
                                let status = if success { "✓" } else { "✗" };
                                format!("{status} {name}")
                            }
                            _ => String::new(),
                        };

                        let _ = writeln!(
                            output,
                            "<code>{ts}</code> [{kind}] {}",
                            html_escape(&detail)
                        );
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(
                    output,
                    "<i>Tape search failed: {}</i>",
                    html_escape(&e.to_string())
                );
            }
        }

        Ok(CommandResult::Html(output))
    }
}

/// Format token count for display (e.g. 15200 → "15.2k").
fn format_tokens(tokens: u32) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}
