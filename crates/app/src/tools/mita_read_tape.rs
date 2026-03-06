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
    memory::TapeService,
    tool::{AgentTool, ToolContext},
};
use serde_json::{Value, json};

/// Mita tool that reads tape entries from a specified session.
///
/// Supports a `recent_n` parameter to limit results to the most recent
/// N entries, avoiding overwhelming Mita's context with long histories.
pub struct ReadTapeTool {
    tape_service: TapeService,
}

impl ReadTapeTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl AgentTool for ReadTapeTool {
    fn name(&self) -> &str { "read_tape" }

    fn description(&self) -> &str {
        "Read tape entries from a specific session. Returns message history including user \
         messages, assistant responses, and tool calls. Use `recent_n` to limit to the most recent \
         entries."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "The session key to read tape from"
                },
                "recent_n": {
                    "type": "integer",
                    "description": "Only return the most recent N entries (default: all entries from last anchor)"
                }
            },
            "required": ["session_id"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> anyhow::Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: session_id"))?;

        let recent_n = params.get("recent_n").and_then(|v| v.as_u64());

        // Read entries from the last anchor onward (the current conversation context).
        let entries = self
            .tape_service
            .from_last_anchor(session_id, None)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read tape for session '{session_id}': {e}"))?;

        // Apply recent_n limit if specified.
        let entries = if let Some(n) = recent_n {
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
                json!({
                    "id": entry.id,
                    "kind": entry.kind.to_string(),
                    "payload": entry.payload,
                    "timestamp": entry.timestamp.to_string(),
                })
            })
            .collect();

        Ok(json!({
            "session_id": session_id,
            "entry_count": formatted.len(),
            "entries": formatted,
        }))
    }
}
