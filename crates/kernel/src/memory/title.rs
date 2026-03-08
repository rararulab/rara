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

//! Auto-generate a short session title from the first few conversation turns.
//!
//! The title is produced by a lightweight LLM call (low temperature, small
//! `max_tokens`) and is intended as a best-effort label for display in session
//! lists.  Failures are logged but never block the event loop.

use crate::llm::{
    driver::LlmDriver,
    types::{CompletionRequest, Message, ToolChoice},
};
use crate::memory::{TapEntry, TapEntryKind};

/// Maximum number of message entries fed to the title-generation prompt.
const MAX_CONTEXT_ENTRIES: usize = 10;

/// Maximum character length per message when building the title prompt.
const MAX_CHARS_PER_MESSAGE: usize = 200;

const SYSTEM_PROMPT: &str = "\
You generate concise session titles (5-10 words). \
Respond with ONLY the title text, nothing else. \
No quotes, no punctuation at the end.";

/// Build a compact conversation summary from tape entries for the title prompt.
fn build_conversation_summary(entries: &[TapEntry]) -> String {
    let mut lines = Vec::new();

    for entry in entries.iter().filter(|e| e.kind == TapEntryKind::Message) {
        if lines.len() >= MAX_CONTEXT_ENTRIES {
            break;
        }

        let payload = &entry.payload;
        let role = payload
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let content = payload
            .get("content")
            .and_then(|v| match v {
                serde_json::Value::String(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("");

        if content.is_empty() {
            continue;
        }

        let truncated = if content.len() > MAX_CHARS_PER_MESSAGE {
            format!("{}…", &content[..MAX_CHARS_PER_MESSAGE])
        } else {
            content.to_string()
        };
        lines.push(format!("{role}: {truncated}"));
    }

    lines.join("\n")
}

/// Generate a short session title from the conversation tape.
///
/// Returns `None` if the tape has no message entries or if the LLM call fails.
/// This function is designed for best-effort async use — callers should
/// `tokio::spawn` it and log errors rather than propagating them.
pub async fn generate_session_title(
    entries: &[TapEntry],
    driver: &dyn LlmDriver,
    model: &str,
) -> Option<String> {
    let summary = build_conversation_summary(entries);
    if summary.is_empty() {
        return None;
    }

    let request = CompletionRequest {
        model:               model.to_string(),
        messages:            vec![
            Message::system(SYSTEM_PROMPT),
            Message::user(format!(
                "Generate a concise title for this conversation:\n\n{summary}"
            )),
        ],
        tools:               vec![],
        temperature:         Some(0.3),
        max_tokens:          Some(50),
        thinking:            None,
        tool_choice:         ToolChoice::None,
        parallel_tool_calls: false,
    };

    match driver.complete(request).await {
        Ok(response) => {
            let title = response.content?.trim().to_string();
            if title.is_empty() { None } else { Some(title) }
        }
        Err(e) => {
            tracing::warn!(%e, "session title generation failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;
    use serde_json::json;

    #[test]
    fn build_summary_skips_non_message_entries() {
        let entries = vec![
            TapEntry {
                id:        1,
                kind:      TapEntryKind::System,
                payload:   json!({"content": "system prompt"}),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        2,
                kind:      TapEntryKind::Message,
                payload:   json!({"role": "user", "content": "Hello!"}),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        3,
                kind:      TapEntryKind::ToolCall,
                payload:   json!({"calls": []}),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        4,
                kind:      TapEntryKind::Message,
                payload:   json!({"role": "assistant", "content": "Hi there!"}),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
        ];

        let summary = build_conversation_summary(&entries);
        assert_eq!(summary, "user: Hello!\nassistant: Hi there!");
    }

    #[test]
    fn build_summary_truncates_long_messages() {
        let long_content = "a".repeat(300);
        let entries = vec![TapEntry {
            id:        1,
            kind:      TapEntryKind::Message,
            payload:   json!({"role": "user", "content": long_content}),
            timestamp: Timestamp::now(),
            metadata:  None,
        }];

        let summary = build_conversation_summary(&entries);
        // 200 chars + "…" suffix
        assert!(summary.len() < 300);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn build_summary_limits_entries() {
        let entries: Vec<TapEntry> = (0..20)
            .map(|i| TapEntry {
                id:        i,
                kind:      TapEntryKind::Message,
                payload:   json!({"role": "user", "content": format!("msg {i}")}),
                timestamp: Timestamp::now(),
                metadata:  None,
            })
            .collect();

        let summary = build_conversation_summary(&entries);
        let line_count = summary.lines().count();
        assert_eq!(line_count, MAX_CONTEXT_ENTRIES);
    }

    #[test]
    fn build_summary_empty_tape() {
        let summary = build_conversation_summary(&[]);
        assert!(summary.is_empty());
    }
}
