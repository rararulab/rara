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

//! Mita-exclusive tool: read tape entries from a specific session.

use async_trait::async_trait;
use rara_kernel::{
    memory::{TapeService, user_tape_name},
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Input parameters for the read-tape tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadTapeParams {
    /// The session key to read tape from.
    session_id: Option<String>,
    /// Read the user's personal tape. Mutually exclusive with session_id.
    user_id:    Option<String>,
    /// Only return the most recent N entries (default: all from last anchor).
    recent_n:   Option<u64>,
}

/// Mita tool that reads tape entries from a specified session.
///
/// Supports a `recent_n` parameter to limit results to the most recent
/// N entries, avoiding overwhelming Mita's context with long histories.
#[derive(ToolDef)]
#[tool(
    name = "read-tape",
    description = "Read tape entries from a session or user tape. Returns message history \
                   including user messages, assistant responses, and tool calls. Use `recent_n` \
                   to limit to the most recent entries. Provide either `session_id` or `user_id`, \
                   not both."
)]
pub struct ReadTapeTool {
    tape_service: TapeService,
}

impl ReadTapeTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl ToolExecute for ReadTapeTool {
    type Output = Value;
    type Params = ReadTapeParams;

    async fn run(&self, params: ReadTapeParams, _ctx: &ToolContext) -> anyhow::Result<Value> {
        let tape_name = match (params.session_id.as_deref(), params.user_id.as_deref()) {
            (Some(_), Some(_)) => {
                anyhow::bail!("session_id and user_id are mutually exclusive");
            }
            (Some(sid), None) => sid.to_string(),
            (None, Some(uid)) => user_tape_name(uid),
            (None, None) => {
                anyhow::bail!("either session_id or user_id is required");
            }
        };

        // Read entries from the last anchor onward (the current conversation context).
        let entries = self
            .tape_service
            .from_last_anchor(&tape_name, None)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read tape '{tape_name}': {e}"))?;

        // Apply recent_n limit if specified.
        let entries = if let Some(n) = params.recent_n {
            let n = n as usize;
            if entries.len() > n {
                entries[entries.len() - n..].to_vec()
            } else {
                entries
            }
        } else {
            entries
        };

        let formatted: Vec<Value> = entries
            .iter()
            .map(|entry| {
                let mut obj = json!({
                    "id": entry.id,
                    "kind": entry.kind.to_string(),
                    "payload": entry.payload,
                    "timestamp": entry.timestamp.to_string(),
                });
                if let Some(ref meta) = entry.metadata {
                    obj["metadata"] = meta.clone();
                }
                obj
            })
            .collect();

        Ok(json!({
            "tape_name": tape_name,
            "entry_count": formatted.len(),
            "entries": formatted,
        }))
    }
}
