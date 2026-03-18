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

//! ACP delegate tool — dispatches a task to an external coding agent via ACP.
//!
//! Spawns a child agent process, sends a prompt over the Agent Communication
//! Protocol, collects streaming events, and returns a structured JSON summary.
//! Agents are resolved dynamically from the [`AcpRegistry`].

use std::{future::Future, path::PathBuf, pin::Pin};

use rara_acp::{
    AcpThread, PermissionRequestInfo, RequestPermissionOutcome, SelectedPermissionOutcome,
    events::{AcpEvent, StopReason, ToolCallStatus},
    registry::AcpRegistryRef,
};
use rara_kernel::tool::{ToolContext, ToolOutput};
use rara_tool_macro::ToolDef;
use serde_json::json;
use tracing::{debug, warn};

/// Tool that delegates a task to an external coding agent via ACP.
///
/// The tool spawns the requested agent as a subprocess, communicates using
/// the Agent Communication Protocol (stdin/stdout JSON-RPC), and collects
/// the agent's text output and tool call summaries into a single JSON
/// response.  Agents are resolved from the [`AcpRegistry`] at runtime.
#[derive(ToolDef)]
#[tool(
    name = "acp-delegate",
    description = "Delegate a task to an external coding agent via the Agent Communication \
                   Protocol. The agent runs as a subprocess, executes the prompt, and returns its \
                   text output and tool call summary. Use list-acp-agents to see available agents.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct AcpDelegateTool {
    registry: AcpRegistryRef,
}

impl AcpDelegateTool {
    /// Create a new instance backed by the given agent registry.
    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }

    fn schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Name of the ACP agent to delegate to (e.g. 'claude', 'codex', 'gemini', or any custom agent)"
                },
                "prompt": {
                    "type": "string",
                    "description": "The task instruction to send to the agent"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the agent subprocess (defaults to workspace root)"
                }
            },
            "required": ["agent", "prompt"]
        })
    }

    async fn exec(
        &self,
        params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let agent_name = params
            .get("agent")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: agent"))?;

        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: prompt"))?;

        let cwd = params
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| rara_paths::workspace_dir().clone());

        // Resolve agent from registry.
        let config = self
            .registry
            .get(agent_name)
            .await
            .map_err(|e| anyhow::anyhow!("failed to look up agent '{agent_name}': {e}"))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown ACP agent '{agent_name}'. Use list-acp-agents to see available \
                     agents."
                )
            })?;

        if !config.enabled {
            return Err(anyhow::anyhow!("ACP agent '{agent_name}' is disabled"));
        }

        let command = config.to_agent_command();

        // Spawn the AcpThread — handles subprocess, handshake, and session
        // creation internally.
        let mut thread = AcpThread::spawn(agent_name, command, cwd)
            .await
            .map_err(|e| anyhow::anyhow!("ACP spawn failed: {e}"))?;

        // Collect streaming events into structured output.
        let mut text_chunks: Vec<String> = Vec::new();
        let mut tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut files_accessed: Vec<serde_json::Value> = Vec::new();

        let stop_reason = thread
            .prompt(
                prompt,
                |event| {
                    collect_event(
                        event,
                        &mut text_chunks,
                        &mut tool_calls,
                        &mut files_accessed,
                    );
                },
                // Auto-approve all permission requests.  When ToolContext
                // gains ApprovalManager support, this should forward to the
                // user for interactive confirmation instead.
                auto_approve_resolver,
            )
            .await
            .map_err(|e| anyhow::anyhow!("ACP prompt failed: {e}"))?;

        // Graceful shutdown — kills subprocess and reaps.
        if let Err(e) = thread.shutdown().await {
            warn!(error = %e, "ACP shutdown error (non-fatal)");
        }

        let stop_reason_str = match &stop_reason {
            StopReason::EndTurn => "end_turn",
            StopReason::MaxTokens => "max_tokens",
            StopReason::Refusal => "refusal",
            StopReason::Cancelled => "cancelled",
            StopReason::Error(_) => "error",
        };

        let combined_text = text_chunks.join("");
        Ok(json!({
            "text": combined_text,
            "stop_reason": stop_reason_str,
            "tool_calls": tool_calls,
            "files_accessed": files_accessed,
        })
        .into())
    }
}

/// Auto-approve permission resolver: selects AllowAlways > AllowOnce > first.
///
/// This is a placeholder until ToolContext exposes ApprovalManager for
/// interactive permission forwarding.
fn auto_approve_resolver(
    info: PermissionRequestInfo,
) -> Pin<Box<dyn Future<Output = RequestPermissionOutcome> + Send>> {
    Box::pin(async move {
        let selected = info
            .options
            .iter()
            .find(|o| o.kind == "AllowAlways")
            .or_else(|| info.options.iter().find(|o| o.kind == "AllowOnce"));

        let option_id = match selected {
            Some(opt) => opt.id.clone(),
            None => match info.options.first() {
                Some(first) => first.id.clone(),
                None => return RequestPermissionOutcome::Cancelled,
            },
        };

        debug!(option_id = %option_id, tool = %info.tool_title, "auto-approved permission");
        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id))
    })
}

/// Accumulate a single ACP event into the output collectors.
fn collect_event(
    event: &AcpEvent,
    text_chunks: &mut Vec<String>,
    tool_calls: &mut Vec<serde_json::Value>,
    files_accessed: &mut Vec<serde_json::Value>,
) {
    match event {
        AcpEvent::Text(text) => text_chunks.push(text.clone()),
        AcpEvent::ToolCallStarted { id, title } => {
            tool_calls.push(json!({
                "id": id,
                "title": title,
                "status": "started",
            }));
        }
        AcpEvent::ToolCallUpdate { id, status, output } => {
            let status_str = match status {
                ToolCallStatus::Running => "running",
                ToolCallStatus::Completed => "completed",
                ToolCallStatus::Failed => "failed",
            };
            tool_calls.push(json!({
                "id": id,
                "status": status_str,
                "output": output,
            }));
        }
        AcpEvent::FileAccess { path, operation } => {
            let op = match operation {
                rara_acp::events::FileOperation::Read => "read",
                rara_acp::events::FileOperation::Write => "write",
            };
            files_accessed.push(json!({
                "path": path.display().to_string(),
                "operation": op,
            }));
        }
        AcpEvent::Plan { title, steps } => {
            debug!(title = ?title, steps = steps.len(), "agent plan received");
        }
        AcpEvent::PermissionAutoApproved { description } => {
            debug!(description, "permission auto-approved");
        }
        _ => {}
    }
}
