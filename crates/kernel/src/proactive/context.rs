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

//! Structured context pack builder for Mita proactive signals.
//!
//! Converts a [`ProactiveSignal`] + optional session context into a
//! human-readable structured text block that Mita can reason about.

use jiff::Timestamp;

use super::{signal::ProactiveSignal, truncate};

/// Shared available actions block appended to all context packs.
const AVAILABLE_ACTIONS: &str = "[Available Actions]\n- dispatch_rara: send a message to user \
                                 through a session\n- notify: push notification to user's \
                                 device\n- (no action): decide this event doesn't need \
                                 intervention";

/// Recent Mita action history for context pack.
#[derive(Debug, Clone)]
pub struct MitaHistory {
    /// Recent Mita actions as human-readable lines.
    pub recent_actions: Vec<String>,
}

/// Optional session context attached to a proactive signal.
#[derive(Debug, Clone)]
pub struct SessionContext {
    /// Human-readable session name (if available).
    pub session_name:      Option<String>,
    /// Session key for routing.
    pub session_key:       String,
    /// When the session became idle (human-readable).
    pub idle_since:        Option<String>,
    /// Last user message in the session (for context).
    pub last_user_message: Option<String>,
}

/// Build a structured context pack for Mita from a proactive signal.
///
/// The output is a multi-section text block that Mita can parse to
/// decide what action (if any) to take.
pub fn build_context_pack(
    signal: &ProactiveSignal,
    session_context: Option<&SessionContext>,
    mita_history: Option<&MitaHistory>,
    available_tools: Option<&[String]>,
) -> String {
    let now = Timestamp::now();
    let mut sections = Vec::new();

    // [Proactive Event] section
    sections.push(format!(
        "[Proactive Event]\nkind: {}\ntimestamp: {}",
        signal.kind_name(),
        now,
    ));

    // Signal-specific details
    match signal {
        ProactiveSignal::SessionIdle { idle_duration, .. } => {
            let mins = idle_duration.as_secs() / 60;
            sections
                .last_mut()
                .expect("sections is non-empty")
                .push_str(&format!("\nidle_duration: {}m", mins,));
        }
        ProactiveSignal::TaskFailed { error } => {
            sections
                .last_mut()
                .expect("sections is non-empty")
                .push_str(&format!("\nerror: {}", truncate(error, 200),));
        }
        ProactiveSignal::SessionCompleted { summary, .. } => {
            sections
                .last_mut()
                .expect("sections is non-empty")
                .push_str(&format!("\nsummary: {}", truncate(summary, 200),));
        }
        ProactiveSignal::MorningGreeting | ProactiveSignal::DailySummary => {
            // No extra fields for time events.
        }
    }

    // [Context] section (if session context is available)
    if let Some(ctx) = session_context {
        let mut context_lines = Vec::new();
        let name_display = ctx
            .session_name
            .as_deref()
            .map(|n| format!("\"{}\" ({})", n, ctx.session_key))
            .unwrap_or_else(|| ctx.session_key.clone());
        context_lines.push(format!("session: {}", name_display));

        if let Some(idle) = &ctx.idle_since {
            context_lines.push(format!("idle_since: {}", idle));
        }
        if let Some(msg) = &ctx.last_user_message {
            context_lines.push(format!("last_user_message: \"{}\"", truncate(msg, 100),));
        }
        sections.push(format!("[Context]\n{}", context_lines.join("\n")));
    }

    // [Mita History] section
    if let Some(history) = mita_history {
        if !history.recent_actions.is_empty() {
            sections.push(format!(
                "[Mita History]\n{}",
                history.recent_actions.join("\n")
            ));
        }
    }

    // [Available Actions] section — prefer dynamic tool list from agent manifest.
    if let Some(tools) = available_tools {
        let tool_lines: Vec<String> = tools.iter().map(|t| format!("- {t}")).collect();
        sections.push(format!(
            "[Available Actions]\n{}\n- (no action): decide this event doesn't need intervention",
            tool_lines.join("\n")
        ));
    } else {
        sections.push(AVAILABLE_ACTIONS.to_string());
    }

    sections.join("\n\n")
}

