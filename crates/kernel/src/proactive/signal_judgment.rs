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

//! Lightweight LLM pre-judgment for proactive signals.
//!
//! Before routing a proactive signal through a full Mita agent turn, a
//! cheap LLM call decides whether the signal is worth acting on. This
//! avoids waking Mita for signals that would just produce noise.
//!
//! # Flow
//!
//! ```text
//! proactive signal
//!   -> rule filter (quiet hours, cooldowns, rate limit)
//!   -> context pack built
//!   -> lightweight LLM call: "should Mita act?"
//!       -> no  -> silently drop
//!       -> yes -> deliver to Mita (full agent turn)
//! ```

use tracing::{debug, info, warn};

use crate::llm::{
    self,
    driver::LlmDriverRef,
    types::{CompletionRequest, ToolChoice},
};

/// System prompt for the lightweight signal judgment.
const SIGNAL_JUDGMENT_PROMPT: &str = r#"You are a judgment module for Mita, a background orchestration agent.

Your task: Given a proactive event and its context, decide whether Mita should take action.

Mita should act when:
- The user likely needs a reminder or follow-up
- Important information should be communicated proactively
- The event indicates something the user would want to know about

Mita should NOT act when:
- The idle session has no actionable context (user just said "ok" or "thanks")
- The event is routine and doesn't need user attention
- Acting would be annoying rather than helpful

Respond with EXACTLY one line:
- "YES: <brief reason>" if Mita should act on this event
- "NO: <brief reason>" if this event should be silently dropped

Be conservative — when in doubt, drop. The user should feel helped, not pestered."#;

/// Result of the lightweight signal judgment.
#[derive(Debug, Clone)]
pub enum SignalJudgment {
    /// Mita should act on this signal.
    ShouldAct { reason: String },
    /// The signal should be silently dropped.
    ShouldDrop { reason: String },
}

/// Run a lightweight LLM judgment to decide whether a proactive signal
/// is worth a full Mita agent turn.
///
/// The `context_pack` should be the already-built context string that
/// would be delivered to Mita.
///
/// Returns [`SignalJudgment::ShouldDrop`] on any error — better to
/// silently drop than to spam the user.
pub async fn should_act(driver: &LlmDriverRef, model: &str, context_pack: &str) -> SignalJudgment {
    let messages = vec![
        llm::Message::system(SIGNAL_JUDGMENT_PROMPT),
        llm::Message::user(format!(
            "Proactive event context:\n{context_pack}\n\nShould Mita act on this event?"
        )),
    ];

    let request = CompletionRequest {
        model: model.to_string(),
        messages,
        tools: Vec::new(),
        temperature: Some(0.0),
        max_tokens: Some(100),
        thinking: None,
        tool_choice: ToolChoice::None,
        parallel_tool_calls: false,
        frequency_penalty: None,
    };

    let response = match driver.complete(request).await {
        Ok(resp) => resp,
        Err(e) => {
            warn!(error = %e, "signal judgment: LLM call failed, dropping signal");
            return SignalJudgment::ShouldDrop {
                reason: "LLM call failed".into(),
            };
        }
    };

    let reply_text = response.content.unwrap_or_default();
    parse_signal_judgment(&reply_text)
}

/// Parse the LLM response into a [`SignalJudgment`].
///
/// Expected format: `"YES: reason"` or `"NO: reason"`.
/// Defaults to drop on unparseable responses.
fn parse_signal_judgment(text: &str) -> SignalJudgment {
    let trimmed = text.trim();
    let upper = trimmed.to_uppercase();

    if upper.starts_with("YES") {
        let reason = trimmed
            .get(3..)
            .map(|s| s.trim_start_matches(':').trim())
            .unwrap_or("")
            .to_string();
        info!(reason = %reason, "signal judgment = ACT");
        SignalJudgment::ShouldAct {
            reason: if reason.is_empty() {
                "LLM decided to act".into()
            } else {
                reason
            },
        }
    } else {
        let reason = if upper.starts_with("NO") {
            trimmed
                .get(2..)
                .map(|s| s.trim_start_matches(':').trim())
                .unwrap_or("")
                .to_string()
        } else {
            debug!(raw_response = %trimmed, "signal judgment: unparseable response, defaulting to drop");
            format!("unparseable response: {}", truncate(trimmed, 80))
        };
        info!(reason = %reason, "signal judgment = DROP");
        SignalJudgment::ShouldDrop {
            reason: if reason.is_empty() {
                "LLM decided to drop".into()
            } else {
                reason
            },
        }
    }
}

use super::truncate;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yes_with_reason() {
        match parse_signal_judgment("YES: user has a pending task reminder") {
            SignalJudgment::ShouldAct { reason } => {
                assert!(reason.contains("pending task"));
            }
            other => panic!("expected ShouldAct, got {other:?}"),
        }
    }

    #[test]
    fn parse_no_with_reason() {
        match parse_signal_judgment("NO: user just said thanks, nothing actionable") {
            SignalJudgment::ShouldDrop { reason } => {
                assert!(reason.contains("thanks"));
            }
            other => panic!("expected ShouldDrop, got {other:?}"),
        }
    }

    #[test]
    fn parse_yes_lowercase() {
        match parse_signal_judgment("yes: important deadline approaching") {
            SignalJudgment::ShouldAct { reason } => {
                assert!(reason.contains("deadline"));
            }
            other => panic!("expected ShouldAct, got {other:?}"),
        }
    }

    #[test]
    fn parse_no_bare() {
        match parse_signal_judgment("NO") {
            SignalJudgment::ShouldDrop { reason } => {
                assert_eq!(reason, "LLM decided to drop");
            }
            other => panic!("expected ShouldDrop, got {other:?}"),
        }
    }

    #[test]
    fn parse_garbage_defaults_to_drop() {
        match parse_signal_judgment("I'm not sure what to do here") {
            SignalJudgment::ShouldDrop { .. } => {}
            other => panic!("expected ShouldDrop, got {other:?}"),
        }
    }

    #[test]
    fn parse_empty_defaults_to_drop() {
        match parse_signal_judgment("") {
            SignalJudgment::ShouldDrop { .. } => {}
            other => panic!("expected ShouldDrop, got {other:?}"),
        }
    }
}
