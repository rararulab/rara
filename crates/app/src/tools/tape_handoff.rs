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

//! Tool for agent-driven context truncation via tape handoff anchors.

use async_trait::async_trait;
use rara_kernel::{
    memory::{HandoffState, TapeService},
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TapeHandoffParams {
    /// Anchor name (default: "handoff").
    name:       Option<String>,
    /// Summary of conversation so far.
    summary:    String,
    /// Actionable items for the next phase.
    next_steps: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TapeHandoffResult {
    pub output: String,
}

/// Creates a handoff anchor in the session tape, enabling context truncation.
#[derive(ToolDef)]
#[tool(
    name = "tape-handoff",
    description = "Create a handoff anchor to checkpoint progress and truncate context history. \
                   This is a core workflow tool \u{2014} use it proactively to manage context, \
                   not just when failures occur. You MUST provide a summary to preserve context \
                   across the handoff boundary.",
    bypass_interceptor,
    tier = "deferred"
)]
pub struct TapeHandoffTool {
    tape_service: TapeService,
}
impl TapeHandoffTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl ToolExecute for TapeHandoffTool {
    type Output = TapeHandoffResult;
    type Params = TapeHandoffParams;

    async fn run(
        &self,
        params: TapeHandoffParams,
        context: &ToolContext,
    ) -> anyhow::Result<TapeHandoffResult> {
        let tape_name = context.session_key.to_string();
        let anchor_name = params.name.as_deref().unwrap_or("handoff");
        let state = HandoffState {
            phase:      None,
            summary:    Some(params.summary),
            next_steps: params.next_steps,
            source_ids: vec![],
            owner:      Some("agent".to_owned()),
            extra:      None,
        };
        self.tape_service
            .handoff(&tape_name, anchor_name, state)
            .await
            .map_err(|e| anyhow::anyhow!("failed to create handoff: {e}"))?;
        Ok(TapeHandoffResult {
            output: format!("handoff created: {anchor_name}"),
        })
    }
}
