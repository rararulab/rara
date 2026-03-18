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

//! Tool for tracing a message's execution history by `rara_message_id`.

use async_trait::async_trait;
use rara_kernel::{
    memory::TapeService,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DebugTraceParams {
    /// The rara_message_id to trace.
    message_id: String,
    /// Maximum number of entries to return (default 50).
    limit:      Option<u64>,
}

/// Searches the session tape for entries associated with a `rara_message_id`.
#[derive(ToolDef)]
#[tool(
    name = "debug_trace",
    description = "Look up all tape entries related to a specific rara_message_id in the current \
                   session. Returns the full execution trace (messages, tool calls, results) with \
                   metadata. Only use when the user asks to debug or trace a specific message."
)]
pub struct DebugTraceTool {
    tape_service: TapeService,
}
impl DebugTraceTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl ToolExecute for DebugTraceTool {
    type Output = Value;
    type Params = DebugTraceParams;

    async fn run(&self, params: DebugTraceParams, ctx: &ToolContext) -> anyhow::Result<Value> {
        let limit = params.limit.unwrap_or(50) as usize;
        let tape_name = ctx.session_key.to_string();
        let entries = self
            .tape_service
            .search(&tape_name, &params.message_id, limit, false)
            .await
            .map_err(|e| anyhow::anyhow!("tape search failed: {e}"))?;
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|entry| {
                entry.metadata.as_ref().map_or(false, |m| {
                    m.get("rara_message_id")
                        .and_then(|v| v.as_str())
                        .map_or(false, |id| id == params.message_id)
                })
            })
            .collect();
        let formatted: Vec<Value> = entries.iter().map(|entry| {
            let mut obj = json!({"id": entry.id, "kind": entry.kind.to_string(), "payload": entry.payload, "timestamp": entry.timestamp.to_string()});
            if let Some(ref meta) = entry.metadata { obj["metadata"] = meta.clone(); }
            obj
        }).collect();
        Ok(
            json!({"tape_name": tape_name, "message_id": params.message_id, "match_count": formatted.len(), "entries": formatted}),
        )
    }
}
