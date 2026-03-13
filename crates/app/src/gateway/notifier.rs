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

//! Lightweight Telegram notifier for gateway update lifecycle events.
//!
//! Uses [`teloxide::Bot`] to send messages to a configured notification
//! channel. Does NOT go through the kernel IO subsystem — the gateway
//! runs independently of the kernel.

use std::sync::atomic::{AtomicU32, Ordering};

use teloxide::{prelude::*, types::ChatId};
use tracing::warn;

/// Telegram notifier for gateway auto-update events.
///
/// All errors are logged but never propagated — notifications must not
/// break the update pipeline.
pub struct UpdateNotifier {
    bot:              Bot,
    channel_id:       i64,
    /// Number of times the agent has been (re)started.
    agent_generation: AtomicU32,
    /// Wall-clock time of the most recent agent launch.
    agent_started_at: std::sync::Mutex<Option<chrono::DateTime<chrono::Local>>>,
    /// Version/revision of the currently running agent binary.
    /// Initially same as gateway version; updated after each successful update.
    agent_version:    std::sync::Mutex<String>,
    /// Repository URL for building commit links (e.g. "https://github.com/rararulab/rara").
    repo_url:         String,
}

impl UpdateNotifier {
    /// Create a new notifier.
    ///
    /// The `bot` should be built via [`rara_channels::telegram::build_bot`]
    /// which handles proxy and timeout configuration.
    pub fn new(bot: Bot, channel_id: i64, version: &str, repo_url: &str) -> Self {
        Self {
            bot,
            channel_id,
            agent_generation: AtomicU32::new(0),
            agent_started_at: std::sync::Mutex::new(None),
            agent_version: std::sync::Mutex::new(version.to_owned()),
            repo_url: repo_url.to_owned(),
        }
    }

    /// Build an HTML link to a commit on GitHub.
    ///
    /// Shows the first 7 characters of the revision as the visible text.
    fn commit_link(&self, rev: &str) -> String {
        let short = if rev.len() >= 7 { &rev[..7] } else { rev };
        format!("<a href=\"{}/commit/{rev}\">{short}</a>", self.repo_url)
    }

    // -- lifecycle events -----------------------------------------------------

    pub async fn agent_healthy(&self) {
        self.agent_generation.fetch_add(1, Ordering::Relaxed);
        *self.agent_started_at.lock().unwrap() = Some(chrono::Local::now());
        self.send(&format!(
            "✅ <b>Agent started and healthy</b>\n{}",
            self.status_block(),
        ))
        .await;
    }

    pub async fn update_started(&self, rev: &str) {
        self.send(&format!(
            "🔄 <b>Auto-update: starting build</b>\ntarget: {}\n{}",
            self.commit_link(rev),
            self.status_block(),
        ))
        .await;
    }

    pub async fn build_in_progress(&self) {
        self.send(&format!(
            "🔨 <b>Auto-update: building new version…</b>\n{}",
            self.status_block(),
        ))
        .await;
    }

    pub async fn update_success(&self, new_rev: &str, build_duration: std::time::Duration) {
        *self.agent_version.lock().unwrap() = new_rev.to_owned();
        self.send(&format!(
            "✅ <b>Auto-update: updated, restarting agent</b>\nnew rev: {}\n⏱ build: {}\n{}",
            self.commit_link(new_rev),
            format_duration(build_duration),
            self.status_block(),
        ))
        .await;
    }

    // -- resource events ------------------------------------------------------

    pub async fn resource_alert(&self, detail: &str) {
        self.send(&format!(
            "\u{26a0}\u{fe0f} <b>Resource alert</b>\n{detail}\n{}",
            self.status_block(),
        ))
        .await;
    }

    // -- error events ---------------------------------------------------------

    pub async fn executor_creation_failed(&self, err: &str) {
        self.send(&format!(
            "❌ <b>Auto-update: executor creation failed</b>\n{}\n<pre>{err}</pre>",
            self.status_block(),
        ))
        .await;
    }

    pub async fn build_failed(&self, rev: &str, reason: &str, build_duration: std::time::Duration) {
        self.send(&format!(
            "❌ <b>Auto-update: build failed</b>\ntarget: {}\n⏱ build: {}\n{}\n<pre>{reason}</pre>",
            self.commit_link(rev),
            format_duration(build_duration),
            self.status_block(),
        ))
        .await;
    }

    pub async fn activation_failed(&self, reason: &str, rolled_back: bool) {
        self.send(&format!(
            "❌ <b>Auto-update: activation failed</b>\nrolled_back: \
             {rolled_back}\n{}\n<pre>{reason}</pre>",
            self.status_block(),
        ))
        .await;
    }

    pub async fn restart_failed(&self, err: &str) {
        self.send(&format!(
            "❌ <b>Auto-update: restart failed</b>\n{}\n<pre>{err}</pre>",
            self.status_block(),
        ))
        .await;
    }

    // -- internal -------------------------------------------------------------

    /// Render the common status block appended to every notification.
    fn status_block(&self) -> String {
        let generation = self.agent_generation.load(Ordering::Relaxed);

        let agent_since = self
            .agent_started_at
            .lock()
            .unwrap()
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "—".into());

        let agent_ver = self.agent_version.lock().unwrap().clone();

        format!(
            "\n🤖 agent: {}\n🔄 generation: {}\n🕐 since: {}",
            self.commit_link(&agent_ver),
            generation,
            agent_since,
        )
    }

    /// Send a message via Telegram. Errors are logged, never propagated.
    async fn send(&self, message: &str) {
        let result = self
            .bot
            .send_message(ChatId(self.channel_id), message)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await;

        if let Err(e) = result {
            warn!(error = %e, "UpdateNotifier: failed to send Telegram notification");
        }
    }
}

/// Format a [`std::time::Duration`] as a human-readable string (e.g. "2m 35s").
fn format_duration(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    if mins > 0 {
        format!("{mins}m {secs}s")
    } else {
        format!("{secs}s")
    }
}
