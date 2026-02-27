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

//! Common output type returned by built-in agent executions.

use crate::runner::AgentRunResponse;

/// Output from a built-in agent execution.
pub struct AgentOutput {
    /// The assistant's response text.
    pub response_text:   String,
    /// Number of LLM iterations used.
    pub iterations:      usize,
    /// Number of tool calls made.
    pub tool_calls_made: usize,
    /// `true` when the agent loop was stopped early because it hit the
    /// max-iterations ceiling. The response contains all work completed
    /// so far, but the task may be incomplete.
    pub truncated:       bool,
}

impl AgentOutput {
    /// Build from an `AgentRunResponse`, extracting the response text.
    pub fn from_run_response(response: &AgentRunResponse) -> Self {
        Self {
            response_text:   response.response_text(),
            iterations:      response.iterations,
            tool_calls_made: response.tool_calls_made,
            truncated:       response.truncated,
        }
    }
}
