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
    agent::AgentManifest,
    handle::KernelHandle,
    io::{AgentEvent, StreamEvent},
    session::{BackgroundTaskEntry, SessionKey},
    tool::{ToolContext, ToolExecute},
};

/// Builtin tool that spawns a background agent for long-running tasks.
///
/// The agent runs independently — the parent\'s turn continues and completes
/// normally. When the background agent finishes, the kernel triggers a
/// proactive turn on the parent to deliver the result.
#[derive(ToolDef)]
#[tool(
    name = "spawn-background",
    description = "Spawn a background agent to handle a long-running task. The agent runs \
                   independently and results are delivered when complete. You cannot interact \
                   with the background agent after spawning it."
)]
pub struct SpawnBackgroundTool {
    handle:      KernelHandle,
    session_key: SessionKey,
}

impl SpawnBackgroundTool {
    pub fn new(handle: KernelHandle, session_key: SessionKey) -> Self {
        Self {
            handle,
            session_key,
        }
    }
}

/// Parameters for the `spawn-background` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpawnBackgroundParams {
    /// Agent manifest for the background agent (name, system_prompt, etc.).
    manifest:    serde_json::Value,
    /// The task instruction to send to the background agent.
    input:       String,
    /// Short human-readable description of the task (shown in status).
    description: String,
}

#[async_trait]
impl ToolExecute for SpawnBackgroundTool {
    type Output = serde_json::Value;
    type Params = SpawnBackgroundParams;

    async fn run(
        &self,
        p: SpawnBackgroundParams,
        context: &ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let mut manifest: AgentManifest = serde_json::from_value(p.manifest)
            .map_err(|e| anyhow::anyhow!("invalid manifest: {e}"))?;

        // Ensure the background agent includes structured-output instructions
        // so it self-summarizes before returning results to the parent.
        if !manifest.system_prompt.contains("structured result") {
            manifest
                .system_prompt
                .push_str(crate::agent::STRUCTURED_OUTPUT_SUFFIX);
        }

        // Resolve principal from parent session.
        let principal = self
            .handle
            .process_table()
            .with(&self.session_key, |proc| proc.principal.clone())
            .ok_or_else(|| anyhow::anyhow!("parent session not found: {}", self.session_key))?;

        info!(
            parent = %self.session_key,
            agent = %manifest.name,
            description = %p.description,
            "spawning background agent"
        );

        let agent_handle = self
            .handle
            .spawn_child(&self.session_key, &principal, manifest.clone(), p.input)
            .await
            .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?;

        let child_key = agent_handle.session_key;

        // Register as background task on parent session.
        self.handle.register_background_task(
            &self.session_key,
            BackgroundTaskEntry {
                child_key,
                agent_name: manifest.name.clone(),
                description: p.description.clone(),
                created_at: jiff::Timestamp::now(),
                trigger_message_id: context.rara_message_id.clone(),
            },
        );

        // Emit BackgroundTaskStarted to parent\'s active streams so clients
        // can display an ongoing status indicator with elapsed timer.
        self.handle.stream_hub().emit_to_session(
            &self.session_key,
            StreamEvent::BackgroundTaskStarted {
                task_id:     child_key.to_string(),
                agent_name:  manifest.name.clone(),
                description: p.description.clone(),
            },
        );

        // Spawn fire-and-forget watcher to drain result_rx.
        tokio::spawn(async move {
            let mut rx = agent_handle.result_rx;
            while let Some(event) = rx.recv().await {
                if matches!(event, AgentEvent::Done(_)) {
                    break;
                }
            }
        });

        Ok(serde_json::json!({
            "task_id": child_key.to_string(),
            "agent_name": manifest.name,
            "status": "spawned",
            "message": "Background agent is now running. Results will be delivered when complete."
        }))
    }
}
