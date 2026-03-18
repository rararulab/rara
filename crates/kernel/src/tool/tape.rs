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

use std::sync::Arc;

use anyhow::Context;
use rara_tool_macro::ToolDef;
use serde::Deserialize;

use crate::{
    memory::{HandoffState, TapEntryKind, TapeService},
    session::{SessionEntry, SessionIndex, SessionKey},
};

/// LLM-callable tool that exposes raw tape memory primitives.
///
/// Registered per-session alongside `SyscallTool`.  The tool name is `"tape"`
/// and its description teaches the LLM how the tape memory model works so it
/// can reason about its own context window, search past conversations, and
/// create anchors to manage memory.
#[derive(ToolDef)]
#[tool(
    name = "tape",
    description = "Your memory is a tape — an append-only timeline that records every message, \
                   tool call, and tool result in this session as sequential entries.\n\n## How \
                   your context window works\n\nYou do NOT see the entire tape. Your LLM context \
                   window only contains entries since the last anchor (checkpoint). Everything \
                   before that anchor still exists on the tape and is fully searchable, but it is \
                   not in your current context.\n\n## When to anchor\n\nCreate anchors based on \
                   **topic/task transitions**, not context size:\n- The user switches to a \
                   different topic or task\n- A task or conversation thread reaches a natural \
                   conclusion\n- The user explicitly asks to move on\n\nUse `info` to check \
                   `estimated_context_tokens` as a health indicator — if context is growing large \
                   after recalling old data, that is a good time to anchor and \
                   consolidate.\n\nWhen creating an anchor, use a descriptive name (e.g. \
                   `topic/immich-setup`, `task/debug-lifetime`) and always provide a `summary` \
                   and `next_steps`. These are injected into your context after the anchor so you \
                   retain key information.\n\n## Recall: accessing past context\n\nWhen the user \
                   refers to something from a previous topic or earlier conversation:\n1. Use \
                   `anchors` to see your memory structure and find the relevant anchor\n2. Use \
                   `between_anchors` to load the full context of that topic segment\n3. Or use \
                   `search` to find specific information by keyword across the entire \
                   tape\n\nData before an anchor is NOT deleted — it is always available for \
                   recall. Your anchors are like bookmarks in a book: you read from the current \
                   bookmark forward, but you can always flip back.\n\n## Entry kinds\n\nEach tape \
                   entry has a kind: `message` (user/assistant chat), `tool_call` (your tool \
                   invocations), `tool_result` (tool outputs), `event` (lifecycle telemetry), \
                   `system` (system prompts), `anchor` (checkpoints).\n\n## Actions\n\n- `info` — \
                   inspect tape state (entries, anchors, `estimated_context_tokens`)\n- `search` \
                   — recall past information by keyword, works across all anchors\n- `anchor` — \
                   create a named checkpoint when transitioning topics\n- `anchors` — list \
                   checkpoints to find relevant past context\n- `entries` — read raw tape entries \
                   in your current context window or after a specific anchor\n- `between_anchors` \
                   — recall the full context of a specific past topic segment\n- `checkout` — \
                   fork from a named anchor, creating a new session with context up to that \
                   point\n- `checkout_root` — find and return to the root (original) session by \
                   walking the fork chain. Use this when you or the user want to go back to the \
                   main conversation after working in a forked session.\n\n### checkout in \
                   detail\n\n**When to use**: The user wants to return to a past topic's state \
                   and continue from there, or explore a different direction from a previous \
                   checkpoint. For example, the user says \"let's go back to where we were \
                   debugging that lifetime issue and try a different approach.\"\n\n**What \
                   happens**: A new session is created containing all tape entries up to and \
                   including the named anchor. The original session is untouched — nothing is \
                   lost or overwritten. Your context window in the new session starts from that \
                   anchor's state, so you can continue working as if you were back at that point \
                   in time.\n\n**checkout vs between_anchors**: `between_anchors` is read-only \
                   recall — you see past entries but stay in the current session and context. \
                   `checkout` forks a new session where you actually continue working from that \
                   earlier point. Use `between_anchors` when you just need to reference old \
                   information; use `checkout` when the user wants to resume or diverge from a \
                   past state. To return to the original session after checking out, use \
                   `checkout_root`.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub(crate) struct TapeTool {
    tape_service: TapeService,
    tape_name:    String,
    sessions:     Arc<dyn SessionIndex>,
}

impl TapeTool {
    pub fn new(
        tape_service: TapeService,
        tape_name: String,
        sessions: Arc<dyn SessionIndex>,
    ) -> Self {
        Self {
            tape_service,
            tape_name,
            sessions,
        }
    }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["info", "search", "anchor", "anchors", "entries", "between_anchors", "checkout", "checkout_root"],
                    "description": "The tape operation to perform."
                },
                "query": {
                    "type": "string",
                    "description": "[search] Text to search for in past conversations. Uses ranked Unicode-aware matching across message payloads and metadata."
                },
                "name": {
                    "type": "string",
                    "description": "[anchor, checkout] Name for the checkpoint or anchor to fork from."
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

    async fn exec(
        &self,
        params: serde_json::Value,
        _context: &super::ToolContext,
    ) -> anyhow::Result<super::ToolOutput> {
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
            TapeParams::Checkout { name } => self.exec_checkout(&name).await,
            TapeParams::CheckoutRoot => self.exec_checkout_root().await,
        }?;

        Ok(json.into())
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

    async fn exec_checkout(&self, anchor_name: &str) -> anyhow::Result<serde_json::Value> {
        use chrono::Utc;

        use crate::memory::set_fork_metadata;

        // 1. Create a new session with fork metadata.
        let new_key = SessionKey::new();
        let mut metadata = None;
        set_fork_metadata(&mut metadata, &self.tape_name, anchor_name);
        let now = Utc::now();
        let entry = SessionEntry {
            key: new_key.clone(),
            title: Some(format!("Fork from {anchor_name}")),
            model: None,
            system_prompt: None,
            message_count: 0,
            preview: None,
            metadata,
            created_at: now,
            updated_at: now,
        };

        self.sessions
            .create_session(&entry)
            .await
            .map_err(|e| anyhow::anyhow!("failed to create fork session: {e}"))?;

        // 2. Copy tape entries up to the anchor into the new tape.
        let new_tape = new_key.to_string();
        if let Err(e) = self
            .tape_service
            .checkout_anchor(&self.tape_name, anchor_name, &new_tape)
            .await
        {
            // Rollback session on tape failure.
            let _ = self.sessions.delete_session(&new_key).await;
            return Err(anyhow::anyhow!("checkout failed: {e}"));
        }

        Ok(serde_json::json!({
            "status": "checked_out",
            "from_anchor": anchor_name,
            "new_session": new_tape,
            "message": format!(
                "Forked from anchor '{}'. New session: {}. Context has been reset to the anchor point.",
                anchor_name, new_tape
            )
        }))
    }

    async fn exec_checkout_root(&self) -> anyhow::Result<serde_json::Value> {
        let root = self
            .tape_service
            .find_root_session(&self.tape_name, self.sessions.as_ref())
            .await
            .context("checkout_root")?;

        if root == self.tape_name {
            return Ok(serde_json::json!({
                "status": "already_at_root",
                "session": root,
                "message": "This session is already the root — there is no parent to return to."
            }));
        }

        Ok(serde_json::json!({
            "status": "root_found",
            "root_session": root,
            "current_session": self.tape_name,
            "message": format!(
                "Root session is {}. Use this session ID to navigate back to the original conversation.",
                root
            )
        }))
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
    Checkout {
        /// Anchor name to fork from.
        name: String,
    },
    CheckoutRoot,
}
