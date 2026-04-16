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

//! Proactive reply judgment for group chats.
//!
//! When Rara receives a group-chat message where she was **not** directly
//! mentioned, a lightweight LLM call decides whether she should reply.
//! This avoids spamming every group message through the full agent loop
//! while still allowing Rara to chime in when she has something valuable
//! to contribute.
//!
//! # Flow
//!
//! ```text
//! group message (not @-mentioned)
//!   → record to session tape (always)
//!   → build short context (last N messages)
//!   → lightweight LLM call: "should I reply?"
//!       → no  → done
//!       → yes → full agent turn (normal path)
//! ```

use tracing::{debug, info, warn};

use crate::{
    llm::{
        self,
        driver::LlmDriverRef,
        types::{CompletionRequest, ToolChoice},
    },
    memory::TapeService,
};

/// Maximum number of recent tape messages to include in the lightweight
/// judgment context.
const JUDGMENT_CONTEXT_MESSAGES: usize = 10;

/// System prompt for the lightweight proactive-reply judgment.
///
/// The LLM receives recent conversation history and must decide whether
/// Rara should reply. The prompt instructs it to return a structured
/// yes/no answer.
const JUDGMENT_SYSTEM_PROMPT: &str = r#"You are a judgment module for Rara, a personal AI assistant participating in a group chat.

Your task: Given the recent group chat messages, decide whether Rara should proactively reply to the latest message.

Rara should reply when:
- Someone asks a question Rara can answer (even if not directed at her)
- The conversation topic is directly relevant to Rara's expertise or ongoing tasks
- Someone indirectly references Rara or something she said earlier
- Rara can provide genuinely valuable information that others haven't mentioned
- The group would benefit from Rara's input

Rara should NOT reply when:
- The conversation is casual chit-chat between humans with no need for AI input
- Someone else already answered the question adequately
- The topic is personal/social between group members
- Rara has nothing substantive to add
- The message is a reaction, emoji, or acknowledgment

Respond with EXACTLY one line:
- "YES: <brief reason>" if Rara should reply
- "NO: <brief reason>" if Rara should stay silent

Be conservative — when in doubt, stay silent. Rara should feel like a helpful presence, not a noisy bot.

Note: You do not have access to Rara's full capability list or current task context. Base your judgment on the conversation content alone."#;

/// Result of the proactive reply judgment.
#[derive(Debug, Clone)]
pub enum ProactiveJudgment {
    /// Rara should reply to this message.
    ShouldReply { reason: String },
    /// Rara should stay silent.
    ShouldSkip { reason: String },
}

/// Run the lightweight LLM judgment to decide whether Rara should
/// proactively reply to a group-chat message.
///
/// Loads the last `JUDGMENT_CONTEXT_MESSAGES` from the session tape,
/// appends the new user message, and asks a small LLM call whether Rara
/// should respond.
///
/// Returns [`ProactiveJudgment::ShouldSkip`] on any error (fail-open for
/// silence — better to miss a reply than to spam).
pub async fn should_reply(
    driver: &LlmDriverRef,
    model: &str,
    tape_service: &TapeService,
    tape_name: &str,
    new_message_text: &str,
    sender_display_name: Option<&str>,
) -> ProactiveJudgment {
    // Build a short context from recent tape messages.
    let context_messages = match build_judgment_context(
        tape_service,
        tape_name,
        new_message_text,
        sender_display_name,
    )
    .await
    {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!(error = %e, "proactive: failed to build judgment context, skipping");
            return ProactiveJudgment::ShouldSkip {
                reason: "context build failed".into(),
            };
        }
    };

    // Make the lightweight LLM call.
    let request = CompletionRequest {
        model:               model.to_string(),
        messages:            context_messages,
        tools:               Vec::new(),
        temperature:         Some(0.0),
        max_tokens:          Some(100),
        thinking:            None,
        tool_choice:         ToolChoice::None,
        parallel_tool_calls: false,
        frequency_penalty:   None,
        top_p:               None,
        emit_reasoning:      false,
    };

    let response = match driver.complete(request).await {
        Ok(resp) => resp,
        Err(e) => {
            warn!(error = %e, "proactive: LLM judgment call failed, skipping");
            return ProactiveJudgment::ShouldSkip {
                reason: "LLM call failed".into(),
            };
        }
    };

    let reply_text = response.content.unwrap_or_default();
    parse_judgment(&reply_text)
}

