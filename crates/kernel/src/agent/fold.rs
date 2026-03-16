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

//! Context folding — automatic hierarchical summarization of agent context.
//!
//! When context pressure exceeds a configurable threshold, the
//! [`ContextFolder`] uses an independent LLM call to produce a compact
//! [`FoldSummary`] that replaces the accumulated messages via a tape
//! handoff anchor.

use crate::{
    llm::{self, LlmDriverRef},
    memory::HandoffState,
};

/// Result of a context fold operation.
pub struct FoldSummary {
    /// Condensed summary of folded messages.
    pub summary:    String,
    /// Actionable next steps extracted from the folded context.
    pub next_steps: String,
}

/// Performs context folding via an independent LLM summarization call.
pub struct ContextFolder {
    driver: LlmDriverRef,
    model:  String,
}

impl ContextFolder {
    /// Create a new folder using the given LLM driver and model.
    pub fn new(driver: LlmDriverRef, model: String) -> Self { Self { driver, model } }

    /// Summarize messages into a compact fold summary.
    ///
    /// If `prior_summary` is provided, it is included as additional context
    /// so the fold can build on previous summarization layers
    /// (hierarchical folding).
    ///
    /// `source_token_estimate` is the approximate token count of the source
    /// messages, used to size the summarization output dynamically:
    /// `max_tokens = (source_token_estimate / 10).clamp(256, 2048)`.
    pub async fn fold_with_prior(
        &self,
        prior_summary: Option<&str>,
        messages: &[llm::Message],
        source_token_estimate: usize,
    ) -> anyhow::Result<FoldSummary> {
        let max_tokens = (source_token_estimate / 10).clamp(256, 2048) as u32;

        // Build the summarization prompt.
        let mut system_parts = vec![
            "You are a context summarizer. Produce a concise summary of the conversation and a \
             bullet list of next steps. Output EXACTLY two sections:\n\n## Summary\n<summary \
             text>\n\n## Next Steps\n<bullet list>"
                .to_owned(),
        ];
        if let Some(prior) = prior_summary {
            system_parts.push(format!(
                "\n\nPrior context summary (build upon this):\n{prior}"
            ));
        }
        let system_prompt = system_parts.join("");

        // Collect the conversation content into a single user message.
        let conversation_text: String = messages
            .iter()
            .map(|m| {
                let role_label = match m.role {
                    llm::Role::System => "system",
                    llm::Role::User => "user",
                    llm::Role::Assistant => "assistant",
                    llm::Role::Tool => "tool",
                };
                format!("[{}] {}", role_label, m.content.as_text())
            })
            .collect::<Vec<_>>()
            .join("\n");

        let request = llm::CompletionRequest {
            model:               self.model.clone(),
            messages:            vec![
                llm::Message::system(system_prompt),
                llm::Message::user(format!(
                    "Summarize the following conversation:\n\n{conversation_text}"
                )),
            ],
            tools:               vec![],
            temperature:         Some(0.0),
            max_tokens:          Some(max_tokens),
            thinking:            None,
            tool_choice:         llm::ToolChoice::None,
            parallel_tool_calls: false,
            frequency_penalty:   None,
        };

        let response = self
            .driver
            .complete(request)
            .await
            .map_err(|e| anyhow::anyhow!("fold LLM call failed: {e}"))?;

        let text = response
            .content
            .ok_or_else(|| anyhow::anyhow!("fold LLM returned empty content"))?;

        // Parse the two sections from the response.
        let (summary, next_steps) = parse_fold_response(&text);

        Ok(FoldSummary {
            summary,
            next_steps,
        })
    }

    /// Convert a [`FoldSummary`] into a [`HandoffState`] suitable for tape
    /// anchoring, with `phase` set to `"auto-fold"`.
    pub fn to_handoff_state(summary: &FoldSummary, _pressure: f64) -> HandoffState {
        HandoffState {
            phase:      Some("auto-fold".to_owned()),
            summary:    Some(summary.summary.clone()),
            next_steps: Some(summary.next_steps.clone()),
            source_ids: vec![],
            owner:      Some("system".to_owned()),
            extra:      None,
        }
    }
}

/// Parse the LLM fold response into (summary, next_steps).
///
/// Expects `## Summary` and `## Next Steps` headers. Falls back to
/// splitting on the first blank-line boundary if headers are absent.
fn parse_fold_response(text: &str) -> (String, String) {
    // Try header-based split first.
    if let Some(summary_start) = text.find("## Summary") {
        let after_summary = &text[summary_start + "## Summary".len()..];
        let summary_end = after_summary
            .find("## Next Steps")
            .unwrap_or(after_summary.len());
        let summary = after_summary[..summary_end].trim().to_owned();

        let next_steps = if let Some(ns_start) = after_summary.find("## Next Steps") {
            after_summary[ns_start + "## Next Steps".len()..]
                .trim()
                .to_owned()
        } else {
            String::new()
        };

        return (summary, next_steps);
    }

    // Fallback: entire text as summary.
    (text.trim().to_owned(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fold_response_with_headers() {
        let text = "## Summary\nDid A and B.\n\n## Next Steps\n- Do C\n- Do D";
        let (summary, next_steps) = parse_fold_response(text);
        assert_eq!(summary, "Did A and B.");
        assert_eq!(next_steps, "- Do C\n- Do D");
    }

    #[test]
    fn parse_fold_response_fallback() {
        let text = "Just a plain summary without headers.";
        let (summary, next_steps) = parse_fold_response(text);
        assert_eq!(summary, "Just a plain summary without headers.");
        assert!(next_steps.is_empty());
    }
}
