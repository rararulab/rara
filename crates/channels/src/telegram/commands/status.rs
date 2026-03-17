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

//! `/status` command — show session metadata, runtime metrics, scheduled
//! jobs, and system stats in a single view.

use std::{fmt::Write, sync::Arc};

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
    handle::KernelHandle,
    session::SessionKey,
};

use super::{
    client::BotServiceClient,
    session::{extract_channel_info, format_timestamp, html_escape},
};

/// Handles the `/status` command — a comprehensive dashboard view of the
/// current session, its scheduled jobs, and kernel-wide system stats.
pub struct StatusCommandHandler {
    client: Arc<dyn BotServiceClient>,
    handle: KernelHandle,
}

impl StatusCommandHandler {
    /// Create a new handler with the given service client and kernel handle.
    pub fn new(client: Arc<dyn BotServiceClient>, handle: KernelHandle) -> Self {
        Self { client, handle }
    }
}

#[async_trait]
impl CommandHandler for StatusCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![CommandDefinition {
            name:        "status".to_owned(),
            description: "Show session status and system stats".to_owned(),
            usage:       Some("/status".to_owned()),
        }]
    }

    async fn handle(
        &self,
        _command: &CommandInfo,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let (channel_type, chat_id) = extract_channel_info(context);

        // Resolve the active session for this channel.
        let session_key_str = match self
            .client
            .get_channel_session(channel_type, &chat_id)
            .await
        {
            Ok(Some(binding)) => binding.session_key,
            Ok(None) => {
                return Ok(CommandResult::Text(
                    "No active session. Send a message to start one.".to_owned(),
                ));
            }
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to resolve session: {e}"
                )));
            }
        };

        let mut text = String::new();

        // -- Section 1: Session metadata ------------------------------------
        match self.client.get_session(&session_key_str).await {
            Ok(detail) => {
                let title = detail.title.as_deref().unwrap_or("Untitled");
                let short_key = &detail.key[..8];
                let _ = writeln!(text, "<b>Session</b>");
                let _ = writeln!(text, "Title: {}", html_escape(title));
                let _ = writeln!(text, "Key: <code>{}</code>", html_escape(short_key));
                if let Some(ref model) = detail.model {
                    let _ = writeln!(text, "Model: {}", html_escape(model));
                }
                let _ = writeln!(text, "Created: {}", format_timestamp(&detail.created_at));
            }
            Err(e) => {
                let _ = writeln!(text, "<b>Session</b>");
                let _ = writeln!(text, "Failed to load session details: {e}");
            }
        }

        // -- Section 2: Runtime metrics (from process table) ----------------
        if let Ok(sk) = SessionKey::try_from_raw(&session_key_str) {
            if let Some(stats) = self.handle.session_stats(&sk).await {
                let _ = writeln!(text);
                let _ = writeln!(text, "<b>Runtime</b>");
                let _ = writeln!(text, "State: {}", stats.state);
                let _ = writeln!(text, "LLM calls: {}", stats.llm_calls);
                let _ = writeln!(text, "Tool calls: {}", stats.tool_calls);
                let _ = writeln!(text, "Tokens: {}", stats.tokens_consumed);
                let _ = writeln!(text, "Children: {}", stats.children.len());
            }

            // -- Section 3: Scheduled jobs ----------------------------------
            let jobs = self.handle.list_jobs(Some(&sk));
            let _ = writeln!(text);
            if jobs.is_empty() {
                let _ = writeln!(text, "<b>Scheduled jobs</b>: none");
            } else {
                const MAX_DISPLAY_JOBS: usize = 10;
                let _ = writeln!(text, "<b>Scheduled jobs</b> ({})", jobs.len());
                for job in jobs.iter().take(MAX_DISPLAY_JOBS) {
                    let msg = truncate_msg(&job.message, 40);
                    let schedule = job.trigger.summary();
                    let next = job.trigger.next_at().to_string();
                    let next_fmt = format_timestamp(&next);
                    let _ = writeln!(
                        text,
                        "  {} | {} | {}",
                        html_escape(&msg),
                        html_escape(&schedule),
                        next_fmt,
                    );
                }
                if jobs.len() > MAX_DISPLAY_JOBS {
                    let _ = writeln!(text, "  ... and {} more", jobs.len() - MAX_DISPLAY_JOBS);
                }
            }
        }

        // -- Section 4: System stats ----------------------------------------
        let sys = self.handle.system_stats();
        let _ = writeln!(text);
        let _ = writeln!(text, "<b>System</b>");
        let _ = writeln!(text, "Active sessions: {}", sys.active_sessions);
        let _ = writeln!(text, "Uptime: {}", format_uptime(sys.uptime_ms));
        let _ = writeln!(text, "Total tokens: {}", sys.total_tokens_consumed);

        Ok(CommandResult::Html(text))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Truncate a string to the first line and at most `max` characters.
fn truncate_msg(s: &str, max: usize) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.len() <= max {
        return first_line.to_owned();
    }
    let limit = max.saturating_sub(3);
    let end = first_line
        .char_indices()
        .take_while(|(i, _)| *i <= limit)
        .last()
        .map_or(0, |(i, c)| i + c.len_utf8());
    format!("{}...", &first_line[..end])
}

/// Format a millisecond uptime as "Xh Ym" or "Xm".
fn format_uptime(ms: u64) -> String {
    let total_minutes = ms / 60_000;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}
