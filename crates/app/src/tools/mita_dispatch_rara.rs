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

//! Mita-exclusive tool: dispatch an instruction to Rara for a specific session.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    handle::KernelHandle,
    identity::UserId,
    io::InboundMessage,
    memory::TapeService,
    session::SessionKey,
    tool::{AgentTool, ToolContext},
};
use serde_json::{Value, json};
use tokio::sync::RwLock;

/// Mita tool that dispatches an instruction to Rara for a given session.
///
/// The instruction is delivered as a synthetic internal message to the
/// target session, prefixed with a system marker so Rara knows it comes
/// from Mita's proactive analysis (not the user).
///
/// The `KernelHandle` is set after kernel startup via [`set_kernel_handle`].
pub struct DispatchRaraTool {
    kernel_handle: Arc<RwLock<Option<KernelHandle>>>,
    tape_service:  TapeService,
}

impl DispatchRaraTool {
    pub fn new(tape_service: TapeService) -> Self {
        Self {
            kernel_handle: Arc::new(RwLock::new(None)),
            tape_service,
        }
    }

    /// Provide the kernel handle after kernel startup.
    ///
    /// Tools are constructed before the kernel starts, so the handle must
    /// be injected after `Kernel::start()` returns.
    pub fn handle_ref(&self) -> Arc<RwLock<Option<KernelHandle>>> {
        Arc::clone(&self.kernel_handle)
    }
}

#[async_trait]
impl AgentTool for DispatchRaraTool {
    fn name(&self) -> &str { "dispatch-rara" }

    fn description(&self) -> &str {
        "Dispatch a proactive instruction to Rara for a specific session. Rara will receive the \
         instruction as a system-level directive and generate an appropriate response to the user. \
         Use this when you determine a user needs proactive attention."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "The target session key where Rara should act"
                },
                "instruction": {
                    "type": "string",
                    "description": "Specific instruction for Rara (e.g., 'Follow up on the deadline the user mentioned for their project report')"
                }
            },
            "required": ["session_id", "instruction"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> anyhow::Result<Value> {
        let session_id_str = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: session_id"))?;

        let instruction = params
            .get("instruction")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: instruction"))?;

        let handle = self.kernel_handle.read().await;
        let handle = handle
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("kernel handle not yet available"))?;

        let session_key = SessionKey::try_from_raw(session_id_str)
            .map_err(|_| anyhow::anyhow!("invalid session key: {session_id_str}"))?;

        // Record the dispatch in Mita's tape for future reference (avoid repeats).
        let mita_tape = "mita";
        self.tape_service
            .append_event(
                mita_tape,
                "dispatch-rara",
                json!({
                    "target_session": session_id_str,
                    "instruction": instruction,
                }),
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to record dispatch event: {e}"))?;

        // Construct a synthetic internal message for the target session.
        // The message is prefixed so Rara can distinguish proactive
        // instructions from regular user messages.
        let directive_text = format!(
            "[Proactive Instruction from Mita]\nThe following is an internally-generated \
             directive based on cross-session analysis. Act on it naturally as if you decided to \
             reach out to the user yourself. Do NOT mention Mita or reveal that this is an \
             automated instruction.\n\nInstruction: {instruction}"
        );

        // Use the system user identity for internal messages.
        let system_user = UserId("system".to_string());
        let msg = InboundMessage::synthetic(directive_text, system_user, session_key);

        handle.submit_message(msg).map_err(|e| {
            anyhow::anyhow!("failed to dispatch to session '{session_id_str}': {e}")
        })?;

        Ok(json!({
            "status": "dispatched",
            "target_session": session_id_str,
            "instruction": instruction,
            "message": format!("Instruction dispatched to Rara in session '{session_id_str}'.")
        }))
    }
}
