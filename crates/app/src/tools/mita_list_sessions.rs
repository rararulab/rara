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

//! Mita-exclusive tool: list all active sessions with metadata.

use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rara_kernel::{
    session::SessionIndex,
    tool::{AgentTool, ToolContext, ToolOutput},
};
use serde_json::{Value, json};

/// Mita tool that lists all active sessions with their metadata.
///
/// Returns session keys, titles, message counts, and timestamps so Mita
/// can decide which sessions to inspect further with `read_tape`.
pub struct ListSessionsTool {
    session_index: Arc<dyn SessionIndex>,
}

impl ListSessionsTool {
    pub fn new(session_index: Arc<dyn SessionIndex>) -> Self { Self { session_index } }
}

#[async_trait]
impl AgentTool for ListSessionsTool {
    fn name(&self) -> &str { "list-sessions" }

    fn description(&self) -> &str {
        "List all active sessions with metadata (key, title, message count, timestamps). Use this \
         to discover sessions worth inspecting."
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
                    "description": "ISO 8601 timestamp — only return sessions updated after this time (e.g. '2025-01-01T00:00:00Z')"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> anyhow::Result<ToolOutput> {
        let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
        let updated_since = params
            .get("updated_since")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::<Utc>::from_str(s).ok());

        let sessions = self
            .session_index
            .list_sessions(limit, 0)
            .await
            .map_err(|e| anyhow::anyhow!("failed to list sessions: {e}"))?;

        let sessions = if let Some(since) = updated_since {
            sessions.into_iter().filter(|s| s.updated_at > since).collect()
        } else {
            sessions
        };

        let entries: Vec<Value> = sessions
            .iter()
            .map(|s| {
                json!({
                    "key": s.key.to_string(),
                    "title": s.title,
                    "message_count": s.message_count,
                    "preview": s.preview,
                    "created_at": s.created_at.to_rfc3339(),
                    "updated_at": s.updated_at.to_rfc3339(),
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
