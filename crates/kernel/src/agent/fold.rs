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

//! Context folding — pressure-driven automatic summarization.
//!
//! [`ContextFolder`] uses an independent LLM call (outside the main agent
//! loop) to compress conversation history into a compact summary that is
//! persisted as a tape anchor via [`HandoffState`].

use std::sync::Arc;

use serde::Deserialize;

use crate::{
    error::{KernelError, Result},
    llm::{
        driver::LlmDriver,
        types::{CompletionRequest, Message, ToolChoice},
    },
    memory::HandoffState,
};

// ---------------------------------------------------------------------------
// FoldSummary
// ---------------------------------------------------------------------------

/// Result of a context fold operation.
#[derive(Debug, Clone)]
pub struct FoldSummary {
    /// Key information summary of the current context.
    pub summary:    String,
    /// Actionable next steps.
    pub next_steps: String,
}

/// Raw JSON shape expected from the fold LLM call.
#[derive(Deserialize)]
struct FoldResponse {
    summary:    String,
    next_steps: String,
}

// ---------------------------------------------------------------------------
// ContextFolder
// ---------------------------------------------------------------------------

/// Orchestrates context compression via an independent LLM call.
///
/// Placed in the kernel layer (alongside the agent loop) rather than in the
/// memory module, because folding involves LLM orchestration logic.
pub struct ContextFolder {
    /// LLM driver used for summarization.
    driver: Arc<dyn LlmDriver>,
    /// Model identifier for the fold call.
    model:  String,
}

impl ContextFolder {
    /// Create a new folder backed by the given driver and model.
    pub fn new(driver: Arc<dyn LlmDriver>, model: String) -> Self { Self { driver, model } }

    /// Fold a sequence of messages into a compact summary.
    ///
    /// Uses an independent short-context LLM call — does NOT go through the
    /// main agent loop.  `source_token_estimate` is used to compute a dynamic
    /// summary length (~10 % of source, clamped to [256, 2048]).
    pub async fn fold_with_prior(
        &self,
        prior_summary: Option<&str>,
        messages: &[Message],
        source_token_estimate: usize,
    ) -> Result<FoldSummary> {
        let max_tokens = (source_token_estimate / 10).clamp(256, 2048) as u32;

        let fold_prompt = Message::system(FOLD_SYSTEM_PROMPT);

        let mut content = String::new();
        if let Some(prior) = prior_summary {
            content.push_str(&format!("## Prior conversation history\n{prior}\n\n"));
        }
        content.push_str("## New conversation to summarize\n");
        content.push_str(&format_messages_for_fold(messages));

        let user_msg = Message::user(content);

        let request = CompletionRequest {
            model:               self.model.clone(),
            messages:            vec![fold_prompt, user_msg],
            tools:               vec![],
            temperature:         Some(0.0),
            max_tokens:          Some(max_tokens),
            thinking:            None,
            tool_choice:         ToolChoice::None,
            parallel_tool_calls: false,
            frequency_penalty:   None,
        };

        let response = self.driver.complete(request).await?;
        let text = response.content.unwrap_or_default();
        parse_fold_response(&text)
    }

    /// Compress plain text to a target character count.
    ///
    /// Intended for P1 fold_branch: the child agent's result may be long and
    /// needs compression before being written back as a ToolResult.
    pub async fn fold_text(&self, text: &str, target_chars: usize) -> Result<String> {
        let prompt = Message::system(
            "Compress the following text to be concise while preserving all key facts, decisions, \
             and actionable information. Use the same language as the input. Output ONLY the \
             compressed text, no wrapper.",
        );
        let user_msg = Message::user(format!("Compress to ~{target_chars} characters:\n\n{text}"));

        let max_tokens = (target_chars / 3).clamp(128, 2048) as u32;

        let request = CompletionRequest {
            model:               self.model.clone(),
            messages:            vec![prompt, user_msg],
            tools:               vec![],
            temperature:         Some(0.0),
            max_tokens:          Some(max_tokens),
            thinking:            None,
            tool_choice:         ToolChoice::None,
            parallel_tool_calls: false,
            frequency_penalty:   None,
        };

        let response = self.driver.complete(request).await?;
        Ok(response.content.unwrap_or_default())
    }

