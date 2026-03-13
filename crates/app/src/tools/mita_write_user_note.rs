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

//! Mita-exclusive tool for writing notes to any user's tape.
//!
//! Unlike [`super::user_note::UserNoteTool`] which derives the user identity
//! from `ToolContext` (enforcing per-session security), this tool accepts an
//! explicit `user_id` parameter.  It is intended only for Mita — the
//! system-level background agent — which needs to write cross-session
//! observations into arbitrary user tapes during heartbeat analysis.

use async_trait::async_trait;
use rara_kernel::{
    memory::TapeService,
    tool::{AgentTool, ToolContext, ToolOutput},
};
use serde_json::json;

use super::{notify::push_notification, user_note::NOTE_CATEGORIES};

/// Mita-exclusive tool: write a structured note into any user's tape.
///
/// Mita is a system-level agent that observes all sessions.  During heartbeat
/// cycles it may discover cross-session patterns or insights that should be
/// persisted in a specific user's tape for future context injection.
pub struct MitaWriteUserNoteTool {
    tape_service: TapeService,
}

impl MitaWriteUserNoteTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl AgentTool for MitaWriteUserNoteTool {
    fn name(&self) -> &str { "write-user-note" }

    fn description(&self) -> &str {
        "Write a structured note into a specific user's tape. This is a system-level tool for \
         recording cross-session observations about a user. Requires explicit \
         user_id.\n\nCategories:\n- preference: User preferences (language, style, tools they \
         like)\n- fact: Important facts about the user (name, role, projects)\n- todo: Tasks or \
         reminders for the user\n- general: Anything else worth remembering"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let categories: Vec<serde_json::Value> = NOTE_CATEGORIES
            .iter()
            .map(|c| serde_json::Value::String((*c).to_owned()))
            .collect();
        json!({
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "The user identifier whose tape to write to"
                },
                "category": {
                    "type": "string",
                    "enum": categories,
                    "description": "Category of the note"
                },
                "content": {
                    "type": "string",
                    "description": "The note content to record"
                }
            },
            "required": ["user_id", "category", "content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let user_id = params
            .get("user_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: user_id"))?;
        let category = params
            .get("category")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: category"))?;
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        if user_id.trim().is_empty() {
            anyhow::bail!("user_id must not be empty");
        }

        if !NOTE_CATEGORIES.contains(&category) {
            anyhow::bail!(
                "invalid category '{category}': must be one of {}",
                NOTE_CATEGORIES.join(", ")
            );
        }

        if content.trim().is_empty() {
            anyhow::bail!("content must not be empty");
        }

        let entry = self
            .tape_service
            .append_user_note(user_id, category, content)
            .await
            .map_err(|e| anyhow::anyhow!("failed to write user note: {e}"))?;

        push_notification(
            context,
            format!("📝 User note [{category}] for {user_id}: {content}"),
        );

        Ok(json!({
            "status": "ok",
            "note_id": entry.id,
            "user_id": user_id,
            "category": category,
            "message": format!("Note recorded for user '{user_id}' under category '{category}'.")
        })
        .into())
    }
}
