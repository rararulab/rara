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

//! Mita-exclusive tool for distilling accumulated user notes into a compact
//! anchor summary.
//!
//! When a user tape has accumulated many notes, Mita can use this tool to
//! condense them into a single distillation anchor.  After distillation, only
//! the summary and notes written after the anchor will appear in the user's
//! LLM context window.

use async_trait::async_trait;
use rara_kernel::{
    memory::{HandoffState, TapeService},
    tool::{AgentTool, ToolContext, ToolOutput},
};
use serde_json::json;

use super::notify::push_notification;

/// Mita-exclusive tool: distill accumulated user notes into a compact anchor.
pub struct DistillUserNotesTool {
    tape_service: TapeService,
}

impl DistillUserNotesTool {
    pub const NAME: &str = "distill-user-notes";

    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl AgentTool for DistillUserNotesTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Distill accumulated user notes into a compact summary anchor. Use this when a user's tape \
         has accumulated many notes that should be condensed. The summary should capture all \
         important knowledge from previous notes plus the existing distilled summary. After \
         distillation, only the summary and newer notes will appear in the user's context."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "The user identifier whose notes to distill"
                },
                "summary": {
                    "type": "string",
                    "description": "The distilled summary of all accumulated knowledge about this user"
                }
            },
            "required": ["user_id", "summary"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let user_id = params
            .get("user_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: user_id"))?;
        let summary = params
            .get("summary")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: summary"))?;

        if user_id.trim().is_empty() {
            anyhow::bail!("user_id must not be empty");
        }
        if summary.trim().is_empty() {
            anyhow::bail!("summary must not be empty");
        }

        let user_tape = rara_kernel::memory::user_tape_name(user_id);

        let handoff_state = HandoffState {
            summary: Some(summary.to_string()),
            owner: Some("mita".into()),
            ..Default::default()
        };

        self.tape_service
            .handoff(&user_tape, "distill", handoff_state)
            .await
            .map_err(|e| anyhow::anyhow!("failed to write distillation anchor: {e}"))?;

        push_notification(context, format!("🗜️ User notes distilled for {user_id}"));

        Ok(json!({
            "status": "ok",
            "user_id": user_id,
            "message": format!("User notes distilled for '{user_id}'. New anchor created.")
        })
        .into())
    }
}
