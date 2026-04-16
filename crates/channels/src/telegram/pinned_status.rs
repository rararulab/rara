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

//! Pinned session summary card for Telegram chats.
//!
//! `PinnedSessionCard` renders an HTML session overview pinned to the top of
//! the chat, modeled after opencode-telegram-bot's session summary card.
//!
//! Updates only on meaningful state changes, with skip-unchanged optimization
//! to avoid redundant Telegram API calls.

use teloxide::types::MessageId;

use super::adapter::format_token_count;

// ---------------------------------------------------------------------------
// Model metadata — context window sizes and per-token pricing
// ---------------------------------------------------------------------------

/// Known model context window size.
struct ModelInfo {
    context_window: u32,
}

/// Shorten an absolute path to at most the last 3 segments.
fn short_path(path: &str) -> &str {
    let bytes = path.as_bytes();
    let mut slash_count = 0;
    for i in (0..bytes.len()).rev() {
        if bytes[i] == b'/' {
            slash_count += 1;
            if slash_count == 3 {
                return &path[i + 1..];
            }
        }
    }
    path
}

/// Best-effort lookup of model metadata by name substring.
///
/// Matches the most specific substring first. Returns `None` for unknown
/// models — the card gracefully omits context % and cost when unavailable.
fn lookup_model_info(model: &str) -> Option<ModelInfo> {
    let m = model.to_lowercase();

    // Claude family — 200K context
    if m.contains("opus") || m.contains("sonnet") || m.contains("haiku") {
        return Some(ModelInfo {
            context_window: 200_000,
        });
    }
    // OpenAI o-series — 200K context
    if m.contains("o3") || m.contains("o4-mini") {
        return Some(ModelInfo {
            context_window: 200_000,
        });
    }
    // GPT-4o family — 128K context
    if m.contains("gpt-4o") {
        return Some(ModelInfo {
            context_window: 128_000,
        });
    }
    // Gemini — 1M context
    if m.contains("gemini-2") || m.contains("gemini-1.5") {
        return Some(ModelInfo {
            context_window: 1_000_000,
        });
    }
    // DeepSeek — 128K context
    if m.contains("deepseek") {
        return Some(ModelInfo {
            context_window: 128_000,
        });
    }

    None
}

/// Look up the context window size (in tokens) for a known model.
///
/// Thin wrapper around [`lookup_model_info`] exposed to sibling modules
/// so that [`super::reply_keyboard`] can compute the context usage gauge.
pub(super) fn context_window_for_model(model: &str) -> Option<u32> {
    lookup_model_info(model).map(|info| info.context_window)
}

// ---------------------------------------------------------------------------
// Session card
// ---------------------------------------------------------------------------

/// Agent execution state shown in the card header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    Running,
    Idle,
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => f.write_str("Running"),
            Self::Idle => f.write_str("Idle"),
        }
    }
}

/// A background sub-agent tracked in the session card.
#[derive(Debug, Clone)]
pub(super) struct BackgroundTaskEntry {
    pub task_id:    String,
    pub agent_name: String,
    pub finished:   bool,
}

/// A file modified during this session, tracked for pinned card display.
#[derive(Debug, Clone)]
pub(super) struct FileChange {
    pub path:      String,
    pub additions: u64,
    pub deletions: u64,
}

/// Pinned session summary card.
///
/// The first line is an **identity bar** — shown in Telegram's floating pin
/// preview — containing session title, agent name, and optional channel tag.
/// Metrics (model, context, cost) go in the card body, visible on tap.
///
/// This matches the opencode-telegram-bot pattern where the pin preview
/// answers "where am I?" rather than "what's happening?".
#[derive(Debug)]
pub(super) struct PinnedSessionCard {
    /// Telegram chat ID this card belongs to.
    pub chat_id:      i64,
    /// Message ID of the pinned message, `None` until first send.
    pub message_id:   Option<MessageId>,
    /// Session identifier used for detecting session switches.
    pub session_id:   String,
    session_title:    String,
    model:            String,
    state:            State,
    input_tokens:     u32,
    output_tokens:    u32,
    thinking_ms:      u64,
    tool_calls:       u32,
    background_tasks: Vec<BackgroundTaskEntry>,
    changed_files:    Vec<FileChange>,
    dirty:            bool,
    /// Last rendered HTML — skip-unchanged optimization.
    last_rendered:    String,
}

impl PinnedSessionCard {
    /// Create a new session card.
    ///
    /// - `session_title` — human-readable session label (topic or ID)
    pub fn new(chat_id: i64, session_id: String, session_title: String) -> Self {
        Self {
            chat_id,
            message_id: None,
            session_id,
            session_title,
            model: String::new(),
            state: State::Running,
            input_tokens: 0,
            output_tokens: 0,
            thinking_ms: 0,
            tool_calls: 0,
            background_tasks: Vec::new(),
            changed_files: Vec::new(),
            dirty: true,
            last_rendered: String::new(),
        }
    }

