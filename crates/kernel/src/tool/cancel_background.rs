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
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use tracing::info;

use crate::{
    handle::KernelHandle,
    io::{BackgroundTaskStatus, StreamEvent},
    session::{SessionKey, Signal},
    tool::{ToolContext, ToolExecute},
};

/// Builtin tool that cancels a running background agent.
#[derive(ToolDef)]
#[tool(
    name = "cancel-background",
    description = "Cancel a running background task by task_id.",
    tier = "deferred"
)]
pub struct CancelBackgroundTool {
    handle:      KernelHandle,
    session_key: SessionKey,
}

impl CancelBackgroundTool {
    pub fn new(handle: KernelHandle, session_key: SessionKey) -> Self {
        Self {
            handle,
            session_key,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CancelBackgroundParams {
    /// The task_id returned by spawn_background
    task_id: String,
    /// Optional reason for cancellation
    reason:  Option<String>,
}

#[async_trait]
impl ToolExecute for CancelBackgroundTool {
    type Output = serde_json::Value;
    type Params = CancelBackgroundParams;

    async fn run(
        &self,
        p: CancelBackgroundParams,
        _context: &ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let task_id = SessionKey::try_from_raw(&p.task_id)
            .map_err(|_| anyhow::anyhow!("invalid task_id: {}", p.task_id))?;
        let reason = p.reason.as_deref().unwrap_or("cancelled by parent");

        if !self.handle.is_background_task(self.session_key, task_id) {
            return Ok(serde_json::json!({
                "error": "task not found or not a background task of this session",
                "task_id": p.task_id,
            }));
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
                "task_id": p.task_id,
            }));
        }

        // Signal succeeded — now safe to remove from active list so
        // ChildSessionDone won't trigger a proactive turn.
        self.handle
            .remove_background_task(self.session_key, task_id);

        // Emit BackgroundTaskDone so clients remove the status indicator.
        self.handle.stream_hub().emit_to_session(
            &self.session_key,
            StreamEvent::BackgroundTaskDone {
                task_id: task_id.to_string(),
                status:  BackgroundTaskStatus::Cancelled,
            },
        );

        Ok(serde_json::json!({
            "task_id": p.task_id,
            "status": "cancelled",
            "reason": reason,
        }))
    }
}
