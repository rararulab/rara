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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::types::ChatMessage;

    #[test]
    fn test_token_count_empty() {
        assert_eq!(token_count(""), 0);
    }

    #[test]
    fn test_token_count_short() {
        // "hello" = 5 chars → ceil(5/4) = 2
        assert_eq!(token_count("hello"), 2);
    }

    #[test]
    fn test_token_count_exact_multiple() {
        // 8 chars → 8/4 = 2
        assert_eq!(token_count("abcdefgh"), 2);
    }

    #[test]
    fn test_token_count_longer() {
        // 100 chars → 25 tokens
        let text = "a".repeat(100);
        assert_eq!(token_count(&text), 25);
    }

    #[test]
    fn test_messages_token_count() {
        let messages = vec![
            ChatMessage::user("hello"),      // 2 tokens
            ChatMessage::assistant("world"), // 2 tokens
        ];
        assert_eq!(messages_token_count(&messages), 4);
    }

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_exact() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_str_truncated() {
        let result = truncate_str("hello world", 5);
        assert_eq!(result, "hello...");
    }

    #[tokio::test]
    async fn test_sliding_window_no_compaction_needed() {
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hi"),
            ChatMessage::assistant("Hello!"),
        ];
        let original_len = messages.len();

        let result = SlidingWindowCompaction.compact(messages, 10000).await;

        // Should return unchanged
        assert_eq!(result.len(), original_len);
        assert_eq!(result[0].role, MessageRole::System);
        assert_eq!(result[1].role, MessageRole::User);
        assert_eq!(result[2].role, MessageRole::Assistant);
    }

    #[tokio::test]
    async fn test_sliding_window_compacts_when_over_budget() {
        // Create a conversation that exceeds a small budget.
        let mut messages = vec![ChatMessage::system("You are helpful.")];
        for i in 0..20 {
            messages.push(ChatMessage::user(format!(
                "This is user message number {i} with some extra content to inflate token count."
            )));
            messages.push(ChatMessage::assistant(format!(
                "This is the assistant reply to message {i} with additional detail."
            )));
        }

        let total_before = messages_token_count(&messages);
        assert!(total_before > 500);

        // Compact to a very tight budget.
        let result = SlidingWindowCompaction.compact(messages, 300).await;

        // Result should be smaller.
        let total_after = messages_token_count(&result);
        assert!(
            total_after <= 300 + 100, // allow summary overhead
            "compacted tokens ({total_after}) should be near the budget (300)"
        );

        // System message should be preserved as first.
        assert_eq!(result[0].role, MessageRole::System);
        assert_eq!(result[0].content.as_text(), "You are helpful.");

        // There should be a summary system message.
        let has_summary = result
            .iter()
            .any(|m| m.role == MessageRole::System && m.content.as_text().contains("compacted"));
        assert!(has_summary, "should contain a summary message");

        // The last message should be the most recent assistant message.
        let last = result.last().unwrap();
        assert_eq!(last.role, MessageRole::Assistant);
    }

    #[tokio::test]
    async fn test_sliding_window_preserves_system_messages() {
        let messages = vec![
            ChatMessage::system("System prompt 1."),
            ChatMessage::system("System prompt 2."),
            ChatMessage::user("Long message ".repeat(100).trim().to_string()),
            ChatMessage::assistant("Long reply ".repeat(100).trim().to_string()),
        ];

        let result = SlidingWindowCompaction.compact(messages, 50).await;

        // Both system messages should be preserved.
        let system_count = result
            .iter()
            .filter(|m| m.role == MessageRole::System)
            .count();
        // Original 2 system messages + 1 summary system message
        assert!(
            system_count >= 2,
            "should preserve original system messages, got {system_count}"
        );
    }

    #[tokio::test]
    async fn test_sliding_window_single_message() {
        let messages = vec![ChatMessage::user("Just one message")];

        let result = SlidingWindowCompaction.compact(messages, 1).await;

        // Should keep at least the last message even if over budget.
        assert!(!result.is_empty());
    }

    #[tokio::test]
    async fn test_maybe_compact_under_budget() {
        let messages = vec![ChatMessage::user("Hi"), ChatMessage::assistant("Hello!")];

        let result = maybe_compact(messages.clone(), 10000, &SlidingWindowCompaction).await;
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn test_maybe_compact_over_budget() {
        let mut messages = Vec::new();
        for i in 0..50 {
            messages.push(ChatMessage::user(format!(
                "Message {i}: {}",
                "x".repeat(200)
            )));
            messages.push(ChatMessage::assistant(format!(
                "Reply {i}: {}",
                "y".repeat(200)
            )));
        }

        let total_before = messages_token_count(&messages);
        assert!(total_before > 1000);

        let result = maybe_compact(messages, 500, &SlidingWindowCompaction).await;
        assert!(
            result.len() < 100,
            "should have fewer messages after compaction"
        );
    }

    #[tokio::test]
    async fn test_summary_message_content() {
        let evicted = vec![
            ChatMessage::user("What is Rust?"),
            ChatMessage::assistant("Rust is a systems programming language."),
            ChatMessage::user("Tell me more about ownership."),
            ChatMessage::assistant("Ownership is Rust's key memory safety feature."),
        ];

        let summary = SlidingWindowCompaction::summarize_evicted(&evicted);
        assert!(summary.contains("2 user message(s)"));
        assert!(summary.contains("2 assistant message(s)"));
        assert!(summary.contains("ownership"));
    }
}
