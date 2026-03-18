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

use async_trait::async_trait;
use rara_kernel::{
    session::{SessionIndexRef, SessionKey},
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Input parameters for the update-session-title tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateSessionTitleParams {
    /// The session key to update.
    session_key: String,
    /// The new title (max 30 characters, match conversation language).
    title:       String,
}

/// Mita-exclusive tool: update a session's title.
#[derive(ToolDef)]
#[tool(
    name = "update-session-title",
    description = "Update the title of a session. Use this to set a concise, descriptive title \
                   for sessions that are missing one. The title should be max 30 characters and \
                   match the language of the conversation."
)]
pub struct UpdateSessionTitleTool {
    session_index: SessionIndexRef,
}

impl UpdateSessionTitleTool {
    pub fn new(session_index: SessionIndexRef) -> Self { Self { session_index } }
}

#[async_trait]
impl ToolExecute for UpdateSessionTitleTool {
    type Output = Value;
    type Params = UpdateSessionTitleParams;

    async fn run(
        &self,
        params: UpdateSessionTitleParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        if params.title.trim().is_empty() {
            anyhow::bail!("title must not be empty");
        }

        let session_key = SessionKey::try_from_raw(&params.session_key)
            .map_err(|e| anyhow::anyhow!("invalid session key: {e}"))?;

        let mut entry = self
            .session_index
            .get_session(&session_key)
            .await
            .map_err(|e| anyhow::anyhow!("failed to get session: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", params.session_key))?;

        // Guard: never overwrite an existing title.
        if entry.title.as_ref().is_some_and(|t| !t.is_empty()) {
            return Ok(json!({
                "status": "skipped",
                "reason": "session already has a title",
                "session_key": params.session_key,
                "existing_title": entry.title,
            }));
        }

        entry.title = Some(params.title.clone());
        entry.updated_at = chrono::Utc::now();

        self.session_index
            .update_session(&entry)
            .await
            .map_err(|e| anyhow::anyhow!("failed to update session: {e}"))?;

        Ok(json!({
            "status": "ok",
            "session_key": params.session_key,
            "title": params.title,
        }))
    }
}
