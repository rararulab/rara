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

//! LLM history conversion utilities.

use crate::llm;

/// Convert persisted chat history into [`llm::Message`] format.
///
/// This is the `LlmDriver`-native equivalent of the legacy
/// `runner::build_history_messages` which returns async-openai types.
pub(crate) fn build_llm_history(
    history: &[crate::channel::types::ChatMessage],
) -> Vec<llm::Message> {
    history
        .iter()
        .filter_map(|msg| {
            use crate::channel::types::MessageRole;
            match msg.role {
                MessageRole::System => Some(llm::Message::system(msg.content.as_text())),
                MessageRole::User => Some(llm::Message::user(msg.content.as_text())),
                MessageRole::Assistant => {
                    if msg.tool_calls.is_empty() {
                        Some(llm::Message::assistant(msg.content.as_text()))
                    } else {
                        let tool_calls: Vec<llm::ToolCallRequest> = msg
                            .tool_calls
                            .iter()
                            .map(|tc| llm::ToolCallRequest {
                                id:        tc.id.to_string(),
                                name:      tc.name.to_string(),
                                arguments: tc.arguments.to_string(),
                            })
                            .collect();
                        Some(llm::Message::assistant_with_tool_calls(
                            msg.content.as_text(),
                            tool_calls,
                        ))
                    }
                }
                MessageRole::Tool | MessageRole::ToolResult => {
                    let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("");
                    Some(llm::Message::tool_result(
                        tool_call_id,
                        msg.content.as_text(),
                    ))
                }
            }
        })
        .collect()
}
