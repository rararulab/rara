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

//! Background task tool — delegates work to a child agent with preset
//! configurations.

pub(crate) mod presets;

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use tracing::info;

use crate::{
    agent::{AgentManifest, AgentRole, Priority},
    handle::KernelHandle,
    io::{AgentEvent, StreamEvent},
    session::{BackgroundTaskEntry, SessionKey},
    tool::{ToolContext, ToolExecute, spawn_background::slug_from_description},
};

/// Builtin tool that launches a background task using a predefined agent type.
///
/// Instead of requiring raw manifest parameters, callers pick a `task_type`
/// (e.g. `"general-purpose"`, `"bash"`) and the tool resolves all configuration
/// from the preset registry.
#[derive(ToolDef)]
#[tool(
    name = "task",
    description = "Launch a background task using a predefined agent type. Pick a task_type \
                   ('general-purpose' for complex multi-step tasks, 'bash' for shell/CLI \
                   operations) and provide the task prompt. The agent runs independently and \
                   results are delivered when complete."
)]
pub struct TaskTool {
    handle:      KernelHandle,
    session_key: SessionKey,
}

impl TaskTool {
    /// Create a new `TaskTool` bound to a parent session.
    pub fn new(handle: KernelHandle, session_key: SessionKey) -> Self {
        Self {
            handle,
            session_key,
        }
    }
}

/// Parameters for the `task` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskParams {
    /// Short human-readable description of the task (3-5 words, shown in
    /// status).
    description: String,
    /// The task instruction to send to the background agent.
    prompt:      String,
    /// Predefined agent type: 'general-purpose' (full tool access, complex
    /// tasks) or 'bash' (shell/CLI specialist).
    task_type:   String,
}

#[async_trait]
impl ToolExecute for TaskTool {
    type Output = serde_json::Value;
    type Params = TaskParams;

    async fn run(&self, p: TaskParams, context: &ToolContext) -> anyhow::Result<serde_json::Value> {
        let preset = presets::get_preset(&p.task_type).ok_or_else(|| {
            let available = presets::list_preset_names().join(", ");
            anyhow::anyhow!(
                "unknown task_type '{}'. Available types: {}",
                p.task_type,
                available
            )
        })?;

        let agent_name = slug_from_description(&p.description);

        // Build manifest from preset configuration.
        let mut manifest = AgentManifest {
            name:                   agent_name,
            role:                   AgentRole::Worker,
            description:            p.description.clone(),
            model:                  None,
            system_prompt:          preset.system_prompt.to_owned(),
            soul_prompt:            None,
            provider_hint:          None,
            max_iterations:         Some(preset.max_iterations),
            tools:                  preset.allowed_tools.clone(),
            max_children:           Some(0),
            max_context_tokens:     None,
            priority:               Priority::default(),
            metadata:               serde_json::Value::Null,
            sandbox:                None,
            default_execution_mode: None,
            tool_call_limit:        None,
            worker_timeout_secs:    None,
        };

        // Append structured-output instructions so the background agent
        // self-summarizes before returning results to the parent.
        manifest
            .system_prompt
            .push_str(crate::agent::STRUCTURED_OUTPUT_SUFFIX);

        // Resolve principal from parent session.
        let principal = self
            .handle
            .process_table()
            .with(&self.session_key, |proc| proc.principal.clone())
            .ok_or_else(|| anyhow::anyhow!("parent session not found: {}", self.session_key))?;

        info!(
            parent = %self.session_key,
            agent = %manifest.name,
            task_type = %p.task_type,
            description = %p.description,
            "spawning task agent"
        );

        let agent_handle = self
            .handle
            .spawn_child(&self.session_key, &principal, manifest.clone(), p.prompt)
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

        // Emit BackgroundTaskStarted to parent's active streams so clients
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
            "task_type": p.task_type,
            "status": "spawned",
            "message": "Background agent is now running. Results will be delivered when complete."
        }))
    }
}
