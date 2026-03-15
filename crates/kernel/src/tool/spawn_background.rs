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
    agent::AgentManifest,
    handle::KernelHandle,
    io::{AgentEvent, StreamEvent},
    session::{BackgroundTaskEntry, SessionKey},
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Builtin tool that spawns a background agent for long-running tasks.
///
/// The agent runs independently — the parent's turn continues and completes
/// normally. When the background agent finishes, the kernel triggers a
/// proactive turn on the parent to deliver the result.
pub struct SpawnBackgroundTool {
    handle:      KernelHandle,
    session_key: SessionKey,
}

impl SpawnBackgroundTool {
    pub fn new(handle: KernelHandle, session_key: SessionKey) -> Self {
        Self { handle, session_key }
    }
}

#[async_trait]
impl AgentTool for SpawnBackgroundTool {
    fn name(&self) -> &str { "spawn_background" }

    fn description(&self) -> &str {
        "Spawn a background agent to handle a long-running task. The agent runs \
         independently and results are delivered when complete. You cannot interact \
         with the background agent after spawning it."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["manifest", "input", "description"],
            "properties": {
                "manifest": {
                    "type": "object",
                    "description": "Agent manifest for the background agent",
                    "required": ["name", "system_prompt"],
                    "properties": {
                        "name": { "type": "string", "description": "Unique name for this background agent" },
                        "description": { "type": "string", "description": "Agent description" },
                        "system_prompt": { "type": "string", "description": "System prompt defining agent behavior" },
                        "model": { "type": "string", "description": "LLM model identifier (inherits parent if omitted)" },
                        "tools": { "type": "array", "items": { "type": "string" }, "description": "Tool names this agent can use" },
                        "max_iterations": { "type": "integer", "description": "Maximum LLM iterations" }
                    }
                },
                "input": { "type": "string", "description": "The task instruction to send to the background agent" },
                "description": { "type": "string", "description": "Short human-readable description of the task (shown in status)" }
            }
        })
    }

    async fn execute(&self, params: Value, _context: &ToolContext) -> anyhow::Result<ToolOutput> {
        let input = params["input"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required field: input"))?
            .to_string();
        let description = params["description"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required field: description"))?
            .to_string();
        let manifest_value = params
            .get("manifest")
            .ok_or_else(|| anyhow::anyhow!("missing required field: manifest"))?;

        let manifest: AgentManifest = serde_json::from_value(manifest_value.clone())
            .map_err(|e| anyhow::anyhow!("invalid manifest: {e}"))?;

        // Resolve principal from parent session.
        let principal = self
            .handle
            .process_table()
            .with(&self.session_key, |p| p.principal.clone())
            .ok_or_else(|| anyhow::anyhow!("parent session not found: {}", self.session_key))?;

        info!(
            parent = %self.session_key,
            agent = %manifest.name,
            description = %description,
            "spawning background agent"
        );

        let agent_handle = self
            .handle
            .spawn_child(&self.session_key, &principal, manifest.clone(), input)
            .await
            .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?;

        let child_key = agent_handle.session_key;

        // Register as background task on parent session.
        self.handle.register_background_task(
            &self.session_key,
            BackgroundTaskEntry {
                child_key,
                agent_name: manifest.name.clone(),
                description: description.clone(),
                created_at: jiff::Timestamp::now(),
            },
        );

        // Emit BackgroundTaskStarted to parent's active streams so clients
        // can display an ongoing status indicator with elapsed timer.
        self.handle
            .stream_hub()
            .emit_to_session(&self.session_key, StreamEvent::BackgroundTaskStarted {
                task_id:     child_key.to_string(),
                agent_name:  manifest.name.clone(),
                description: description.clone(),
            });

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
        })
        .into())
    }
}
