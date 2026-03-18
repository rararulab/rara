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
//! Agents are resolved dynamically from the
//! [`AcpRegistry`](rara_acp::AcpRegistry).

use std::{future::Future, path::PathBuf, pin::Pin};

use async_trait::async_trait;
use rara_acp::{
    AcpThread, PermissionRequestInfo, RequestPermissionOutcome, SelectedPermissionOutcome,
    events::{AcpEvent, StopReason, ToolCallStatus},
    registry::AcpRegistryRef,
};
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Parameters for delegating a task to an ACP agent.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AcpDelegateParams {
    /// Name of the ACP agent to delegate to (e.g. 'claude', 'codex', 'gemini',
    /// or any custom agent).
    agent:  String,
    /// The task instruction to send to the agent.
    prompt: String,
    /// Working directory for the agent subprocess (defaults to workspace root).
    cwd:    Option<String>,
}

/// A tool call event from the delegated agent.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolCallEvent {
    /// The agent started a new tool call.
    Started { id: String, title: String },
    /// A progress or completion update for an in-flight tool call.
    Updated {
        id:     String,
        status: DelegateToolCallStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<String>,
    },
}

/// Status of a tool call within a delegated agent session.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegateToolCallStatus {
    Running,
    Completed,
    Failed,
}

/// File operation performed by the delegated agent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegateFileOp {
    Read,
    Write,
}

/// Summary of a file access by the delegated agent.
#[derive(Debug, Serialize)]
pub struct FileAccessSummary {
    path:      String,
    operation: DelegateFileOp,
}

/// Why the delegated agent stopped.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegateStopReason {
    EndTurn,
    MaxTokens,
    Refusal,
    Cancelled,
    Error,
}

/// Result of an ACP delegation.
#[derive(Debug, Serialize)]
pub struct AcpDelegateResult {
    text:           String,
    stop_reason:    DelegateStopReason,
    tool_calls:     Vec<ToolCallEvent>,
    files_accessed: Vec<FileAccessSummary>,
}

/// Tool that delegates a task to an external coding agent via ACP.
///
/// The tool spawns the requested agent as a subprocess, communicates using
/// the Agent Communication Protocol (stdin/stdout JSON-RPC), and collects
/// the agent's text output and tool call summaries into a single JSON
/// response.  Agents are resolved from the
/// [`AcpRegistry`](rara_acp::AcpRegistry) at runtime.
#[derive(ToolDef)]
#[tool(
    name = "acp-delegate",
    description = "Delegate a task to an external coding agent via the Agent Communication \
                   Protocol. The agent runs as a subprocess, executes the prompt, and returns its \
                   text output and tool call summary. Use list-acp-agents to see available agents."
)]
pub struct AcpDelegateTool {
    registry: AcpRegistryRef,
}

impl AcpDelegateTool {
    /// Create a new instance backed by the given agent registry.
    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for AcpDelegateTool {
    type Output = AcpDelegateResult;
    type Params = AcpDelegateParams;

    async fn run(
        &self,
        params: AcpDelegateParams,
        _context: &ToolContext,
    ) -> anyhow::Result<AcpDelegateResult> {
        let cwd = params
            .cwd
            .map(PathBuf::from)
            .unwrap_or_else(|| rara_paths::workspace_dir().clone());

        // Resolve agent from registry.
        let config = self
            .registry
            .get(&params.agent)
            .await
            .map_err(|e| anyhow::anyhow!("failed to look up agent '{}': {e}", params.agent))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown ACP agent '{}'. Use list-acp-agents to see available agents.",
                    params.agent
                )
            })?;

        if !config.enabled {
            return Err(anyhow::anyhow!("ACP agent '{}' is disabled", params.agent));
        }

        let command = config.to_agent_command();
        let full_cmd = std::iter::once(command.program.as_str())
            .chain(command.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        // Spawn the AcpThread — handles subprocess, handshake, and session
        // creation internally.
        let mut thread = AcpThread::spawn(&params.agent, command, cwd)
            .await
            .map_err(|e| anyhow::anyhow!("ACP spawn failed (cmd: `{full_cmd}`): {e}"))?;

        // Collect streaming events into structured output.
        let mut text_chunks: Vec<String> = Vec::new();
        let mut tool_calls: Vec<ToolCallEvent> = Vec::new();
        let mut files_accessed: Vec<FileAccessSummary> = Vec::new();

        let stop_reason = thread
            .prompt(
                &params.prompt,
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

        let delegate_stop = match &stop_reason {
            StopReason::EndTurn => DelegateStopReason::EndTurn,
            StopReason::MaxTokens => DelegateStopReason::MaxTokens,
            StopReason::Refusal => DelegateStopReason::Refusal,
            StopReason::Cancelled => DelegateStopReason::Cancelled,
            StopReason::Error(_) => DelegateStopReason::Error,
        };

        Ok(AcpDelegateResult {
            text: text_chunks.join(""),
            stop_reason: delegate_stop,
            tool_calls,
            files_accessed,
        })
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
    tool_calls: &mut Vec<ToolCallEvent>,
    files_accessed: &mut Vec<FileAccessSummary>,
) {
    match event {
        AcpEvent::Text(text) => text_chunks.push(text.clone()),
        AcpEvent::ToolCallStarted { id, title } => {
            tool_calls.push(ToolCallEvent::Started {
                id:    id.clone(),
                title: title.clone(),
            });
        }
        AcpEvent::ToolCallUpdate { id, status, output } => {
            let status = match status {
                ToolCallStatus::Running => DelegateToolCallStatus::Running,
                ToolCallStatus::Completed => DelegateToolCallStatus::Completed,
                ToolCallStatus::Failed => DelegateToolCallStatus::Failed,
            };
            tool_calls.push(ToolCallEvent::Updated {
                id: id.clone(),
                status,
                output: output.clone(),
            });
        }
        AcpEvent::FileAccess { path, operation } => {
            let op = match operation {
                rara_acp::events::FileOperation::Read => DelegateFileOp::Read,
                rara_acp::events::FileOperation::Write => DelegateFileOp::Write,
            };
            files_accessed.push(FileAccessSummary {
                path:      path.display().to_string(),
                operation: op,
            });
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
