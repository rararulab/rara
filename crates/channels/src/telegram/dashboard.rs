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

//! Telegram inline-message Dashboard.
//!
//! Renders a tab-based status view inside a single Telegram message, using
//! `editMessageText` + `InlineKeyboardMarkup` for navigation.  No external
//! URL or Mini App required — the entire UI lives in native Telegram messages.

use rara_kernel::session::{SessionKey, SessionState, SessionStats};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

// ── Tab enum ────────────────────────────────────────────────────────────

/// Dashboard tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashTab {
    Tasks,
    Sessions,
}

impl DashTab {
    /// Parse from the callback data segment (e.g. `"tasks"` / `"sess"`).
    pub fn from_str_prefix(s: &str) -> Self {
        match s {
            "sess" => Self::Sessions,
            _ => Self::Tasks,
        }
    }

    /// Short string used in callback data (must be short — 64-byte limit).
    fn callback_key(self) -> &'static str {
        match self {
            Self::Tasks => "tasks",
            Self::Sessions => "sess",
        }
    }
}

// ── Session scoping ─────────────────────────────────────────────────────

/// Filter `all_sessions` to only those belonging to `root` and its children.
///
/// This enforces per-chat isolation: a Telegram chat should only see the
/// session it is bound to (the root) and any children spawned from it.
pub fn scoped_sessions(all_sessions: &[SessionStats], root: SessionKey) -> Vec<&SessionStats> {
    all_sessions
        .iter()
        .filter(|s| s.session_key == root || s.parent_id == Some(root))
        .collect()
}

// ── Rendering ───────────────────────────────────────────────────────────

/// Render the full dashboard message body (HTML) for the given tab.
///
/// The `sessions` slice should already be scoped to the requesting chat
/// via [`scoped_sessions`].
pub fn render_dashboard(tab: DashTab, sessions: &[&SessionStats]) -> String {
    let mut out = String::with_capacity(1024);

    match tab {
        DashTab::Tasks => render_tasks_tab(sessions, &mut out),
        DashTab::Sessions => render_sessions_tab(sessions, &mut out),
    }

    truncate_html_safe(&mut out, 4000);
    out
}

/// Build the inline keyboard for the dashboard.
///
/// Layout row 1: `[📋 Tasks] [🖥 Sessions] [🔄]`
/// Layout row 2 (if trace_id present): `[← Back]`
///
/// When `trace_id` is `Some`, a second row with a Back button is added so
/// users can return to the execution trace view.  The active tab gets a `·`
/// suffix.
pub fn dashboard_keyboard(
    active_tab: DashTab,
    chat_id: i64,
    msg_id: i32,
    trace_id: Option<&str>,
) -> InlineKeyboardMarkup {
    let tasks_label = if active_tab == DashTab::Tasks {
        "\u{1f4cb} Tasks \u{b7}"
    } else {
        "\u{1f4cb} Tasks"
    };
    let sess_label = if active_tab == DashTab::Sessions {
        "\u{1f5a5} Sessions \u{b7}"
    } else {
        "\u{1f5a5} Sessions"
    };

    // Embed trace_id in tab callbacks so it survives tab switches.
    let tid = trace_id.unwrap_or("-");
    let tasks_cb = format!("dash:tasks:{chat_id}:{msg_id}:{tid}");
    let sess_cb = format!("dash:sess:{chat_id}:{msg_id}:{tid}");
    let refresh_cb = format!(
        "dash:{}:{chat_id}:{msg_id}:{tid}",
        active_tab.callback_key()
    );

    let mut rows = vec![vec![
        InlineKeyboardButton::callback(tasks_label, tasks_cb),
        InlineKeyboardButton::callback(sess_label, sess_cb),
        InlineKeyboardButton::callback("\u{1f504}", refresh_cb),
    ]];

    // Back button to restore trace view — only if we have a real trace_id.
    if let Some(tid) = trace_id {
        let back_cb = format!("trace:hide:{chat_id}:{msg_id}:{tid}");
        rows.push(vec![InlineKeyboardButton::callback(
            "\u{2190} Back",
            back_cb,
        )]);
    }

    InlineKeyboardMarkup::new(rows)
}

// ── Tasks tab ───────────────────────────────────────────────────────────

fn render_tasks_tab(sessions: &[&SessionStats], out: &mut String) {
    out.push_str("\u{1f4ca} <b>rara \u{b7} Tasks</b>\n");
    out.push_str("───────────────\n");

    // Background tasks = child sessions (parent_id.is_some()).
    let children: Vec<&&SessionStats> = sessions.iter().filter(|s| s.parent_id.is_some()).collect();

    if children.is_empty() {
        out.push_str("\nNo background tasks.\n");
        return;
    }

    let running: Vec<&&&SessionStats> = children
        .iter()
        .filter(|s| matches!(s.state, SessionState::Active | SessionState::Ready))
        .collect();
    let done: Vec<&&&SessionStats> = children
        .iter()
        .filter(|s| matches!(s.state, SessionState::Suspended | SessionState::Paused))
        .collect();

    if !running.is_empty() {
        out.push_str(&format!("\n\u{1f504} <b>Running ({})</b>\n", running.len()));
        for s in &running {
            push_task_line(s, false, out);
        }
    }

    if !done.is_empty() {
        out.push_str(&format!("\n\u{2705} <b>Done ({})</b>\n", done.len()));
        // Show only the most recent 10 to stay within message limits.
        for s in done.iter().rev().take(10) {
            push_task_line(s, true, out);
        }
        if done.len() > 10 {
            out.push_str(&format!("  <i>… and {} more</i>\n", done.len() - 10));
        }
    }
}

