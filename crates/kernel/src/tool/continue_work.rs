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

//! Self-continuation signal tool.
//!
//! When the agent calls `continue-work`, it signals that the current task
//! is not yet complete and the agent loop should inject a continuation
//! wake message instead of terminating the turn.
//!
//! This is the structured (tool-call) channel of the dual-channel
//! continuation signal. The text-token fallback (`CONTINUE_WORK` at
//! response tail) is handled separately in the agent loop.
//!
//! Inspired by OpenClaw's CONTINUE_WORK signal.

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tool::{ToolContext, ToolExecute, ToolHint};

/// Tool that signals the agent wants another turn to continue working.
///
/// The tool itself does nothing — its effect is communicated via
/// [`ToolHint::ContinueWork`] which the agent loop inspects after
/// tool execution.
#[derive(ToolDef)]
#[tool(
    name = "continue-work",
    description = "Signal that you have more work to do on the current task. Call this instead of \
                   stopping to ask the user 'should I continue?' — the system will automatically \
                   give you another turn. Only use when you have concrete next steps, not to seem \
                   busy.",
    tier = "core"
)]
pub struct ContinueWorkTool;

/// Parameters for the `continue-work` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ContinueWorkParams {
    /// Brief description of what you will do next (for logging and context).
    reason: String,
}

/// Result returned to the LLM.
#[derive(Debug, Serialize)]
pub struct ContinueWorkResult {
    status: &'static str,
}

#[async_trait]
impl ToolExecute for ContinueWorkTool {
    type Output = ContinueWorkResult;
    type Params = ContinueWorkParams;

    fn hints(&self) -> Vec<ToolHint> {
        vec![ToolHint::ContinueWork {
            reason: String::new(),
        }]
    }

    async fn run(
        &self,
        params: ContinueWorkParams,
        _context: &ToolContext,
    ) -> anyhow::Result<ContinueWorkResult> {
        tracing::info!(reason = %params.reason, "agent elected to continue working");
        Ok(ContinueWorkResult {
            status: "continuing",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_returns_continue_work() {
        let tool = ContinueWorkTool;
        let hints = tool.hints();
        assert_eq!(hints.len(), 1);
        assert!(matches!(&hints[0], ToolHint::ContinueWork { .. }));
    }
}
