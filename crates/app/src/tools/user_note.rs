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

//! Tool for recording structured notes about a user into their persistent user
//! tape.
//!
//! The LLM invokes this tool to persist facts, preferences, and TODOs that
//! should be recalled across future sessions with the same user.

use async_trait::async_trait;
use rara_kernel::{memory::TapeService, tool::AgentTool};
use serde_json::json;

/// Single source of truth for valid user-note categories.
///
/// Both the JSON schema (sent to the LLM) and the runtime validation match
/// derive from this array, eliminating the risk of silent divergence.
pub const NOTE_CATEGORIES: &[&str] = &["preference", "fact", "todo", "general"];

/// Layer 2 service tool: persist a structured note in the user's tape.
pub struct UserNoteTool {
    tape_service: TapeService,
}

impl UserNoteTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl AgentTool for UserNoteTool {
    fn name(&self) -> &str { "user-note" }

    fn description(&self) -> &str {
        "Record a note about the current user for future reference. The user is automatically \
         identified from the session context — do NOT pass a user_id. Notes persist across \
         sessions and are automatically loaded into context for future conversations with this \
         user.\n\nCategories:\n- preference: User preferences (language, style, tools they \
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
            "required": ["category", "content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let user_id = context
            .user_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no authenticated user in session context"))?;
        let category = params
            .get("category")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: category"))?;
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        // Validate category against the single source of truth.
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

        Ok(json!({
            "status": "ok",
            "note_id": entry.id,
            "user_id": user_id,
            "category": category,
            "message": format!("Note recorded for user '{user_id}' under category '{category}'.")
        }))
    }
}