    /// Render the session summary card as HTML.
    ///
    /// **Line 1 — Identity bar** (visible in Telegram's floating pin preview):
    /// `🟢 {session_title} · {agent_name}`
    /// Answers "where am I?" — session context, not telemetry.
    ///
    /// **Body — Metrics** (visible when user taps the pinned message):
    /// Model, Context %, Cost, Thinking, Tools, Background.
    pub fn render(&self) -> String {
        let mut lines = Vec::with_capacity(8);

        // Line 1: Identity bar — status emoji + session title + agent name.
        // This is what shows in the floating pin preview at the chat top.
        let status_emoji = match self.state {
            State::Running => "\u{1f7e2}", // 🟢
            State::Idle => "\u{26aa}",     // ⚪
        };
        lines.push(format!("{status_emoji} <b>{}</b>", self.session_title));

        let model_info = if self.model.is_empty() {
            None
        } else {
            lookup_model_info(&self.model)
        };

        // Model.
        if !self.model.is_empty() {
            lines.push(format!("Model: <code>{}</code>", self.model));
        }

        // Context: input_tokens is the current prompt/context size in our
        // stream protocol. output_tokens is cumulative completion — must NOT
        // be mixed into the context meter.
        if self.input_tokens > 0 {
            let used_str = format_token_count(self.input_tokens);
            let context_line = if let Some(ref info) = model_info {
                let limit_str = format_token_count(info.context_window);
                let pct = (self.input_tokens as f64 / info.context_window as f64 * 100.0) as u32;
                format!("Context: {used_str} / {limit_str} ({pct}%)")
            } else {
                format!("Context: {used_str}")
            };
            lines.push(context_line);
        }

        // Thinking time (rara-specific, shown when non-zero).
        if self.thinking_ms > 0 {
            let secs = self.thinking_ms / 1000;
            if secs > 0 {
                lines.push(format!("Thinking: {secs}s"));
            }
        }

        // Tool calls (rara-specific, shown when non-zero).
        if self.tool_calls > 0 {
            lines.push(format!("\u{1f527} {} tool calls", self.tool_calls));
        }

        // Active background tasks (rara-specific).
        let active: Vec<&BackgroundTaskEntry> = self
            .background_tasks
            .iter()
            .filter(|t| !t.finished)
            .collect();
        if !active.is_empty() {
            lines.push(String::new());
            lines.push(format!("\u{1f504} <b>Background ({})</b>", active.len()));
            for task in &active {
                lines.push(format!("\u{23f3} {}", task.agent_name));
            }
        }

        // Changed files (shown when any file-mutating tool completed).
        if !self.changed_files.is_empty() {
            let total = self.changed_files.len();
            lines.push(String::new());
            lines.push(format!("\u{1f4c1} <b>Files ({total})</b>"));
            let max_display = 10;
            for f in self.changed_files.iter().take(max_display) {
                let rel = short_path(&f.path);
                let mut parts = Vec::new();
                if f.additions > 0 {
                    parts.push(format!("+{}", f.additions));
                }
                if f.deletions > 0 {
                    parts.push(format!("-{}", f.deletions));
                }
                let diff_str = if parts.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", parts.join(" "))
                };
                lines.push(format!("  {rel}{diff_str}"));
            }
            if total > max_display {
                lines.push(format!("  \u{2026} and {} more", total - max_display));
            }
        }

