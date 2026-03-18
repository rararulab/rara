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

//! Mita-exclusive tool for updating a session's title.
//!
//! Used during heartbeat cycles to fill in missing titles for sessions
//! that were created without one (e.g. when the auto-title LLM call
//! failed or the session predates the auto-title feature).

use rara_kernel::{
    session::{SessionIndexRef, SessionKey},
    tool::{ToolContext, ToolOutput},
};
use rara_tool_macro::ToolDef;
use serde_json::json;

/// Mita-exclusive tool: update a session's title.
#[derive(ToolDef)]
#[tool(
    name = "update-session-title",
    description = "Update the title of a session. Use this to set a concise, descriptive title \
                   for sessions that are missing one. The title should be max 30 characters and \
                   match the language of the conversation.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct UpdateSessionTitleTool {
    session_index: SessionIndexRef,
}

impl UpdateSessionTitleTool {
    pub fn new(session_index: SessionIndexRef) -> Self { Self { session_index } }

    fn schema() -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["session_key", "title"],
            "properties": {
                "session_key": {
                    "type": "string",
                    "description": "The session key to update"
                },
                "title": {
                    "type": "string",
                    "description": "The new title (max 30 characters, match conversation language)"
                }
            }
        })
    }

    async fn exec(
        &self,
        params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let session_key_str = params
            .get("session_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: session_key"))?;
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: title"))?;

        if title.trim().is_empty() {
            anyhow::bail!("title must not be empty");
        }

        let session_key = SessionKey::try_from_raw(session_key_str)
            .map_err(|e| anyhow::anyhow!("invalid session key: {e}"))?;

        let mut entry = self
            .session_index
            .get_session(&session_key)
            .await
            .map_err(|e| anyhow::anyhow!("failed to get session: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("session not found: {session_key_str}"))?;

        // Guard: never overwrite an existing title.
        if entry.title.as_ref().is_some_and(|t| !t.is_empty()) {
            return Ok(json!({
                "status": "skipped",
                "reason": "session already has a title",
                "session_key": session_key_str,
                "existing_title": entry.title,
            })
            .into());
        }

        entry.title = Some(title.to_string());
        entry.updated_at = chrono::Utc::now();

        self.session_index
            .update_session(&entry)
            .await
            .map_err(|e| anyhow::anyhow!("failed to update session: {e}"))?;

        Ok(json!({
            "status": "ok",
            "session_key": session_key_str,
            "title": title,
        })
        .into())
    }
}