/// Build a structured context pack for a heartbeat patrol.
///
/// Replaces the previous one-line heartbeat message with the same
/// structured format used by proactive signals.
pub fn build_heartbeat_context_pack(
    active_session_count: usize,
    mita_history: Option<&MitaHistory>,
    available_tools: Option<&[String]>,
) -> String {
    let now = Timestamp::now();
    let mut sections = Vec::new();

    sections.push(format!(
        "[Proactive Event]\nkind: heartbeat_patrol\ntimestamp: {}",
        now,
    ));

    sections.push(format!(
        "[Context]\nactive_sessions: {}\nAnalyze active sessions and determine if any proactive \
         actions are needed. Review your previous tape entries to avoid repeating recent actions.",
        active_session_count,
    ));

    // [Mita History] section
    if let Some(history) = mita_history {
        if !history.recent_actions.is_empty() {
            sections.push(format!(
                "[Mita History]\n{}",
                history.recent_actions.join("\n")
            ));
        }
    }

    if let Some(tools) = available_tools {
        let tool_lines: Vec<String> = tools.iter().map(|t| format!("- {t}")).collect();
        sections.push(format!(
            "[Available Actions]\n{}\n- (no action): decide this event doesn't need intervention",
            tool_lines.join("\n")
        ));
    } else {
        sections.push(AVAILABLE_ACTIONS.to_string());
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn context_pack_session_idle() {
        use crate::session::SessionKey;

        let signal = ProactiveSignal::SessionIdle {
            session_key:   SessionKey::deterministic("session-abc"),
            idle_duration: Duration::from_secs(7200),
        };
        let ctx = SessionContext {
            session_name:      Some("PR review".to_string()),
            session_key:       "session-abc".to_string(),
            idle_since:        Some("2h ago".to_string()),
            last_user_message: Some("check that PR".to_string()),
        };
        let pack = build_context_pack(&signal, Some(&ctx), None, None);
        assert!(pack.contains("kind: session_idle"));
        assert!(pack.contains("idle_duration: 120m"));
        assert!(pack.contains("\"PR review\""));
        assert!(pack.contains("[Available Actions]"));
    }

    #[test]
    fn context_pack_morning_greeting() {
        let signal = ProactiveSignal::MorningGreeting;
        let pack = build_context_pack(&signal, None, None, None);
        assert!(pack.contains("kind: morning_greeting"));
        assert!(pack.contains("[Available Actions]"));
        // No [Context] section when no session context.
        assert!(!pack.contains("[Context]"));
    }

    #[test]
    fn heartbeat_context_pack() {
        let pack = build_heartbeat_context_pack(3, None, None);
        assert!(pack.contains("kind: heartbeat_patrol"));
        assert!(pack.contains("active_sessions: 3"));
    }

    #[test]
    fn context_pack_with_mita_history() {
        let signal = ProactiveSignal::MorningGreeting;
        let history = MitaHistory {
            recent_actions: vec![
                "2026-03-21T08:00:00Z: called dispatch_rara".to_string(),
                "2026-03-21T07:30:00Z: called notify".to_string(),
            ],
        };
        let pack = build_context_pack(&signal, None, Some(&history), None);
        assert!(pack.contains("[Mita History]"));
        assert!(pack.contains("called dispatch_rara"));
        assert!(pack.contains("called notify"));
        // History should appear before Available Actions.
        let history_pos = pack.find("[Mita History]").expect("history section exists");
        let actions_pos = pack
            .find("[Available Actions]")
            .expect("actions section exists");
        assert!(history_pos < actions_pos);
    }

    #[test]
    fn context_pack_empty_history_omitted() {
        let signal = ProactiveSignal::MorningGreeting;
        let history = MitaHistory {
            recent_actions: vec![],
        };
        let pack = build_context_pack(&signal, None, Some(&history), None);
        assert!(!pack.contains("[Mita History]"));
    }

    #[test]
    fn context_pack_with_dynamic_tools() {
        let signal = ProactiveSignal::MorningGreeting;
        let tools = vec![
            "dispatch_rara".to_string(),
            "notify".to_string(),
            "schedule_reminder".to_string(),
        ];
        let pack = build_context_pack(&signal, None, None, Some(&tools));
        assert!(pack.contains("[Available Actions]"));
        assert!(pack.contains("- dispatch_rara"));
        assert!(pack.contains("- notify"));
        assert!(pack.contains("- schedule_reminder"));
        assert!(pack.contains("- (no action)"));
        // Should NOT contain the hardcoded fallback wording.
        assert!(!pack.contains("push notification to user"));
    }

    #[test]
    fn heartbeat_context_pack_with_dynamic_tools() {
        let tools = vec!["dispatch_rara".to_string(), "notify".to_string()];
        let pack = build_heartbeat_context_pack(1, None, Some(&tools));
        assert!(pack.contains("- dispatch_rara"));
        assert!(pack.contains("- notify"));
        assert!(pack.contains("- (no action)"));
    }

    #[test]
    fn heartbeat_context_pack_with_history() {
        let history = MitaHistory {
            recent_actions: vec!["2026-03-21T09:00:00Z: called notify".to_string()],
        };
        let pack = build_heartbeat_context_pack(2, Some(&history), None);
        assert!(pack.contains("[Mita History]"));
        assert!(pack.contains("called notify"));
    }
}
