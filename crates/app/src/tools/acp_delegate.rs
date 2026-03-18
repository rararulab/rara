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
//! Spawns a child agent process (Claude, Codex, Gemini, or custom), sends a
//! prompt over the Agent Communication Protocol, collects streaming events,
//! and returns a structured JSON summary.

use std::path::PathBuf;

use async_trait::async_trait;
use rara_acp::{
    AcpConnection,
    events::AcpEvent,
    registry::{AgentKind, AgentRegistry},
};
use rara_kernel::tool::{AgentTool, ToolOutput};
use serde_json::json;
use tracing::{debug, warn};

/// Tool that delegates a task to an external coding agent via ACP.
///
/// The tool spawns the requested agent as a subprocess, communicates using
/// the Agent Communication Protocol (stdin/stdout JSON-RPC), and collects
/// the agent's text output and tool call summaries into a single JSON
/// response.
pub struct AcpDelegateTool;

impl AcpDelegateTool {
    /// Canonical tool name.
    pub const NAME: &str = rara_kernel::tool_names::ACP_DELEGATE;

    /// Create a new instance.
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for AcpDelegateTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Delegate a task to an external coding agent (Claude, Codex, or Gemini) via the Agent \
         Communication Protocol. The agent runs as a subprocess, executes the prompt, and returns \
         its text output and tool call summary."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Agent to delegate to: 'claude', 'codex', 'gemini', or a custom agent name",
                    "enum": ["claude", "codex", "gemini"]
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

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &rara_kernel::tool::ToolContext,
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

        // Resolve agent kind from the string parameter.
        let agent_kind = match agent_name {
            "claude" => AgentKind::Claude,
            "codex" => AgentKind::Codex,
            "gemini" => AgentKind::Gemini,
            other => AgentKind::Custom(other.to_string()),
        };

        // Resolve agent command from the default registry.
        let registry = AgentRegistry::with_defaults();
        let command = registry.resolve(&agent_kind).ok_or_else(|| {
            anyhow::anyhow!("unknown agent: {agent_name} — not found in ACP registry")
        })?;
        let command = command.clone();

        let prompt = prompt.to_string();

        // The ACP protocol is !Send (uses spawn_local internally).  The
        // AgentTool::execute future must be Send, so we cannot use LocalSet
        // on the current runtime.  Instead we spawn a dedicated
        // single-threaded (current_thread) tokio runtime on a background OS
        // thread and run the entire ACP session there.
        let result = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build current-thread runtime for ACP");

            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                run_acp_session(&command, &cwd, &prompt).await
            })
        })
        .await
        .map_err(|e| anyhow::anyhow!("ACP worker thread panicked: {e}"))?;

        match result {
            Ok(output) => Ok(output.into()),
            Err(e) => Ok(json!({
                "error": format!("{e:#}"),
                "agent": agent_name,
            })
            .into()),
        }
    }
}

/// Run a complete ACP session: connect, create session, send prompt, collect
/// events, and return a JSON summary.
async fn run_acp_session(
    command: &rara_acp::registry::AgentCommand,
    cwd: &std::path::Path,
    prompt: &str,
) -> anyhow::Result<serde_json::Value> {
    // Connect and perform handshake.
    let (mut conn, mut event_rx) = AcpConnection::connect(command, cwd)
        .await
        .map_err(|e| anyhow::anyhow!("ACP connect failed: {e}"))?;

    // Create a new session.
    conn.new_session()
        .await
        .map_err(|e| anyhow::anyhow!("ACP new_session failed: {e}"))?;

    // Send the prompt (this blocks until the agent's turn completes).
    let response = conn
        .send_prompt(prompt)
        .await
        .map_err(|e| anyhow::anyhow!("ACP prompt failed: {e}"))?;

    // Drain all pending events from the delegate channel.
    let mut text_chunks: Vec<String> = Vec::new();
    let mut tool_calls: Vec<serde_json::Value> = Vec::new();
    let mut files_accessed: Vec<serde_json::Value> = Vec::new();

    while let Ok(event) = event_rx.try_recv() {
        match event {
            AcpEvent::Text(text) => {
                text_chunks.push(text);
            }
            AcpEvent::Thinking(_) => {
                // Thinking chunks are internal reasoning; skip in output.
            }
            AcpEvent::ToolCallStarted { id, title } => {
                tool_calls.push(json!({
                    "id": id,
                    "title": title,
                    "status": "started",
                }));
            }
            AcpEvent::ToolCallUpdate { id, status, output } => {
                let status_str = match status {
                    rara_acp::events::ToolCallStatus::Running => "running",
                    rara_acp::events::ToolCallStatus::Completed => "completed",
                    rara_acp::events::ToolCallStatus::Failed => "failed",
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
            AcpEvent::TurnComplete { .. } | AcpEvent::ProcessExited { .. } => {}
        }
    }

    // Clean shutdown of the agent subprocess.
    if let Err(e) = conn.shutdown().await {
        warn!(error = %e, "ACP shutdown error (non-fatal)");
    }

    let stop_reason = format!("{:?}", response.stop_reason);
    let combined_text = text_chunks.join("");

    Ok(json!({
        "text": combined_text,
        "stop_reason": stop_reason,
        "tool_calls": tool_calls,
        "files_accessed": files_accessed,
    }))
}
