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

//! Automatic session title generation.
//!
//! After the first successful turn in a session that has no title, we ask
//! a lightweight LLM call to produce a short (5–10 word) title summarising
//! the conversation topic.  The result is persisted via [`SessionIndex`].

use tracing::warn;

use crate::{
    llm::{self, LlmDriverRef},
    session::{SessionIndexRef, SessionKey},
};

const TITLE_SYSTEM_PROMPT: &str = "\
You are a concise title generator. Given a conversation between a user and an assistant, \
produce a short title (5-10 words) that captures the main topic. \
Reply with ONLY the title text, no quotes, no punctuation at the end, no explanation.";

/// Generate a session title from the first user message and assistant reply,
/// then persist it to the session index.
///
/// This is designed to be called from a `tokio::spawn` — all errors are
/// logged but never propagated.
pub(crate) async fn generate_and_set_title(
    session_key: SessionKey,
    user_text: String,
    assistant_text: String,
    driver: LlmDriverRef,
    model: String,
    session_index: SessionIndexRef,
) {
    let title = match generate_title(&user_text, &assistant_text, &driver, &model).await {
        Ok(t) => t,
        Err(e) => {
            warn!(
                %session_key,
                error = %e,
                "failed to generate session title"
            );
            return;
        }
    };

    // Read-modify-write the session entry.
    let session = match session_index.get_session(&session_key).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            warn!(%session_key, "session not found when setting title");
            return;
        }
        Err(e) => {
            warn!(%session_key, error = %e, "failed to read session for title update");
            return;
        }
    };

    // Double-check: another turn might have already set the title.
    if session.title.is_some() {
        return;
    }

    let mut updated = session;
    updated.title = Some(title.clone());
    if let Err(e) = session_index.update_session(&updated).await {
        warn!(%session_key, error = %e, "failed to persist session title");
    } else {
        tracing::info!(%session_key, %title, "auto-generated session title");
    }
}

async fn generate_title(
    user_text: &str,
    assistant_text: &str,
    driver: &LlmDriverRef,
    model: &str,
) -> anyhow::Result<String> {
    let conversation = format!(
        "User: {}\n\nAssistant: {}",
        truncate(user_text, 500),
        truncate(assistant_text, 500),
    );

    let request = llm::CompletionRequest {
        model:               model.to_string(),
        messages:            vec![
            llm::Message::system(TITLE_SYSTEM_PROMPT),
            llm::Message::user(conversation),
        ],
        tools:               vec![],
        temperature:         Some(0.3),
        max_tokens:          Some(50),
        thinking:            None,
        tool_choice:         llm::ToolChoice::None,
        parallel_tool_calls: false,
    };

    let response = driver.complete(request).await?;
    let title = response
        .content
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .to_string();

    if title.is_empty() {
        anyhow::bail!("LLM returned empty title");
    }

    Ok(title)
}

/// Truncate a string to at most `max_chars` characters.
fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}