        lines.join("\n")
    }

    // ── Event callbacks ──
    // Each sets dirty=true. The forwarder calls needs_flush() which compares
    // render() output against last_rendered to skip no-op API calls.

    /// Called when cumulative usage counters are updated.
    pub fn on_usage_update(&mut self, input_tokens: u32, output_tokens: u32, thinking_ms: u64) {
        self.input_tokens = input_tokens;
        self.output_tokens = output_tokens;
        self.thinking_ms = thinking_ms;
        self.dirty = true;
    }

    /// Called when a tool call begins.
    pub fn on_tool_start(&mut self) {
        self.tool_calls += 1;
        self.dirty = true;
    }

    /// Called when a tool call finishes.
    pub fn on_tool_end(&mut self) { self.dirty = true; }

    /// Called when turn metrics arrive (resolves the model name).
    pub fn on_turn_metrics(&mut self, model: String) {
        self.model = model;
        self.dirty = true;
    }

    /// Record a file mutation (write/edit) with diff stats.
    ///
    /// If the file was already tracked, accumulates the +/- counts.
    pub fn on_file_changed(&mut self, path: String, additions: u64, deletions: u64) {
        if let Some(existing) = self.changed_files.iter_mut().find(|f| f.path == path) {
            existing.additions += additions;
            existing.deletions += deletions;
        } else {
            self.changed_files.push(FileChange {
                path,
                additions,
                deletions,
            });
        }
        self.dirty = true;
    }

    /// Called when a background sub-agent is spawned.
    pub fn on_background_task_started(&mut self, task_id: String, agent_name: String) {
        self.background_tasks.push(BackgroundTaskEntry {
            task_id,
            agent_name,
            finished: false,
        });
        self.dirty = true;
    }

    /// Called when a background sub-agent finishes.
    pub fn on_background_task_done(&mut self, task_id: &str) {
        if let Some(task) = self
            .background_tasks
            .iter_mut()
            .find(|t| t.task_id == task_id)
        {
            task.finished = true;
            self.dirty = true;
        }
    }

    /// Transition to idle state on stream close.
    pub fn on_stream_close(&mut self) {
        self.state = State::Idle;
        self.dirty = true;
    }

    /// Whether the card has pending changes worth flushing.
    ///
    /// Returns `true` only when both conditions hold:
    /// 1. An event callback set the dirty flag.
    /// 2. The rendered HTML differs from the last flush.
    ///
    /// This skip-unchanged check avoids hitting the Telegram API when
    /// repeated events produce identical rendered output.
    pub fn needs_flush(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        let current = self.render();
        if current == self.last_rendered {
            self.dirty = false;
            return false;
        }
        true
    }

    /// Record the flushed HTML and clear the dirty flag.
    pub fn mark_flushed(&mut self) {
        self.last_rendered = self.render();
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_with_model_and_context() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        card.model = "claude-sonnet-4".to_string();
        card.input_tokens = 45_000;
        card.output_tokens = 12_000;
        card.thinking_ms = 5_000;
        card.tool_calls = 8;

        let text = card.render();
        // Identity bar: 🟢 mita
        assert!(text.contains("\u{1f7e2}"));
        assert!(text.contains("<b>mita</b>"));
        assert!(text.contains("<code>claude-sonnet-4</code>"));
        // Context uses input_tokens only (not input+output).
        assert!(text.contains("45.0k / 200.0k"));
        assert!(text.contains('%'));
        assert!(!text.contains("Cost:"));
        assert!(text.contains("Thinking: 5s"));
        assert!(text.contains("8 tool calls"));
    }

    #[test]
    fn render_unknown_model_omits_limit() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        card.model = "some-unknown-model".to_string();
        card.input_tokens = 10_000;
        card.output_tokens = 5_000;

        let text = card.render();
        // Context uses input_tokens only, no limit/percent for unknown model.
        assert!(text.contains("Context: 10.0k"));
        assert!(!text.contains('%'));
    }

    #[test]
    fn render_idle_state() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        card.on_stream_close();
        let text = card.render();
        // Idle uses ⚪ emoji, no "Idle" text in identity bar.
        assert!(text.contains("\u{26aa}"));
        assert!(text.contains("<b>mita</b>"));
    }

    #[test]
    fn skip_unchanged_render() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        assert!(card.needs_flush());
        card.mark_flushed();

        card.dirty = true;
        assert!(!card.needs_flush());
    }

    #[test]
    fn dirty_flag_lifecycle() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        card.mark_flushed();
        assert!(!card.needs_flush());

        card.on_tool_start();
        assert!(card.needs_flush());

        card.mark_flushed();
        assert!(!card.needs_flush());
    }

    #[test]
    fn background_tasks_render() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        card.on_background_task_started("task-1".into(), "Researcher".into());
        let text = card.render();
        assert!(text.contains("<b>Background (1)</b>"));
        assert!(text.contains("Researcher"));
    }

    #[test]
    fn background_task_done_hides_from_render() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        card.on_background_task_started("task-1".into(), "Researcher".into());
        card.on_background_task_done("task-1");
        let text = card.render();
        assert!(!text.contains("Background"));
    }

    #[test]
    fn render_changed_files() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        card.on_file_changed("src/main.rs".into(), 12, 5);
        card.on_file_changed("tests/unit.rs".into(), 8, 0);
        let text = card.render();
        assert!(text.contains("<b>Files (2)</b>"));
        assert!(text.contains("src/main.rs (+12 -5)"));
        assert!(text.contains("tests/unit.rs (+8)"));
    }

    #[test]
    fn file_change_accumulates() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        card.on_file_changed("src/main.rs".into(), 5, 2);
        card.on_file_changed("src/main.rs".into(), 3, 1);
        assert_eq!(card.changed_files.len(), 1);
        assert_eq!(card.changed_files[0].additions, 8);
        assert_eq!(card.changed_files[0].deletions, 3);
    }

    #[test]
    fn changed_files_truncated_at_10() {
        let mut card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        for i in 0..15 {
            card.on_file_changed(format!("file_{i}.rs"), 1, 0);
        }
        let text = card.render();
        assert!(text.contains("Files (15)"));
        assert!(text.contains("\u{2026} and 5 more"));
        assert!(text.contains("file_9.rs"));
        assert!(!text.contains("file_10.rs"));
    }

    #[test]
    fn short_path_trims_prefix() {
        assert_eq!(
            short_path("/Users/ryan/code/rara/src/main.rs"),
            "rara/src/main.rs"
        );
        assert_eq!(short_path("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn omits_empty_sections() {
        let card = PinnedSessionCard::new(123, "s1".into(), "mita".into());
        let text = card.render();
        assert!(!text.contains("Model:"));
        assert!(!text.contains("Context:"));
        assert!(!text.contains("Cost:"));
        assert!(!text.contains("tool calls"));
    }
}
