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

//! Tool for tracing a turn's execution history by `rara_turn_id`.

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
    /// The `rara_turn_id` of the turn to trace. Accepts the legacy
    /// `rara_message_id` key on read so prompt-cached tool calls
    /// referencing the old parameter name still parse — see issue #1978.
    #[serde(alias = "rara_message_id", alias = "message_id")]
    pub rara_turn_id: String,
    /// Maximum number of entries to return (default 50).
    pub limit:        Option<u64>,
}

/// Searches the session tape for entries associated with a `rara_turn_id`.
#[derive(ToolDef)]
#[tool(
    name = "debug_trace",
    description = "Look up all tape entries related to a specific rara_turn_id in the current \
                   session. Returns the full execution trace (messages, tool calls, results) with \
                   metadata. Only use when the user asks to debug or trace a specific turn.",
    tier = "deferred",
    read_only,
    concurrency_safe
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
            .search(&tape_name, &params.rara_turn_id, limit, false)
            .await
            .map_err(|e| anyhow::anyhow!("tape search failed: {e}"))?;
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|entry| {
                entry.metadata.as_ref().is_some_and(|m| {
                    // Honor the legacy `rara_message_id` key on tape entries
                    // written before issue #1978 alongside the new key.
                    m.get("rara_turn_id")
                        .or_else(|| m.get("rara_message_id"))
                        .and_then(|v| v.as_str())
                        .is_some_and(|id| id == params.rara_turn_id)
                })
            })
            .collect();
        let formatted: Vec<Value> = entries.iter().map(|entry| {
            let mut obj = json!({"id": entry.id, "kind": entry.kind.to_string(), "payload": entry.payload, "timestamp": entry.timestamp.to_string()});
            if let Some(ref meta) = entry.metadata { obj["metadata"] = meta.clone(); }
            obj
        }).collect();
        Ok(
            json!({"tape_name": tape_name, "rara_turn_id": params.rara_turn_id, "match_count": formatted.len(), "entries": formatted}),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::DebugTraceParams;

    /// Issue #1978 back-compat: prompt caches generated before the rename
    /// still reference the legacy `rara_message_id` parameter name. The
    /// tool's input deserializer must accept that key and parse it into
    /// the renamed `rara_turn_id` field, so an old cached tool call does
    /// not produce a hard schema-mismatch failure for the LLM.
    #[test]
    fn accepts_legacy_rara_message_id_param() {
        let input = json!({"rara_message_id": "trace-1"});
        let parsed: DebugTraceParams =
            serde_json::from_value(input).expect("legacy key must deserialize");
        assert_eq!(parsed.rara_turn_id, "trace-1");
        assert_eq!(parsed.limit, None);
    }
}
