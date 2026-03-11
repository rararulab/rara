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

//! Tape memory tool — exposes the raw tape subsystem to the LLM agent.
//!
//! The tape is an append-only JSONL timeline that records every conversation
//! event for a session.  This tool lets the agent inspect, search, and manage
//! its own tape directly, giving it full visibility into the underlying memory
//! mechanism so it can make informed decisions about context management.

use anyhow::Context;
use async_trait::async_trait;
use serde::Deserialize;

use crate::memory::{HandoffState, TapEntryKind, TapeService};

/// LLM-callable tool that exposes raw tape memory primitives.
///
/// Registered per-session alongside `SyscallTool`.  The tool name is `"tape"`
/// and its description teaches the LLM how the tape memory model works so it
/// can reason about its own context window, search past conversations, and
/// create anchors to manage memory.
pub(crate) struct TapeTool {
    tape_service: TapeService,
    tape_name:    String,
}

impl TapeTool {
    pub fn new(tape_service: TapeService, tape_name: String) -> Self {
        Self {
            tape_service,
            tape_name,
        }
    }

    async fn exec_info(&self) -> anyhow::Result<serde_json::Value> {
        let info = self
            .tape_service
            .info(&self.tape_name)
            .await
            .context("tape-info")?;
        Ok(serde_json::json!({
            "tape_name": info.name,
            "total_entries": info.entries,
            "anchor_count": info.anchors,
            "last_anchor": info.last_anchor,
            "entries_since_last_anchor": info.entries_since_last_anchor,
            "last_token_usage": info.last_token_usage,
            "estimated_context_tokens": info.estimated_context_tokens,
        }))
    }

    async fn exec_search(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<serde_json::Value> {
        let results = self
            .tape_service
            .search(&self.tape_name, query, limit.unwrap_or(10), false)
            .await
            .context("tape_search")?;
        let count = results.len();
        Ok(serde_json::json!({ "results": results, "count": count }))
    }

    async fn exec_anchor(
        &self,
        name: &str,
        summary: Option<&str>,
        next_steps: Option<&str>,
        state: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let handoff_state = HandoffState {
            summary: summary.map(|s| s.to_owned()),
            next_steps: next_steps.map(|s| s.to_owned()),
            owner: Some("agent".into()),
            extra: state,
            ..Default::default()
        };

        let entries = self
            .tape_service
            .handoff(&self.tape_name, name, handoff_state)
            .await
            .context("tape_anchor")?;
        Ok(serde_json::json!({
            "anchor_name": name,
            "entries_after_anchor": entries.len(),
        }))
    }

    async fn exec_anchors(&self, limit: Option<usize>) -> anyhow::Result<serde_json::Value> {
        let anchors = self
            .tape_service
            .anchors(&self.tape_name, limit.unwrap_or(10))
            .await
            .context("tape_anchors")?;
        let count = anchors.len();
        Ok(serde_json::json!({ "anchors": anchors, "count": count }))
    }

    async fn exec_entries(
        &self,
        after_anchor: Option<&str>,
        kinds: Option<Vec<String>>,
    ) -> anyhow::Result<serde_json::Value> {
        let kind_filters: Option<Vec<TapEntryKind>> = kinds.map(|ks| {
            ks.iter()
                .filter_map(|k| k.parse::<TapEntryKind>().ok())
                .collect()
        });
        let kind_refs = kind_filters.as_deref();

        let entries = if let Some(anchor) = after_anchor {
            self.tape_service
                .after_anchor(&self.tape_name, anchor, kind_refs)
                .await
        } else {
            self.tape_service
                .from_last_anchor(&self.tape_name, kind_refs)
                .await
        }
        .context("tape_entries")?;

        let count = entries.len();
        Ok(serde_json::json!({ "entries": entries, "count": count }))
    }

    async fn exec_between_anchors(
        &self,
        start: &str,
        end: &str,
        kinds: Option<Vec<String>>,
    ) -> anyhow::Result<serde_json::Value> {
        let kind_filters: Option<Vec<TapEntryKind>> = kinds.map(|ks| {
            ks.iter()
                .filter_map(|k| k.parse::<TapEntryKind>().ok())
                .collect()
        });
        let kind_refs = kind_filters.as_deref();

        let entries = self
            .tape_service
            .between_anchors(&self.tape_name, start, end, kind_refs)
            .await
            .context("tape_between_anchors")?;

        let count = entries.len();
        Ok(serde_json::json!({ "entries": entries, "count": count }))
    }
}

// ============================================================================
// Parameter types
// ============================================================================

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum TapeParams {
    Info,
    Search {
        query: String,
        #[serde(default)]
        limit: Option<usize>,
    },
    Anchor {
        name:       String,
        #[serde(default)]
        summary:    Option<String>,
        #[serde(default)]
        next_steps: Option<String>,
        #[serde(default)]
        state:      Option<serde_json::Value>,
    },
    Anchors {
        #[serde(default)]
        limit: Option<usize>,
    },
    Entries {
        #[serde(default)]
        after_anchor: Option<String>,
        #[serde(default)]
        kinds:        Option<Vec<String>>,
    },
    BetweenAnchors {
        start: String,
        end:   String,
        #[serde(default)]
        kinds: Option<Vec<String>>,
    },
}

// ============================================================================
// AgentTool impl
// ============================================================================

#[async_trait]
impl crate::tool::AgentTool for TapeTool {
    fn name(&self) -> &str { "tape" }

