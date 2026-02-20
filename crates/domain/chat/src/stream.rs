// Copyright 2025 Crrow
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

//! SSE event types for the streaming chat endpoint.
//!
//! [`ChatStreamEvent`] is a domain-level representation of streaming events
//! emitted during a chat turn. It is a 1:1 mapping from
//! [`rara_agents::runner::RunnerEvent`] with tool-call argument details
//! stripped (to keep payloads small).

use serde::{Deserialize, Serialize};

/// SSE event types for the streaming chat endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatStreamEvent {
    /// Incremental text content from the LLM.
    TextDelta { text: String },
    /// Incremental reasoning content from the LLM.
    ReasoningDelta { text: String },
    /// The LLM has started processing.
    Thinking,
    /// The LLM has finished processing.
    ThinkingDone,
    /// A new iteration of the agent loop has begun.
    Iteration { index: usize },
    /// A tool call has started.
    ToolCallStart { id: String, name: String },
    /// A tool call has completed.
    ToolCallEnd {
        id:      String,
        name:    String,
        success: bool,
        error:   Option<String>,
    },
    /// The agent loop completed successfully.
    Done { text: String },
    /// The agent loop failed with an error.
    Error { message: String },
}

impl ChatStreamEvent {
    /// Return the SSE event type name for the `event:` field.
    pub fn event_type_name(&self) -> &'static str {
        match self {
            Self::TextDelta { .. } => "text_delta",
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::Thinking => "thinking",
            Self::ThinkingDone => "thinking_done",
            Self::Iteration { .. } => "iteration",
            Self::ToolCallStart { .. } => "tool_call_start",
            Self::ToolCallEnd { .. } => "tool_call_end",
            Self::Done { .. } => "done",
            Self::Error { .. } => "error",
        }
    }
}

impl From<rara_agents::runner::RunnerEvent> for ChatStreamEvent {
    fn from(event: rara_agents::runner::RunnerEvent) -> Self {
        use rara_agents::runner::RunnerEvent;
        match event {
            RunnerEvent::TextDelta(text) => Self::TextDelta { text },
            RunnerEvent::ReasoningDelta(text) => Self::ReasoningDelta { text },
            RunnerEvent::Thinking => Self::Thinking,
            RunnerEvent::ThinkingDone => Self::ThinkingDone,
            RunnerEvent::Iteration(i) => Self::Iteration { index: i },
            RunnerEvent::ToolCallStart { id, name, .. } => Self::ToolCallStart { id, name },
            RunnerEvent::ToolCallEnd {
                id,
                name,
                success,
                error,
                ..
            } => Self::ToolCallEnd {
                id,
                name,
                success,
                error,
            },
            RunnerEvent::Done { text, .. } => Self::Done { text },
            RunnerEvent::Error(msg) => Self::Error { message: msg },
        }
    }
}
