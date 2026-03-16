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

use async_trait::async_trait;
use serde_json::Value;
use tracing::info;

use crate::{
    handle::KernelHandle,
    io::{BackgroundTaskStatus, StreamEvent},
    session::{SessionKey, Signal},
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Builtin tool that cancels a running background agent.
pub struct CancelBackgroundTool {
    handle:      KernelHandle,
    session_key: SessionKey,
}

impl CancelBackgroundTool {
    pub const NAME: &str = crate::tool_names::CANCEL_BACKGROUND;

    pub fn new(handle: KernelHandle, session_key: SessionKey) -> Self {
        Self { handle, session_key }
    }
}

#[async_trait]
impl AgentTool for CancelBackgroundTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Cancel a running background task by task_id."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": { "type": "string", "description": "The task_id returned by spawn_background" },
                "reason": { "type": "string", "description": "Optional reason for cancellation" }
            }
        })
    }

    async fn execute(&self, params: Value, _context: &ToolContext) -> anyhow::Result<ToolOutput> {
        let task_id_str = params["task_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required field: task_id"))?;
        let task_id = SessionKey::try_from_raw(task_id_str)
            .map_err(|_| anyhow::anyhow!("invalid task_id: {task_id_str}"))?;
        let reason = params["reason"].as_str().unwrap_or("cancelled by parent");

        if !self.handle.is_background_task(&self.session_key, &task_id) {
            return Ok(serde_json::json!({
                "error": "task not found or not a background task of this session",
                "task_id": task_id_str,
            })
            .into());
        }

        info!(
            parent = %self.session_key,
            task_id = %task_id,
            reason = %reason,
            "cancelling background agent"
        );

        // Send Terminate signal first — if this fails the task is still tracked
        // and the caller can retry.
        if let Err(e) = self.handle.send_signal(task_id, Signal::Terminate) {
            return Ok(serde_json::json!({
                "error": format!("failed to send terminate signal: {e}"),
                "task_id": task_id_str,
            })
            .into());
        }

        // Signal succeeded — now safe to remove from active list so
        // ChildSessionDone won't trigger a proactive turn.
        self.handle.remove_background_task(&self.session_key, &task_id);

        // Emit BackgroundTaskDone so clients remove the status indicator.
        self.handle.stream_hub().emit_to_session(
            &self.session_key,
            StreamEvent::BackgroundTaskDone {
                task_id: task_id.to_string(),
                status:  BackgroundTaskStatus::Cancelled,
            },
        );

        Ok(serde_json::json!({
            "task_id": task_id_str,
            "status": "cancelled",
            "reason": reason,
        })
        .into())
    }
}