    fn description(&self) -> &str {
        "Your memory is a tape — an append-only timeline that records every message, tool call, \
         and tool result in this session as sequential entries.\n\n## How your context window \
         works\n\nYou do NOT see the entire tape. Your LLM context window only contains entries \
         since the last anchor (checkpoint). Everything before that anchor still exists on the \
         tape and is fully searchable, but it is not in your current context.\n\n## Anchors\n\nAn \
         anchor is a named checkpoint you insert into the tape. When you create an anchor, your \
         future context window starts from that point. Use anchors when:\n- A topic or task is \
         complete and you want to free up context space\n- You want to mark a logical boundary in \
         the conversation\n- Your context is getting large and you need to trim it\n\nWhen \
         creating an anchor, always provide a `summary` of the conversation so far and \
         `next_steps` if applicable. These are stored in the anchor state so you retain key \
         context even after the older entries leave your context window.\n\nData before an anchor \
         is NOT deleted — `search` can still find it across all anchors.\n\n## Entry kinds\n\nEach \
         tape entry has a kind: `message` (user/assistant chat), `tool_call` (your tool \
         invocations), `tool_result` (tool outputs), `event` (lifecycle telemetry), `system` \
         (system prompts), `anchor` (checkpoints).\n\n## Actions\n\n- `info` — inspect tape state \
         (total entries, anchor count, context window size)\n- `search` — find past conversations \
         by text, works across all anchors including forgotten context\n- `anchor` — create a \
         checkpoint to trim your context window\n- `anchors` — list recent checkpoints to \
         understand your memory structure\n- `entries` — read raw tape entries in your current \
         context window or after a specific anchor
         - `between_anchors` — read entries between two named anchors to inspect a specific past \
         context window"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["info", "search", "anchor", "anchors", "entries", "between_anchors"],
                    "description": "The tape operation to perform."
                },
                "query": {
                    "type": "string",
                    "description": "[search] Text to search for in past conversations. Uses ranked Unicode-aware matching across message payloads and metadata."
                },
                "name": {
                    "type": "string",
                    "description": "[anchor] Name for the checkpoint (e.g. 'topic/weather-done', 'handoff')."
                },
                "summary": {
                    "type": "string",
                    "description": "[anchor] Summary of the conversation up to this point. Always provide this so you retain context after the anchor trims your window."
                },
                "next_steps": {
                    "type": "string",
                    "description": "[anchor] What should happen next. Helps you pick up where you left off after context is trimmed."
                },
                "state": {
                    "description": "[anchor] Optional additional JSON state to attach to the checkpoint."
                },
                "limit": {
                    "type": "integer",
                    "description": "[search, anchors] Maximum number of results to return. Default: 10."
                },
                "after_anchor": {
                    "type": "string",
                    "description": "[entries] Read entries after this named anchor instead of the most recent one."
                },
                "start": {
                    "type": "string",
                    "description": "[between_anchors] Starting anchor name."
                },
                "end": {
                    "type": "string",
                    "description": "[between_anchors] Ending anchor name."
                },
                "kinds": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["message", "tool_call", "tool_result", "event", "system", "anchor"]
                    },
                    "description": "[entries, between_anchors] Filter entries by kind."
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &crate::tool::ToolContext,
    ) -> anyhow::Result<crate::tool::ToolOutput> {
        let action: TapeParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid tape tool params: {e}"))?;

        let json = match action {
            TapeParams::Info => self.exec_info().await,
            TapeParams::Search { query, limit } => self.exec_search(&query, limit).await,
            TapeParams::Anchor {
                name,
                summary,
                next_steps,
                state,
            } => {
                self.exec_anchor(&name, summary.as_deref(), next_steps.as_deref(), state)
                    .await
            }
            TapeParams::Anchors { limit } => self.exec_anchors(limit).await,
            TapeParams::Entries {
                after_anchor,
                kinds,
            } => self.exec_entries(after_anchor.as_deref(), kinds).await,
            TapeParams::BetweenAnchors { start, end, kinds } => {
                self.exec_between_anchors(&start, &end, kinds).await
            }
        }?;

        Ok(json.into())
    }
}