fn push_task_line(s: &SessionStats, finished: bool, out: &mut String) {
    let icon = if finished {
        "\u{2714}"
    } else {
        "\u{23f3} \u{1f916}"
    };
    let uptime = format_uptime(s.uptime_ms);
    let name = html_escape(&s.manifest_name);

    if finished {
        out.push_str(&format!("{icon} {name} \u{2014} {uptime}\n"));
    } else {
        out.push_str(&format!(
            "{icon} {name}\n   {uptime} \u{b7} {} tools\n",
            s.tool_calls,
        ));
    }
}

// ── Sessions tab ────────────────────────────────────────────────────────

fn render_sessions_tab(sessions: &[&SessionStats], out: &mut String) {
    out.push_str("\u{1f4ca} <b>rara \u{b7} Sessions</b>\n");
    out.push_str("───────────────\n");

    if sessions.is_empty() {
        out.push_str("\nNo active sessions.\n");
        return;
    }

    out.push('\n');
    for s in sessions {
        let (icon, state_label) = match s.state {
            SessionState::Active => ("\u{25b6}", "Running"),
            SessionState::Ready => ("\u{25b6}", "Idle"),
            SessionState::Suspended => ("\u{23f8}\u{fe0f}", "Suspended"),
            SessionState::Paused => ("\u{23f8}\u{fe0f}", "Paused"),
        };
        let name = html_escape(&s.manifest_name);
        let uptime = format_uptime(s.uptime_ms);
        let child_hint = if s.parent_id.is_some() {
            " (child)"
        } else {
            ""
        };

        out.push_str(&format!(
            "{icon} <b>{name}</b>{child_hint} \u{2014} {state_label} \u{b7} {uptime}\n",
        ));
        out.push_str(&format!(
            "  \u{2191}{} \u{2193}{} \u{b7} {} tools\n",
            format_tokens(s.tokens_consumed / 2), // rough split: half in half out
            format_tokens(s.tokens_consumed / 2),
            s.tool_calls,
        ));
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn format_uptime(ms: u64) -> String {
    let total_sec = ms / 1000;
    let hours = total_sec / 3600;
    let minutes = (total_sec % 3600) / 60;
    let seconds = total_sec % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}

/// Escape HTML entities for Telegram HTML parse mode.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Truncate an HTML string at a safe boundary: on a char boundary that is
/// not inside an HTML tag or entity.  Falls back to the last `\n` before
/// `max_len` to avoid splitting a line mid-tag.
fn truncate_html_safe(s: &mut String, max_len: usize) {
    if s.len() <= max_len {
        return;
    }
    // Find the last newline at or before max_len (char-safe since '\n' is ASCII).
    let cut = s[..max_len]
        .rfind('\n')
        .unwrap_or_else(|| s.floor_char_boundary(max_len));
    s.truncate(cut);
    s.push_str("\n<i>… truncated</i>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_tab_roundtrip() {
        assert_eq!(DashTab::from_str_prefix("tasks"), DashTab::Tasks);
        assert_eq!(DashTab::from_str_prefix("sess"), DashTab::Sessions);
        assert_eq!(DashTab::from_str_prefix("unknown"), DashTab::Tasks);
    }

    #[test]
    fn keyboard_callback_data_within_64_bytes() {
        use teloxide::types::InlineKeyboardButtonKind;
        // Worst case: supergroup chat_id ~14 digits, msg_id ~10 digits,
        // trace_id = ULID (26 chars).
        let tid = "01JRWQY1234567890ABCDEFGH";
        let kb = dashboard_keyboard(DashTab::Tasks, -1001234567890, 2147483647, Some(tid));
        for row in &kb.inline_keyboard {
            for btn in row {
                if let InlineKeyboardButtonKind::CallbackData(ref data) = btn.kind {
                    assert!(
                        data.len() <= 64,
                        "callback data too long ({} bytes): {data}",
                        data.len(),
                    );
                }
            }
        }
    }

    #[test]
    fn html_escape_special_chars() {
        assert_eq!(html_escape("<b>&test</b>"), "&lt;b&gt;&amp;test&lt;/b&gt;");
    }

    #[test]
    fn format_uptime_values() {
        assert_eq!(format_uptime(500), "0s");
        assert_eq!(format_uptime(5_000), "5s");
        assert_eq!(format_uptime(65_000), "1m 5s");
        assert_eq!(format_uptime(3_665_000), "1h 1m");
    }

    #[test]
    fn render_empty_tasks() {
        let text = render_dashboard(DashTab::Tasks, &[]);
        assert!(text.contains("No background tasks"));
    }

    #[test]
    fn truncate_html_safe_preserves_valid_html() {
        let mut s = "line1\nline2\nline3\nline4".to_string();
        truncate_html_safe(&mut s, 12);
        // Should cut at newline before pos 12, not mid-line.
        assert!(s.starts_with("line1\nline2"));
        assert!(s.ends_with("truncated</i>"));
    }

    #[test]
    fn truncate_html_safe_noop_when_short() {
        let mut s = "short".to_string();
        truncate_html_safe(&mut s, 4000);
        assert_eq!(s, "short");
    }

    #[test]
    fn render_empty_sessions() {
        let text = render_dashboard(DashTab::Sessions, &[]);
        assert!(text.contains("No active sessions"));
    }
}
