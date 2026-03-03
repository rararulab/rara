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

//! Trace and result types for agent turns.

/// Maximum byte length for result preview strings.
pub(crate) const RESULT_PREVIEW_MAX_BYTES: usize = 2048;

/// Truncate a string to at most `max_bytes` bytes on a valid char boundary.
pub(crate) fn truncate_preview(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let boundary = s.floor_char_boundary(max_bytes);
    format!("{}... (truncated)", &s[..boundary])
}

/// A tool call being incrementally assembled from streaming deltas.
pub(super) struct PendingToolCall {
    pub(super) id:            String,
    pub(super) name:          String,
    pub(super) arguments_buf: String,
}

/// Trace of a single tool call within an iteration.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolCallTrace {
    pub name:           String,
    pub id:             String,
    pub duration_ms:    u64,
    pub success:        bool,
    pub arguments:      serde_json::Value,
    pub result_preview: String,
    pub error:          Option<String>,
}

/// Trace of a single LLM iteration within a turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IterationTrace {
    pub index:          usize,
    pub first_token_ms: Option<u64>,
    pub stream_ms:      u64,
    /// First 200 chars of accumulated text.
    pub text_preview:   String,
    /// Full accumulated reasoning text for this iteration.
    pub reasoning_text: Option<String>,
    pub tool_calls:     Vec<ToolCallTrace>,
}

/// Complete trace of a single agent turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnTrace {
    pub duration_ms:      u64,
    pub model:            String,
    /// The user message that triggered this turn.
    pub input_text:       Option<String>,
    pub iterations:       Vec<IterationTrace>,
    pub final_text_len:   usize,
    pub total_tool_calls: usize,
    pub success:          bool,
    pub error:            Option<String>,
}

/// Result of a single agent turn.
#[derive(Debug)]
pub struct AgentTurnResult {
    /// The final text produced by the agent.
    pub text:       String,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Number of tool calls executed.
    pub tool_calls: usize,
    /// Model used for this turn.
    pub model:      String,
    /// Detailed trace of the turn for observability.
    pub trace:      TurnTrace,
}
