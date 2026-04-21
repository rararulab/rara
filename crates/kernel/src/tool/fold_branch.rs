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
//!
//! ## Design decisions
//!
//! **Tool inheritance (privilege boundary):**  When the caller omits the
//! `tools` parameter, the child inherits the *parent's* tool whitelist rather
//! than receiving an empty list.  In the tool registry an empty `tools` vec
//! means "all tools allowed", so defaulting to `vec![]` would silently
//! escalate the child's privileges beyond the parent's scope.
//!
//! **Timeout → terminate:**  If the child exceeds `timeout_secs`, we send
//! `Signal::Terminate` to the child session.  Without this the child would
//! continue running, holding its parent-child semaphore permit and potentially
//! producing side-effects (tape writes, tool calls) after the parent has
//! already returned a timeout result.
//!
//! **No duplicate tape persist:**  `Kernel::handle_child_completed` normally
//! appends every child's output to the parent's tape as a system message.
//! For fold-branch children this would duplicate the content that is already
//! returned inline as a `ToolResult` (and persisted by the agent loop's
//! standard tool-result recording).  We use the [`FOLD_BRANCH_NAME_PREFIX`]
//! convention to let `handle_child_completed` skip the tape append for
//! fold-branch children.

use std::sync::Arc;

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use tracing::{info, warn};

use crate::{
    agent::{AgentManifest, fold::ContextFolder},
    handle::KernelHandle,
    io::AgentEvent,
    session::{SessionKey, Signal},
    tool::{ToolContext, ToolExecute},
};

/// Maximum character length for the compressed result returned to the caller.
/// Results exceeding this threshold are compressed via
/// [`ContextFolder::fold_text`] before being returned as a `ToolResult`.
const COMPACT_TARGET_CHARS: usize = 2000;

/// Name prefix for fold-branch child agents.
///
/// Used by [`Kernel::handle_child_completed`](crate::kernel::Kernel) to
/// identify fold-branch children and skip the automatic tape-persist step.
/// The result is already delivered inline as a `ToolResult`, so persisting
/// it again as a system message would duplicate the content in the parent's
/// conversation history and accelerate tape bloat.
pub(crate) const FOLD_BRANCH_NAME_PREFIX: &str = "fold-branch-";

/// Builtin tool that spawns a child agent, waits for it to complete, and
/// returns a compressed version of the result.
///
/// The child runs in an isolated context window with its own tape.  On
/// completion the raw output is compressed via [`ContextFolder::fold_text`]
/// (if it exceeds [`COMPACT_TARGET_CHARS`]) and returned as a JSON
/// `ToolResult` to the parent's agent loop.
#[derive(ToolDef)]
#[tool(
    name = "fold-branch",
    description = "Spawn a child agent for a focused sub-task, wait for completion, and return a \
                   compressed result. Use this when you need the result inline (synchronous). For \
                   fire-and-forget tasks, use spawn-background instead.",
    tier = "deferred"
)]
pub struct FoldBranchTool {
    handle:         KernelHandle,
    session_key:    SessionKey,
    context_folder: Arc<ContextFolder>,
}

impl FoldBranchTool {
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

/// Parameters for the `fold-branch` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FoldBranchParams {
    /// Short name describing the sub-task.
    task:           String,
    /// Detailed instruction for the child agent.
    instruction:    String,
    /// Tool names the child agent can use (inherits parent if omitted).
    tools:          Option<Vec<crate::tool::ToolName>>,
    /// Maximum LLM iterations for the child agent.
    max_iterations: Option<usize>,
    /// Timeout in seconds for the child agent (default: 120).
    timeout_secs:   Option<u64>,
}

#[async_trait]
impl ToolExecute for FoldBranchTool {
    type Output = serde_json::Value;
    type Params = FoldBranchParams;

    async fn run(
        &self,
        p: FoldBranchParams,
        _context: &ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        // SECURITY: When tools is omitted, inherit the parent's tool whitelist.
        // An empty `tools` vec in AgentManifest means "allow ALL tools" (see
        // ToolRegistry::build_for_agent), so defaulting to `vec![]` would
        // silently escalate the child's privileges beyond the parent's scope.
        let tools = match p.tools {
            Some(t) => t,
            None => self
                .handle
                .process_table()
                .with(&self.session_key, |proc| proc.manifest.tools.clone())
                .unwrap_or_default(),
        };

        let timeout_secs = p.timeout_secs.unwrap_or(120);

        // The name prefix is significant: `handle_child_completed` uses it to
        // identify fold-branch children and skip the automatic tape-persist
        // step (see FOLD_BRANCH_NAME_PREFIX doc comment).
        let manifest = AgentManifest {
            name: format!("{}{}", FOLD_BRANCH_NAME_PREFIX, p.task),
            role: Default::default(),
            description: format!("Fold-branch child for: {}", p.task),
            model: None,
            system_prompt: format!(
                "You are a focused sub-agent. Complete the assigned task concisely.{suffix}",
                suffix = crate::agent::STRUCTURED_OUTPUT_SUFFIX,
            ),
            soul_prompt: None,
            provider_hint: None,
            max_iterations: p.max_iterations,
            tools,
            excluded_tools: vec![],
            max_children: Some(0),
            max_context_tokens: None,
            priority: Default::default(),
            metadata: Default::default(),
            sandbox: None,
            default_execution_mode: None,
            tool_call_limit: None,
            max_continuations: Some(0),
            worker_timeout_secs: None,
            max_output_chars: None,
        };

        // Resolve principal from parent session.
        let principal = self
            .handle
            .process_table()
            .with(&self.session_key, |proc| proc.principal.clone())
            .ok_or_else(|| anyhow::anyhow!("parent session not found: {}", self.session_key))?;

        info!(
            parent = %self.session_key,
            task = %p.task,
            timeout_secs = timeout_secs,
            "spawning fold-branch child agent"
        );

        let agent_handle = self
            .handle
            .spawn_child(self.session_key, &principal, manifest, p.instruction)
            .await
            .map_err(|e| anyhow::anyhow!("fold-branch spawn failed: {e}"))?;

        let child_key = agent_handle.session_key;

        // Block until the child finishes or the timeout fires.
        // Unlike spawn_background (fire-and-forget), we consume the result_rx
        // synchronously so the parent turn cannot proceed until we have the
        // child's output.
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
                }));
            }
        };

        // Compress the result via an independent LLM call if it exceeds the
        // target size.  On compression failure, fall back to the raw text to
        // avoid losing the child's output entirely.
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
            "result": compressed,
            "note": "This result is authoritative. Do NOT re-read the same files locally."
        }))
    }
}
