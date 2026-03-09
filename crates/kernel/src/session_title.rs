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

//! Session title generation — auto-generate a short title from conversation
//! content after the first turn completes.

use tracing::{info, warn};

use crate::{
    llm::{CompletionRequest, LlmDriver, Message, ToolChoice},
    memory::{TapEntry, TapEntryKind},
};

/// Generate a short session title (5-10 words) from conversation tape entries.
///
/// Returns `None` if there is no conversational content or the LLM call fails.
pub async fn generate_title(
    entries: &[TapEntry],
    driver: &dyn LlmDriver,
    model: &str,
) -> Option<String> {
    let conversation = build_conversation_text(entries);
    if conversation.is_empty() {
        info!("session title: no conversation content");
        return None;
    }

    // Truncate to ~2000 chars to keep the request small — title only needs
    // the gist of the conversation, not every detail.
    let truncated = if conversation.len() > 2000 {
        &conversation[..2000]
    } else {
        &conversation
    };

    let system_prompt = "You are a title generator. Given a conversation, produce a short title (5-10 words) that captures the main topic. Output ONLY the title text, no quotes, no punctuation at the end, no explanation.";

    let request = CompletionRequest {
        model:               model.to_string(),
        messages:            vec![
            Message::system(system_prompt),
            Message::user(format!(
                "Generate a title for this conversation:\n\n{truncated}"
            )),
        ],
        tools:               Vec::new(),
        temperature:         Some(0.3),
        max_tokens:          Some(50),
        thinking:            None,
        tool_choice:         ToolChoice::None,
        parallel_tool_calls: false,
    };

    match driver.complete(request).await {
        Ok(response) => {
            let title = response
                .content
                .as_deref()
                .unwrap_or("")
                .trim()
                .to_string();
            if title.is_empty() {
                warn!("session title: LLM returned empty title");
                None
            } else {
                info!(title = %title, "session title generated");
                Some(title)
            }
        }
        Err(e) => {
            warn!(%e, "session title: LLM call failed");
            None
        }
    }
}

fn build_conversation_text(entries: &[TapEntry]) -> String {
    let mut lines = Vec::new();
    for entry in entries {
        if entry.kind != TapEntryKind::Message {
            continue;
        }
        let role = entry
            .payload
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let content = entry
            .payload
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !content.is_empty() {
            lines.push(format!("[{role}]: {content}"));
        }
    }
    lines.join("\n")
}