/// Build the message list for the judgment LLM call.
///
/// Structure:
/// 1. System prompt (judgment instructions)
/// 2. Recent conversation messages (up to [`JUDGMENT_CONTEXT_MESSAGES`])
///    formatted as a single user message summarizing the chat
/// 3. The new message to evaluate
async fn build_judgment_context(
    tape_service: &TapeService,
    tape_name: &str,
    new_message_text: &str,
    sender_display_name: Option<&str>,
) -> crate::memory::TapResult<Vec<llm::Message>> {
    // Load recent messages from the tape.
    let entries = tape_service
        .from_last_anchor(tape_name, Some(&[crate::memory::TapEntryKind::Message]))
        .await?;

    // Take the last N entries for context.
    let recent: Vec<_> = if entries.len() > JUDGMENT_CONTEXT_MESSAGES {
        entries[entries.len() - JUDGMENT_CONTEXT_MESSAGES..].to_vec()
    } else {
        entries
    };

    // Format recent messages into a readable conversation summary.
    let mut conversation_lines = Vec::new();
    for entry in &recent {
        if let Some(payload) = entry.payload.as_object() {
            let role = payload
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let content = payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !content.is_empty() {
                let label = match role {
                    "assistant" => "Rara",
                    "user" => "User",
                    _ => role,
                };
                conversation_lines.push(format!("[{label}]: {content}"));
            }
        }
    }

    let sender_label = sender_display_name.unwrap_or("User");

    let mut messages = vec![llm::Message::system(JUDGMENT_SYSTEM_PROMPT)];

    if conversation_lines.is_empty() {
        // No prior context — just the new message.
        messages.push(llm::Message::user(format!(
            "New message from {sender_label}:\n{new_message_text}\n\nShould Rara reply?"
        )));
    } else {
        let conversation = conversation_lines.join("\n");
        messages.push(llm::Message::user(format!(
            "Recent conversation:\n{conversation}\n\nNew message from \
             {sender_label}:\n{new_message_text}\n\nShould Rara reply?"
        )));
    }

    Ok(messages)
}

/// Parse the LLM response into a [`ProactiveJudgment`].
///
/// Expected format: "YES: reason" or "NO: reason".
/// Defaults to skip on unparseable responses.
fn parse_judgment(text: &str) -> ProactiveJudgment {
    let trimmed = text.trim();
    let upper = trimmed.to_uppercase();

    if upper.starts_with("YES") {
        let reason = trimmed
            .get(3..)
            .map(|s| s.trim_start_matches(':').trim())
            .unwrap_or("")
            .to_string();
        info!(reason = %reason, "proactive: judgment = REPLY");
        ProactiveJudgment::ShouldReply {
            reason: if reason.is_empty() {
                "LLM decided to reply".into()
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
            debug!(raw_response = %trimmed, "proactive: unparseable judgment, defaulting to skip");
            format!("unparseable response: {}", truncate(trimmed, 80))
        };
        info!(reason = %reason, "proactive: judgment = SKIP");
        ProactiveJudgment::ShouldSkip {
            reason: if reason.is_empty() {
                "LLM decided to stay silent".into()
            } else {
                reason
            },
        }
    }
}

/// Truncate a string to at most `max` characters.
fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yes_with_reason() {
        match parse_judgment("YES: user asked about Rust") {
            ProactiveJudgment::ShouldReply { reason } => {
                assert!(reason.contains("Rust"));
            }
            other => panic!("expected ShouldReply, got {other:?}"),
        }
    }

    #[test]
    fn parse_no_with_reason() {
        match parse_judgment("NO: casual conversation") {
            ProactiveJudgment::ShouldSkip { reason } => {
                assert!(reason.contains("casual"));
            }
            other => panic!("expected ShouldSkip, got {other:?}"),
        }
    }

    #[test]
    fn parse_yes_lowercase() {
        match parse_judgment("yes: relevant topic") {
            ProactiveJudgment::ShouldReply { reason } => {
                assert!(reason.contains("relevant"));
            }
            other => panic!("expected ShouldReply, got {other:?}"),
        }
    }

    #[test]
    fn parse_garbage_defaults_to_skip() {
        match parse_judgment("I'm not sure what to do") {
            ProactiveJudgment::ShouldSkip { .. } => {}
            other => panic!("expected ShouldSkip, got {other:?}"),
        }
    }

    #[test]
    fn parse_empty_defaults_to_skip() {
        match parse_judgment("") {
            ProactiveJudgment::ShouldSkip { .. } => {}
            other => panic!("expected ShouldSkip, got {other:?}"),
        }
    }
}
