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

use async_trait::async_trait;
use rara_kernel::{
    memory::TapeService,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const NOTE_CATEGORIES: &[&str] = &["preference", "fact", "todo", "general"];

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UserNoteParams {
    /// Category of the note.
    category: String,
    /// The note content to record.
    content:  String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserNoteResult {
    pub status:   String,
    pub note_id:  String,
    pub user_id:  String,
    pub category: String,
    pub message:  String,
}

/// Layer 2 service tool: persist a structured note in the user's tape.
#[derive(ToolDef)]
#[tool(
    name = "user-note",
    description = "Record a note about the current user for future reference. The user is \
                   automatically identified from the session context \u{2014} do NOT pass a \
                   user_id. Notes persist across sessions and are automatically loaded into \
                   context for future conversations with this user.\n\nCategories:\n- preference: \
                   User preferences (language, style, tools they like)\n- fact: Important facts \
                   about the user (name, role, projects)\n- todo: Tasks or reminders for the \
                   user\n- general: Anything else worth remembering",
    tier = "deferred"
)]
pub struct UserNoteTool {
    tape_service: TapeService,
}
impl UserNoteTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl ToolExecute for UserNoteTool {
    type Output = UserNoteResult;
    type Params = UserNoteParams;

    async fn run(
        &self,
        params: UserNoteParams,
        context: &ToolContext,
    ) -> anyhow::Result<UserNoteResult> {
        let user_id = context.user_id.as_str();
        if !NOTE_CATEGORIES.contains(&params.category.as_str()) {
            anyhow::bail!(
                "invalid category '{}': must be one of {}",
                params.category,
                NOTE_CATEGORIES.join(", ")
            );
        }
        if params.content.trim().is_empty() {
            anyhow::bail!("content must not be empty");
        }
        let entry = self
            .tape_service
            .append_user_note(user_id, &params.category, &params.content)
            .await
            .map_err(|e| anyhow::anyhow!("failed to write user note: {e}"))?;
        Ok(UserNoteResult {
            status:   "ok".to_owned(),
            note_id:  entry.id.to_string(),
            user_id:  user_id.to_owned(),
            category: params.category,
            message:  format!("Note recorded for user '{user_id}'."),
        })
    }
}
