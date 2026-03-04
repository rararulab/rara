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

//! Context compaction — the "virtual memory" of the Agent OS.
//!
//! When a conversation grows beyond the agent's token budget, the kernel
//! applies a [`CompactionStrategy`] to trim the in-memory history before
//! forwarding it to the LLM.
//!
//! The simplest strategy, [`SlidingWindowCompaction`], keeps the system
//! prompt + most recent messages and replaces older messages with a
//! single summary message.

use async_trait::async_trait;

use crate::channel::types::{ChatMessage, MessageRole};

// ---------------------------------------------------------------------------
// Token counting
// ---------------------------------------------------------------------------

/// Default token budget when the manifest does not specify one.
pub const DEFAULT_MAX_CONTEXT_TOKENS: usize = 8192;

/// Estimate the number of tokens in a string.
///
/// Uses the simple heuristic `tokens ≈ chars / 4`, which is a reasonable
/// approximation for English text and mixed-language content. Good enough
/// for budget checks without pulling in a full tokenizer.
#[inline]
pub fn token_count(text: &str) -> usize {
    // Ceiling division so even short strings count as at least 1 token.
    (text.len() + 3) / 4
}

/// Estimate the total token count of a slice of [`ChatMessage`]s.
pub fn messages_token_count(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| token_count(&m.content.as_text()))
        .sum()
}

// ---------------------------------------------------------------------------
// CompactionStrategy trait
// ---------------------------------------------------------------------------

/// Strategy for compacting a conversation history that exceeds the token
/// budget.
///
/// Implementations receive the full conversation history and must return a
/// compacted version that fits within `max_tokens`. The compacted history
/// must preserve semantic correctness (e.g., system messages stay first,
/// tool-call/tool-result pairs are not split).
#[async_trait]
pub trait CompactionStrategy: Send + Sync {
    /// Compact a conversation history to fit within `max_tokens`.
    ///
    /// Returns the compacted message list. If the history already fits,
    /// implementations should return it unchanged.
    async fn compact(&self, messages: Vec<ChatMessage>, max_tokens: usize) -> Vec<ChatMessage>;
}

// ---------------------------------------------------------------------------
// SlidingWindowCompaction
// ---------------------------------------------------------------------------

/// Simplest compaction strategy: keep system messages + the most recent
/// messages, summarize evicted middle messages into a single synthetic
/// system note.
///
/// The algorithm:
/// 1. Separate system messages (always preserved) from non-system messages.
/// 2. If total tokens fit within budget, return as-is.
/// 3. Otherwise, keep the last `N` non-system messages that fit the budget
///    (reserving room for the summary message), and replace everything before
///    them with a summary.
pub struct SlidingWindowCompaction;

impl SlidingWindowCompaction {
    /// Build a short summary of evicted messages.
    fn summarize_evicted(evicted: &[ChatMessage]) -> String {
        let user_count = evicted
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .count();
        let assistant_count = evicted
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
            .count();
        let tool_count = evicted
            .iter()
            .filter(|m| matches!(m.role, MessageRole::Tool | MessageRole::ToolResult))
            .count();

        let mut summary = format!(
            "[Earlier conversation compacted: {user_count} user message(s), {assistant_count} \
             assistant message(s)"
        );
        if tool_count > 0 {
            summary.push_str(&format!(", {tool_count} tool interaction(s)"));
        }
        summary.push_str(". Key points: ");

        // Append a brief excerpt from the last evicted user and assistant
        // messages to preserve some context.
        if let Some(last_user) = evicted.iter().rev().find(|m| m.role == MessageRole::User) {
            let text = last_user.content.as_text();
            let excerpt = truncate_str(&text, 200);
            summary.push_str(&format!("Last user topic: \"{excerpt}\". "));
        }
        if let Some(last_assistant) = evicted
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::Assistant)
        {
            let text = last_assistant.content.as_text();
            let excerpt = truncate_str(&text, 200);
            summary.push_str(&format!("Last assistant response: \"{excerpt}\". "));
        }

        summary.push(']');
        summary
    }
}

/// Truncate a string to at most `max_chars` characters, appending "..." if
/// truncated.
fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        // Find a safe char boundary
        let boundary = s
            .char_indices()
            .take_while(|(i, _)| *i < max_chars)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!("{}...", &s[..boundary])
    }
}

#[async_trait]
impl CompactionStrategy for SlidingWindowCompaction {
    async fn compact(&self, messages: Vec<ChatMessage>, max_tokens: usize) -> Vec<ChatMessage> {
        // Step 1: separate system messages from the rest.
        let (system_msgs, non_system_msgs): (Vec<_>, Vec<_>) = messages
            .into_iter()
            .partition(|m| m.role == MessageRole::System);

        let system_tokens: usize = system_msgs
            .iter()
            .map(|m| token_count(&m.content.as_text()))
            .sum();

        // If everything already fits, return unchanged.
        let non_system_tokens: usize = non_system_msgs
            .iter()
            .map(|m| token_count(&m.content.as_text()))
            .sum();
        let total = system_tokens + non_system_tokens;
        if total <= max_tokens {
            let mut result = system_msgs;
            result.extend(non_system_msgs);
            return result;
        }

        // Step 2: determine how many recent messages we can keep.
        // Reserve some budget for the summary message (~100 tokens).
        let summary_budget = 100;
        let available = max_tokens.saturating_sub(system_tokens + summary_budget);

        // Walk backwards through non-system messages, accumulating tokens.
        let mut keep_start = non_system_msgs.len();
        let mut kept_tokens = 0usize;
        for (i, msg) in non_system_msgs.iter().enumerate().rev() {
            let msg_tokens = token_count(&msg.content.as_text());
            if kept_tokens + msg_tokens > available {
                break;
            }
            kept_tokens += msg_tokens;
            keep_start = i;
        }

        // Ensure we keep at least the very last message.
        if keep_start >= non_system_msgs.len() && !non_system_msgs.is_empty() {
            keep_start = non_system_msgs.len() - 1;
        }

        // Step 3: build result.
        let evicted = &non_system_msgs[..keep_start];
        let kept = &non_system_msgs[keep_start..];

        let mut result = system_msgs;

        if !evicted.is_empty() {
            let summary_text = Self::summarize_evicted(evicted);
            result.push(ChatMessage::system(summary_text));
        }

        result.extend(kept.iter().cloned());
        result
    }
}

/// Apply compaction to a conversation if it exceeds the token budget.
///
/// This is the main entry point called by the kernel's process loop before
/// each agent turn. If no compaction strategy is provided or the
/// conversation fits within budget, the messages are returned unchanged.
pub async fn maybe_compact(
    messages: Vec<ChatMessage>,
    max_tokens: usize,
    strategy: &dyn CompactionStrategy,
) -> Vec<ChatMessage> {
    let current_tokens = messages_token_count(&messages);
    if current_tokens <= max_tokens {
        return messages;
    }

    tracing::info!(
        current_tokens,
        max_tokens,
        message_count = messages.len(),
        "compacting conversation history"
    );

    strategy.compact(messages, max_tokens).await
}