    /// Convert a [`FoldSummary`] into a [`HandoffState`] for tape anchoring.
    pub fn to_handoff_state(summary: &FoldSummary, pressure: f64) -> HandoffState {
        HandoffState {
            phase:      Some("auto-fold".into()),
            summary:    Some(summary.summary.clone()),
            next_steps: Some(summary.next_steps.clone()),
            source_ids: vec![],
            owner:      Some("system".into()),
            extra:      Some(serde_json::json!({
                "trigger": "context_pressure",
                "pressure_at_fold": pressure,
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Render messages into a flat text representation suitable for the fold LLM.
pub(crate) fn format_messages_for_fold(messages: &[Message]) -> String {
    use crate::llm::types::Role;

    let mut buf = String::new();
    for msg in messages {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        buf.push_str(&format!("[{role}] {}\n", msg.content.as_text()));

        for tc in &msg.tool_calls {
            buf.push_str(&format!("  -> tool_call: {}({})\n", tc.name, tc.arguments));
        }
    }
    buf
}

/// Parse the LLM response into a [`FoldSummary`].
///
/// Handles the case where the LLM wraps JSON in markdown code fences.
pub(crate) fn parse_fold_response(text: &str) -> Result<FoldSummary> {
    // Strip optional markdown code fence wrappers.
    let trimmed = text.trim();
    let json_str = if trimmed.starts_with("```") {
        // Remove opening fence (with optional language tag) and closing fence.
        let after_open = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed);
        after_open
            .trim()
            .strip_suffix("```")
            .unwrap_or(after_open)
            .trim()
    } else {
        trimmed
    };

    let parsed: FoldResponse =
        serde_json::from_str(json_str).map_err(|e| KernelError::AgentExecution {
            message: format!("failed to parse fold response as JSON: {e}\nraw: {text}"),
        })?;

    Ok(FoldSummary {
        summary:    parsed.summary,
        next_steps: parsed.next_steps,
    })
}

// ---------------------------------------------------------------------------
// Fold system prompt
// ---------------------------------------------------------------------------

const FOLD_SYSTEM_PROMPT: &str = r#"You are a context compression specialist.
Given a conversation history, produce two parts:

1. **summary**: Key information summary. MUST preserve:
   - User identity and preferences
   - All factual information (file paths, code state, config values)
   - Decisions made and their reasoning
   - Errors encountered and solutions attempted
   DELETE: greetings, redundant tool outputs, intermediate reasoning steps

2. **next_steps**: Work currently in progress or about to begin.

Output JSON: {"summary": "...", "next_steps": "..."}
IMPORTANT: Generate the summary in the SAME LANGUAGE as the conversation being summarized."#;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Scan the tape for the most recent `auto-fold` anchor and return its entry
/// ID.  This allows the cooldown counter to survive across turns — without it,
/// `last_fold_entry_id` would reset to `None` on every `run_agent_loop` call.
pub(crate) async fn find_last_auto_fold_entry_id(
    tape: &crate::memory::TapeService,
    tape_name: &str,
) -> Option<u64> {
    let entries = tape.entries(tape_name).await.ok()?;
    entries
        .iter()
        .rev()
        .find(|e| {
            e.kind == crate::memory::TapEntryKind::Anchor
                && e.payload
                    .get("state")
                    .and_then(|s| s.get("phase"))
                    .and_then(|p| p.as_str())
                    == Some("auto-fold")
        })
        .map(|e| e.id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::Message;

    // -- parse_fold_response -------------------------------------------------

    #[test]
    fn parse_fold_response_valid_json() {
        let input = r#"{"summary": "test summary", "next_steps": "test steps"}"#;
        let result = parse_fold_response(input).unwrap();
        assert_eq!(result.summary, "test summary");
        assert_eq!(result.next_steps, "test steps");
    }

    #[test]
    fn parse_fold_response_json_in_code_fence() {
        let input =
            "```json\n{\"summary\": \"fenced summary\", \"next_steps\": \"fenced steps\"}\n```";
        let result = parse_fold_response(input).unwrap();
        assert_eq!(result.summary, "fenced summary");
        assert_eq!(result.next_steps, "fenced steps");
    }

    #[test]
    fn parse_fold_response_code_fence_no_lang_tag() {
        let input = "```\n{\"summary\": \"no lang\", \"next_steps\": \"steps\"}\n```";
        let result = parse_fold_response(input).unwrap();
        assert_eq!(result.summary, "no lang");
        assert_eq!(result.next_steps, "steps");
    }

    #[test]
    fn parse_fold_response_invalid_json() {
        let input = "this is not json";
        let result = parse_fold_response(input);
        assert!(result.is_err());
    }

    // -- to_handoff_state ----------------------------------------------------

    #[test]
    fn to_handoff_state_fields() {
        let summary = FoldSummary {
            summary:    "the summary".into(),
            next_steps: "the steps".into(),
        };
        let pressure = 0.82;
        let hs = ContextFolder::to_handoff_state(&summary, pressure);

        assert_eq!(hs.phase.as_deref(), Some("auto-fold"));
        assert_eq!(hs.summary.as_deref(), Some("the summary"));
        assert_eq!(hs.next_steps.as_deref(), Some("the steps"));
        assert_eq!(hs.owner.as_deref(), Some("system"));

        let extra = hs.extra.unwrap();
        assert_eq!(extra["trigger"], "context_pressure");
        assert_eq!(extra["pressure_at_fold"], serde_json::json!(0.82));
    }

    // -- format_messages_for_fold --------------------------------------------

    #[test]
    fn format_messages_for_fold_basic() {
        let msgs = vec![
            Message::user("Hello there"),
            Message::assistant("Hi! How can I help?"),
        ];
        let output = format_messages_for_fold(&msgs);

        assert!(output.contains("[user] Hello there"));
        assert!(output.contains("[assistant] Hi! How can I help?"));
    }

    #[test]
    fn format_messages_for_fold_system_role() {
        let msgs = vec![Message::system("You are helpful.")];
        let output = format_messages_for_fold(&msgs);
        assert!(output.contains("[system] You are helpful."));
    }
}
