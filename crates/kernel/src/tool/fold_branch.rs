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

//! Synchronous child-spawning tool that waits for completion and compresses
//! the result via [`ContextFolder`].
//!
//! Unlike [`SpawnBackgroundTool`](super::spawn_background::SpawnBackgroundTool)
//! which is fire-and-forget, `FoldBranchTool` blocks until the child agent
//! finishes and returns a compressed summary.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    agent::{AgentManifest, fold::ContextFolder},
    handle::KernelHandle,
    io::AgentEvent,
    session::{SessionKey, Signal},
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Maximum character length for the compressed result returned to the caller.
const COMPACT_TARGET_CHARS: usize = 2000;

/// Name prefix for fold-branch child agents.  Used by
/// `handle_child_completed` to skip tape persistence (the result is already
/// returned inline as a ToolResult).
pub(crate) const FOLD_BRANCH_NAME_PREFIX: &str = "fold-branch-";

/// Builtin tool that spawns a child agent, waits for it to complete, and
/// returns a compressed version of the result.
pub struct FoldBranchTool {
    handle:         KernelHandle,
    session_key:    SessionKey,
    context_folder: Arc<ContextFolder>,
}

impl FoldBranchTool {
    pub const NAME: &str = crate::tool_names::FOLD_BRANCH;

    pub fn new(
        handle: KernelHandle,
        session_key: SessionKey,
        context_folder: Arc<ContextFolder>,
    ) -> Self {
        Self {
            handle,
            session_key,
            context_folder,
        }
    }
}

#[async_trait]
impl AgentTool for FoldBranchTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Spawn a child agent for a focused sub-task, wait for completion, and return a compressed \
         result. Use this when you need the result inline (synchronous). For fire-and-forget \
         tasks, use spawn-background instead."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["task", "instruction"],
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Short name describing the sub-task"
                },
                "instruction": {
                    "type": "string",
                    "description": "Detailed instruction for the child agent"
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tool names the child agent can use (inherits parent if omitted)"
                },
                "max_iterations": {
                    "type": "integer",
                    "description": "Maximum LLM iterations for the child agent"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds for the child agent (default: 120)"
                }
            }
        })
    }

    async fn execute(&self, params: Value, _context: &ToolContext) -> anyhow::Result<ToolOutput> {
        let task = params["task"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required field: task"))?
            .to_string();
        let instruction = params["instruction"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required field: instruction"))?
            .to_string();
        // When tools is omitted, inherit the parent's tool set rather than
        // granting all tools (empty vec = all tools in the registry).
        let tools: Vec<String> = match params.get("tools") {
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| anyhow::anyhow!("invalid tools array: {e}"))?,
            None => self
                .handle
                .process_table()
                .with(&self.session_key, |p| p.manifest.tools.clone())
                .unwrap_or_default(),
        };
        let max_iterations = params
            .get("max_iterations")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(120);

        let manifest = AgentManifest {
            name: format!("{}{}", FOLD_BRANCH_NAME_PREFIX, task),
            role: Default::default(),
            description: format!("Fold-branch child for: {task}"),
            model: None,
            system_prompt: "You are a focused sub-agent. Complete the assigned task concisely."
                .to_string(),
            soul_prompt: None,
            provider_hint: None,
            max_iterations,
            tools,
            max_children: Some(0),
            max_context_tokens: None,
            priority: Default::default(),
            metadata: Default::default(),
            sandbox: None,
            default_execution_mode: None,
        };

        // Resolve principal from parent session.
        let principal = self
            .handle
            .process_table()
            .with(&self.session_key, |p| p.principal.clone())
            .ok_or_else(|| anyhow::anyhow!("parent session not found: {}", self.session_key))?;

        info!(
            parent = %self.session_key,
            task = %task,
            timeout_secs = timeout_secs,
            "spawning fold-branch child agent"
        );

        let agent_handle = self
            .handle
            .spawn_child(&self.session_key, &principal, manifest, instruction)
            .await
            .map_err(|e| anyhow::anyhow!("fold-branch spawn failed: {e}"))?;

        let child_key = agent_handle.session_key;

        // Wait for child completion with timeout.
        let mut rx = agent_handle.result_rx;
        let timeout = tokio::time::Duration::from_secs(timeout_secs);
        let result_text = match tokio::time::timeout(timeout, async {
            let mut final_result = None;
            while let Some(event) = rx.recv().await {
                if let AgentEvent::Done(result) = event {
                    final_result = Some(result.output);
                    break;
                }
            }
            final_result
        })
        .await
        {
            Ok(Some(text)) => text,
            Ok(None) => "(child agent completed with no output)".to_string(),
            Err(_) => {
                // Terminate the child to release its semaphore permit and
                // prevent it from continuing to run or writing results.
                if let Err(e) = self.handle.send_signal(child_key, Signal::Terminate) {
                    warn!(
                        error = %e,
                        child = %child_key,
                        "fold-branch: failed to terminate timed-out child"
                    );
                }
                return Ok(serde_json::json!({
                    "task_id": child_key.to_string(),
                    "status": "timeout",
                    "message": format!("fold-branch child timed out after {timeout_secs}s")
                })
                .into());
            }
        };

        // Compress the result if it exceeds the target size.
        let compressed = if result_text.len() > COMPACT_TARGET_CHARS {
            self.context_folder
                .fold_text(&result_text, COMPACT_TARGET_CHARS)
                .await
                .unwrap_or(result_text)
        } else {
            result_text
        };

        Ok(serde_json::json!({
            "task_id": child_key.to_string(),
            "status": "completed",
            "result": compressed
        })
        .into())
    }
}
