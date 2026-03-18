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
    memory::TapeService,
    session::SessionKey,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::RwLock;

use super::notify::push_notification;

/// Input parameters for the dispatch-rara tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DispatchRaraParams {
    /// The target session key where Rara should act.
    session_id:  String,
    /// Specific instruction for Rara.
    instruction: String,
}

/// Mita tool that dispatches an instruction to Rara for a given session.
///
/// The instruction is delivered as a synthetic internal message to the
/// target session, prefixed with a system marker so Rara knows it comes
/// from Mita's proactive analysis (not the user).
///
/// The `KernelHandle` is set after kernel startup via the `handle_ref` hook.
#[derive(ToolDef)]
#[tool(
    name = "dispatch-rara",
    description = "Dispatch a proactive instruction to Rara for a specific session. Rara will \
                   receive the instruction as a system-level directive and generate an \
                   appropriate response to the user. Use this when you determine a user needs \
                   proactive attention."
)]
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
impl ToolExecute for DispatchRaraTool {
    type Output = Value;
    type Params = DispatchRaraParams;

    async fn run(&self, params: DispatchRaraParams, ctx: &ToolContext) -> anyhow::Result<Value> {
        let handle = self.kernel_handle.read().await;
        let handle = handle
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("kernel handle not yet available"))?;

        let session_key = SessionKey::try_from_raw(&params.session_id)
            .map_err(|_| anyhow::anyhow!("invalid session key: {}", params.session_id))?;

        // Verify target session is alive before recording dispatch.
        if !handle.process_table().contains(&session_key) {
            return Ok(json!({
                "status": "error",
                "target_session": params.session_id,
                "message": format!("Target session '{}' is not active. Dispatch skipped.", params.session_id)
            }));
        }

        // Record the dispatch in Mita's tape for future reference (avoid repeats).
        let mita_tape = &SessionKey::deterministic("mita").to_string();
        self.tape_service
            .append_event(
                mita_tape,
                "dispatch-rara",
                json!({
                    "target_session": params.session_id,
                    "instruction": params.instruction,
                }),
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to record dispatch event: {e}"))?;

        handle
            .dispatch_directive(session_key, params.instruction.clone())
            .map_err(|e| {
                anyhow::anyhow!(
                    "failed to dispatch directive to session '{}': {e}",
                    params.session_id
                )
            })?;

        push_notification(
            ctx,
            format!(
                "\u{1f4e8} Mita \u{2192} Rara [{}]: {}",
                params.session_id, params.instruction
            ),
        );

        Ok(json!({
            "status": "dispatched",
            "target_session": params.session_id,
            "instruction": params.instruction,
            "message": format!("Instruction dispatched to Rara in session '{}'.", params.session_id)
        }))
    }
}
