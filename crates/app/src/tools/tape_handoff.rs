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
    tool::AgentTool,
};
use serde_json::json;

/// Creates a handoff anchor in the session tape, enabling context truncation.
pub struct TapeHandoffTool {
    tape_service: TapeService,
}

impl TapeHandoffTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl AgentTool for TapeHandoffTool {
    fn name(&self) -> &str { "tape_handoff" }

    fn description(&self) -> &str {
        "Create a handoff anchor in the session tape to truncate context. \
         Use this when context is getting too long and may cause model call failures. \
         Provide a summary of the conversation so far and any next steps."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Anchor name (default: \"handoff\")"
                },
                "summary": {
                    "type": "string",
                    "description": "Summary of conversation so far"
                },
                "next_steps": {
                    "type": "string",
                    "description": "Actionable items for the next phase"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let tape_name = context
            .session_key
            .as_ref()
            .map(|k| k.to_string())
            .ok_or_else(|| anyhow::anyhow!("no session context"))?;

        let anchor_name = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("handoff");

        let summary = params
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        let next_steps = params
            .get("next_steps")
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        let state = HandoffState {
            phase: None,
            summary,
            next_steps,
            source_ids: vec![],
            owner: Some("agent".to_owned()),
            extra: None,
        };

        self.tape_service
            .handoff(&tape_name, anchor_name, state)
            .await
            .map_err(|e| anyhow::anyhow!("failed to create handoff: {e}"))?;

        Ok(json!({ "output": format!("handoff created: {anchor_name}") }))
    }
}
