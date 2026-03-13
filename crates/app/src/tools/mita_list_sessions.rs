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

//! Mita-exclusive tool: list live sessions from the kernel process table.

use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use rara_kernel::{
    handle::KernelHandle,
    tool::{AgentTool, ToolContext, ToolOutput},
};
use serde_json::{Value, json};
use tokio::sync::RwLock;

/// Mita tool that lists all live sessions from the kernel's process table.
///
/// Returns session keys, agent names, state, metrics, and timestamps so Mita
/// can decide which sessions to inspect further with `read_tape`.
pub struct ListSessionsTool {
    kernel_handle: Arc<RwLock<Option<KernelHandle>>>,
}

impl ListSessionsTool {
    pub fn new() -> Self {
        Self {
            kernel_handle: Arc::new(RwLock::new(None)),
        }
    }

    /// Return a cloneable reference to the inner handle slot, so the caller
    /// can inject the `KernelHandle` after kernel startup.
    pub fn handle_ref(&self) -> Arc<RwLock<Option<KernelHandle>>> {
        Arc::clone(&self.kernel_handle)
    }
}

#[async_trait]
impl AgentTool for ListSessionsTool {
    fn name(&self) -> &str { "list-sessions" }

    fn description(&self) -> &str {
        "List all live sessions currently running in the kernel process table. Returns session \
         key, agent name, state, metrics, and timestamps. Use this to discover sessions worth \
         inspecting."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of sessions to return (default 50)",
                    "default": 50
                },
                "updated_since": {
                    "type": "string",
                    "description": "ISO 8601 timestamp — only return sessions with activity after this time (e.g. '2025-01-01T00:00:00Z')"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> anyhow::Result<ToolOutput> {
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let updated_since: Option<Timestamp> = params
            .get("updated_since")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());

        let handle = self.kernel_handle.read().await;
        let handle = handle
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("kernel handle not yet available"))?;

        let mut sessions = handle.list_processes().await;

        // Filter by last_activity if requested.
        if let Some(since) = updated_since {
            sessions.retain(|s| s.last_activity.map_or(false, |ts| ts > since));
        }

        // Truncate to limit.
        sessions.truncate(limit);

        let entries: Vec<Value> = sessions
            .iter()
            .map(|s| {
                json!({
                    "session_key": s.session_key.to_string(),
                    "agent_name": s.manifest_name,
                    "state": s.state.to_string(),
                    "parent_id": s.parent_id.map(|k| k.to_string()),
                    "children": s.children.iter().map(|k| k.to_string()).collect::<Vec<_>>(),
                    "created_at": s.created_at.to_string(),
                    "finished_at": s.finished_at.map(|t| t.to_string()),
                    "uptime_ms": s.uptime_ms,
                    "messages_received": s.messages_received,
                    "llm_calls": s.llm_calls,
                    "tool_calls": s.tool_calls,
                    "tokens_consumed": s.tokens_consumed,
                    "last_activity": s.last_activity.map(|t| t.to_string()),
                })
            })
            .collect();

        Ok(json!({
            "total": entries.len(),
            "sessions": entries,
        })
        .into())
    }
}
