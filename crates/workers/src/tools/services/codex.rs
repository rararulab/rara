// Copyright 2025 Crrow
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

//! Layer 2 service tools for dispatching CLI coding agents (Claude Code /
//! Codex).
//!
//! Now backed by [`rara_coding_task::service::CodingTaskService`] for
//! persistent PG storage.

use async_trait::async_trait;
use rara_coding_task::service::CodingTaskService;
use rara_coding_task::types::AgentType;
use serde_json::json;
use tool_core::AgentTool;

// ---------------------------------------------------------------------------
// CodexRunTool
// ---------------------------------------------------------------------------

/// Dispatch a coding task to a CLI agent (Claude Code or Codex).
pub struct CodexRunTool {
    service: CodingTaskService,
}

impl CodexRunTool {
    pub fn new(service: CodingTaskService) -> Self {
        Self { service }
    }
}

#[async_trait]
impl AgentTool for CodexRunTool {
    fn name(&self) -> &str { "codex_run" }

    fn description(&self) -> &str {
        "Dispatch a coding task to a CLI agent (Claude Code or Codex). Creates a git worktree for \
         isolation and runs the agent in a tmux session. Returns immediately with a task ID. Sends \
         a Telegram notification when the task completes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The coding task prompt to send to the agent"
                },
                "agent": {
                    "type": "string",
                    "enum": ["claude", "codex"],
                    "description": "Which CLI agent to use (default: claude)"
                },
                "repo_url": {
                    "type": "string",
                    "description": "Git repository URL (uses default if omitted)"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: prompt"))?;

        let agent_type = match params.get("agent").and_then(|v| v.as_str()) {
            Some("codex") => AgentType::Codex,
            _ => AgentType::Claude,
        };

        let repo_url = params.get("repo_url").and_then(|v| v.as_str());

        let task = self
            .service
            .dispatch(repo_url, prompt, agent_type, None)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        Ok(json!({
            "task_id": task.id.to_string(),
            "branch": task.branch,
            "tmux_session": task.tmux_session,
            "status": "dispatched",
        }))
    }
}

// ---------------------------------------------------------------------------
// CodexStatusTool
// ---------------------------------------------------------------------------

/// Check status and output of a dispatched coding task.
pub struct CodexStatusTool {
    service: CodingTaskService,
}

impl CodexStatusTool {
    pub fn new(service: CodingTaskService) -> Self { Self { service } }
}

#[async_trait]
impl AgentTool for CodexStatusTool {
    fn name(&self) -> &str { "codex_status" }

    fn description(&self) -> &str {
        "Check status and output of a dispatched coding task by task ID."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "UUID of the coding task to check"
                }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let task_id_str = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: task_id"))?;
        let task_id: uuid::Uuid = task_id_str.parse()?;

        match self.service.get(task_id).await {
            Ok(t) => Ok(json!({
                "id": t.id.to_string(),
                "agent_type": t.agent_type.to_string(),
                "branch": t.branch,
                "status": t.status.to_string(),
                "prompt": t.prompt,
                "output": t.output,
                "exit_code": t.exit_code,
                "pr_url": t.pr_url,
                "error": t.error,
            })),
            Err(e) => Ok(json!({ "error": e.to_string() })),
        }
    }
}

// ---------------------------------------------------------------------------
// CodexListTool
// ---------------------------------------------------------------------------

/// List all dispatched coding tasks and their status.
pub struct CodexListTool {
    service: CodingTaskService,
}

impl CodexListTool {
    pub fn new(service: CodingTaskService) -> Self { Self { service } }
}

#[async_trait]
impl AgentTool for CodexListTool {
    fn name(&self) -> &str { "codex_list" }

    fn description(&self) -> &str { "List all dispatched coding tasks and their status." }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let tasks = self.service.list().await.unwrap_or_default();
        let items: Vec<serde_json::Value> = tasks
            .iter()
            .map(|t| {
                json!({
                    "id": t.id.to_string(),
                    "agent_type": t.agent_type.to_string(),
                    "branch": t.branch,
                    "status": t.status.to_string(),
                    "prompt": truncate(&t.prompt, 100),
                    "pr_url": t.pr_url,
                })
            })
            .collect();
        Ok(json!({
            "count": items.len(),
            "tasks": items,
        }))
    }
}

/// Truncate a string to at most `max` characters, appending "..." if
/// truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
