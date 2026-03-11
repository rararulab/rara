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

//! Telegram command listener for the gateway supervisor.
//!
//! Polls `getUpdates` on the notification channel using a dedicated bot token
//! (separate from rara's bot). Only processes `/command` messages from the
//! configured `channel_id`; everything else is silently ignored.

use teloxide::{
    payloads::{GetUpdatesSetters, SendMessageSetters},
    prelude::*,
    requests::{Request, Requester},
    types::{AllowedUpdate, ChatId, ParseMode, UpdateKind},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{detector::UpdateState, supervisor::SupervisorHandle};

/// Lightweight Telegram polling loop for gateway management commands.
pub struct GatewayTelegramListener {
    bot:               Bot,
    channel_id:        i64,
    supervisor_handle: SupervisorHandle,
    update_state_rx:   watch::Receiver<UpdateState>,
    shutdown:          CancellationToken,
    health_url:        String,
}

impl GatewayTelegramListener {
    pub fn new(
        bot_token: &str,
        channel_id: i64,
        supervisor_handle: SupervisorHandle,
        update_state_rx: watch::Receiver<UpdateState>,
        shutdown: CancellationToken,
        health_url: String,
    ) -> Self {
        Self {
            bot: Bot::new(bot_token),
            channel_id,
            supervisor_handle,
            update_state_rx,
            shutdown,
            health_url,
        }
    }

    /// Run the polling loop until the shutdown token is cancelled.
    pub async fn run(self) {
        let cancel = &self.shutdown;
        // Delete any stale webhook so getUpdates works.
        if let Err(e) = self.bot.delete_webhook().await {
            warn!(error = %e, "gateway telegram: failed to delete webhook");
        }

        match self.bot.get_me().await {
            Ok(me) => {
                info!(
                    bot_id = me.id.0,
                    bot_username = ?me.username,
                    "gateway telegram: bot identity verified"
                );
            }
            Err(e) => {
                warn!(error = %e, "gateway telegram: failed to verify bot — listener will not start");
                return;
            }
        };
        let mut offset: Option<i32> = None;

        loop {
            if cancel.is_cancelled() {
                info!("gateway telegram: shutting down");
                return;
            }

            // Build getUpdates request.
            let mut request = self.bot.get_updates().timeout(30);
            if let Some(off) = offset {
                request = request.offset(off);
            }
            request = request.allowed_updates(vec![AllowedUpdate::Message]);

            let updates = tokio::select! {
                result = request.send() => {
                    match result {
                        Ok(updates) => updates,
                        Err(e) => {
                            warn!(error = %e, "gateway telegram: getUpdates failed");
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            continue;
                        }
                    }
                }
                () = cancel.cancelled() => {
                    info!("gateway telegram: shutting down during poll");
                    return;
                }
            };

            for update in &updates {
                // Advance offset past this update.
                #[allow(clippy::cast_possible_wrap)]
                let next_offset = update.id.0 as i32 + 1;
                offset = Some(next_offset);

                let msg = match &update.kind {
                    UpdateKind::Message(msg) => msg,
                    _ => continue,
                };

                // Authorization: only process messages from the notification channel.
                if msg.chat.id.0 != self.channel_id {
                    continue;
                }

                let Some(raw_text) = msg.text() else {
                    continue;
                };

                let text = raw_text.trim();
                if !text.starts_with('/') {
                    continue;
                }

                let parts: Vec<&str> = text.split_whitespace().collect();
                // Strip @bot suffix from command token (e.g. "/restart@MyBot" → "/restart").
                let command_raw = parts[0];
                let command = command_raw
                    .find('@')
                    .map(|i| &command_raw[..i])
                    .unwrap_or(command_raw);
                let args = &parts[1..];

                let reply = self.handle_command(command, args).await;
                self.reply(&reply).await;

                // If shutdown was requested, cancel the token after replying.
                if command == "/shutdown" {
                    self.shutdown.cancel();
                }
            }
        }
    }

    /// Dispatch a command and return the HTML reply.
    async fn handle_command(&self, command: &str, args: &[&str]) -> String {
        match command {
            "/restart" => self.cmd_restart().await,
            "/shutdown" => self.cmd_shutdown(),
            "/status" => self.cmd_status(),
            "/sync" => self.cmd_sync(args).await,
            "/logs" => self.cmd_logs().await,
            "/health" => self.cmd_health().await,
            "/help" => self.cmd_help(),
            _ => format!("Unknown command: <code>{command}</code>\nUse /help to see available commands."),
        }
    }

    // -- Command implementations -----------------------------------------------

    async fn cmd_restart(&self) -> String {
        match self.supervisor_handle.restart().await {
            Ok(()) => "Restart initiated. Agent process will be restarted.".to_owned(),
            Err(e) => format!("<b>Restart failed</b>\n<pre>{e}</pre>"),
        }
    }

    fn cmd_shutdown(&self) -> String {
        "Shutdown initiated. Gateway and agent will shut down.".to_owned()
    }

    fn cmd_status(&self) -> String {
        let sup = self.supervisor_handle.status();
        let update = self.update_state_rx.borrow().clone();

        let running = if sup.running { "running" } else { "stopped" };
        let pid_str = sup.pid.map(|p| p.to_string()).unwrap_or_else(|| "—".into());
        let update_available = if update.update_available { "yes" } else { "no" };
        let upstream = update.upstream_rev.as_deref().unwrap_or("unknown");
        let current = &update.current_rev;
        let short_current = if current.len() >= 7 { &current[..7] } else { current };
        let short_upstream = if upstream.len() >= 7 { &upstream[..7] } else { upstream };

        format!(
            "<b>Gateway Status</b>\n\n\
             agent: {running}\n\
             pid: <code>{pid_str}</code>\n\
             restart_count: {}\n\
             local rev: <code>{short_current}</code>\n\
             upstream rev: <code>{short_upstream}</code>\n\
             update available: {update_available}",
            sup.restart_count,
        )
    }

    async fn cmd_sync(&self, args: &[&str]) -> String {
        let do_restart = args.contains(&"--restart");

        // git fetch origin main
        let fetch = tokio::process::Command::new("git")
            .args(["fetch", "origin", "main"])
            .current_dir(repo_dir())
            .output()
            .await;

        match fetch {
            Ok(o) if !o.status.success() => {
                let stderr = html_escape(&String::from_utf8_lossy(&o.stderr));
                return format!("<b>Sync failed</b>\n<code>git fetch</code> failed:\n<pre>{stderr}</pre>");
            }
            Err(e) => {
                return format!("<b>Sync failed</b>\n<pre>{}</pre>", html_escape(&e.to_string()));
            }
            _ => {}
        }

        // git merge --ff-only origin/main
        let merge = tokio::process::Command::new("git")
            .args(["merge", "--ff-only", "origin/main"])
            .current_dir(repo_dir())
            .output()
            .await;

        match merge {
            Ok(o) if o.status.success() => {
                let stdout = html_escape(&String::from_utf8_lossy(&o.stdout));
                let mut reply = format!("<b>Sync complete</b>\n<pre>{stdout}</pre>");
                if do_restart {
                    match self.supervisor_handle.restart().await {
                        Ok(()) => reply.push_str("\nRestart initiated."),
                        Err(e) => reply.push_str(&format!("\nRestart failed: <pre>{}</pre>", html_escape(&e.to_string()))),
                    }
                }
                reply
            }
            Ok(o) => {
                let stderr = html_escape(&String::from_utf8_lossy(&o.stderr));
                format!("<b>Sync failed</b>\n<code>git merge --ff-only</code> failed:\n<pre>{stderr}</pre>")
            }
            Err(e) => {
                format!("<b>Sync failed</b>\n<pre>{}</pre>", html_escape(&e.to_string()))
            }
        }
    }

    async fn cmd_logs(&self) -> String {
        let logs_dir = rara_paths::logs_dir();

        // Find the most recent .log file.
        let mut entries: Vec<_> = match std::fs::read_dir(logs_dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(e) => return format!("<b>Logs failed</b>\n<pre>{}</pre>", html_escape(&e.to_string())),
        };
        entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));

        let Some(latest) = entries.first() else {
            return "No log files found.".to_owned();
        };

        // Read last ~4KB from the file to avoid loading huge logs into memory.
        let tail_text = match read_tail(&latest.path(), 4096).await {
            Ok(t) => t,
            Err(e) => return format!("<b>Logs failed</b>\n<pre>{}</pre>", html_escape(&e.to_string())),
        };

        // Keep only complete lines (drop the first partial line after seek).
        let lines: Vec<&str> = tail_text.lines().collect();
        let lines = if lines.len() > 1 { &lines[1..] } else { &lines[..] };
        let tail = lines.iter().rev().take(50).rev().copied().collect::<Vec<_>>().join("\n");

        // Telegram message limit is 4096 chars. Truncate if needed.
        let truncated = if tail.len() > 3800 {
            &tail[tail.len() - 3800..]
        } else {
            &tail
        };

        format!(
            "<b>Recent logs</b> (last 50 lines)\n<pre>{}</pre>",
            html_escape(truncated),
        )
    }

    async fn cmd_health(&self) -> String {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        match client.get(&self.health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                "<b>Health check passed</b>\nAgent HTTP server is responding.".to_owned()
            }
            Ok(resp) => {
                format!(
                    "<b>Health check warning</b>\nStatus: <code>{}</code>",
                    resp.status()
                )
            }
            Err(e) => {
                format!("<b>Health check failed</b>\n<pre>{e}</pre>")
            }
        }
    }

    fn cmd_help(&self) -> String {
        "<b>Gateway Commands</b>\n\n\
         /restart — Restart the agent process\n\
         /shutdown — Shut down gateway + agent\n\
         /status — Show running status and update info\n\
         /sync — Git pull latest code (ff-only)\n\
         /sync --restart — Git pull + restart agent\n\
         /logs — Show last 50 lines of agent logs\n\
         /health — Check agent HTTP health endpoint\n\
         /help — Show this message"
            .to_owned()
    }

    // -- Helpers ---------------------------------------------------------------

    /// Send an HTML reply to the notification channel.
    async fn reply(&self, message: &str) {
        let result = self
            .bot
            .send_message(ChatId(self.channel_id), message)
            .parse_mode(ParseMode::Html)
            .await;

        if let Err(e) = result {
            warn!(error = %e, "gateway telegram: failed to send reply");
        }
    }
}

/// Minimal HTML escaping for output embedded in `<pre>` tags.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Read the last `max_bytes` from a file without loading it entirely.
async fn read_tail(path: &std::path::Path, max_bytes: u64) -> Result<String, std::io::Error> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = tokio::fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    let len = metadata.len();

    if len > max_bytes {
        file.seek(std::io::SeekFrom::End(-(max_bytes as i64))).await?;
    }

    let mut buf = String::new();
    file.read_to_string(&mut buf).await?;
    Ok(buf)
}

/// Best-effort repo root detection (directory containing the running executable).
fn repo_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}
