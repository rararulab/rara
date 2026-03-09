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

use sysinfo::System;
use teloxide::{prelude::*, types::ChatId};
use tracing::warn;

/// Telegram notifier for gateway auto-update events.
///
/// All errors are logged but never propagated — notifications must not
/// break the update pipeline.
pub struct UpdateNotifier {
    bot:              Bot,
    channel_id:       i64,
    /// Gateway version string (injected from build_info).
    version:          String,
    /// Machine hostname.
    hostname:         String,
    /// OS version string.
    os:               String,
    /// CPU model string.
    cpu:              String,
    /// Total physical memory formatted as GB.
    memory:           String,
    /// Wall-clock time the gateway process started.
    started_at:       chrono::DateTime<chrono::Local>,
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
    /// System information (hostname, OS, CPU, memory) is gathered automatically
    /// via `sysinfo` at construction time.
    pub fn new(bot_token: &str, channel_id: i64, version: &str, repo_url: &str) -> Self {
        let bot = Bot::new(bot_token);

        let hostname = System::host_name().unwrap_or_else(|| "unknown".into());
        let os = System::long_os_version().unwrap_or_else(|| "unknown".into());

        let mut sys = System::new();
        sys.refresh_cpu_all();
        let cpu = sys
            .cpus()
            .first()
            .map(|c| c.brand().to_owned())
            .unwrap_or_else(|| "unknown".into());

        sys.refresh_memory();
        let total_gb = sys.total_memory() as f64 / 1_073_741_824.0;
        let memory = format!("{total_gb:.1} GB");

        Self {
            bot,
            channel_id,
            version: version.to_owned(),
            hostname,
            os,
            cpu,
            memory,
            started_at: chrono::Local::now(),
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

    pub async fn update_success(&self, new_rev: &str) {
        *self.agent_version.lock().unwrap() = new_rev.to_owned();
        self.send(&format!(
            "✅ <b>Auto-update: updated, restarting agent</b>\nnew rev: {}\n{}",
            self.commit_link(new_rev),
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

    pub async fn build_failed(&self, rev: &str, reason: &str) {
        self.send(&format!(
            "❌ <b>Auto-update: build failed</b>\ntarget: {}\n{}\n<pre>{reason}</pre>",
            self.commit_link(rev),
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
            "\n🖥 host: <code>{}</code>\n💻 os: <code>{}</code>\n🧠 cpu: <code>{}</code>\n💾 mem: \
             <code>{}</code>\n📦 gateway: <code>{}</code>\n🤖 agent: {}\n⏱ gateway since: {}\n🔄 \
             agent generation: {}\n🕐 agent since: {}",
            self.hostname,
            self.os,
            self.cpu,
            self.memory,
            self.version,
            self.commit_link(&agent_ver),
            self.started_at.format("%Y-%m-%d %H:%M:%S"),
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
