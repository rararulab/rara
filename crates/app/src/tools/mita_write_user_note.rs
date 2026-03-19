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
//! explicit `user_id` parameter.  It is intended only for Mita -- the
//! system-level background agent -- which needs to write cross-session
//! observations into arbitrary user tapes during heartbeat analysis.

use async_trait::async_trait;
use rara_kernel::{
    memory::TapeService,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{notify::push_notification, user_note::NOTE_CATEGORIES};

/// Input parameters for the write-user-note tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteUserNoteParams {
    /// The user identifier whose tape to write to.
    user_id:  String,
    /// Category of the note.
    category: String,
    /// The note content to record.
    content:  String,
}

/// Typed result returned by the write-user-note tool.
#[derive(Debug, Clone, Serialize)]
pub struct WriteUserNoteResult {
    /// Operation status.
    pub status:   String,
    /// Unique identifier for the created note.
    pub note_id:  String,
    /// User identifier.
    pub user_id:  String,
    /// Note category.
    pub category: String,
    /// Human-readable confirmation message.
    pub message:  String,
}

/// Mita-exclusive tool: write a structured note into any user's tape.
///
/// Mita is a system-level agent that observes all sessions.  During heartbeat
/// cycles it may discover cross-session patterns or insights that should be
/// persisted in a specific user's tape for future context injection.
#[derive(ToolDef)]
#[tool(
    name = "write-user-note",
    description = "Write a structured note into a specific user's tape. This is a system-level \
                   tool for recording cross-session observations about a user. Requires explicit \
                   user_id.\n\nCategories:\n- preference: User preferences (language, style, \
                   tools they like)\n- fact: Important facts about the user (name, role, \
                   projects)\n- todo: Tasks or reminders for the user\n- general: Anything else \
                   worth remembering",
    bypass_interceptor
)]
pub struct MitaWriteUserNoteTool {
    tape_service: TapeService,
}

impl MitaWriteUserNoteTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl ToolExecute for MitaWriteUserNoteTool {
    type Output = WriteUserNoteResult;
    type Params = WriteUserNoteParams;

    async fn run(
        &self,
        params: WriteUserNoteParams,
        context: &ToolContext,
    ) -> anyhow::Result<WriteUserNoteResult> {
        if params.user_id.trim().is_empty() {
            anyhow::bail!("user_id must not be empty");
        }

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
            .append_user_note(&params.user_id, &params.category, &params.content)
            .await
            .map_err(|e| anyhow::anyhow!("failed to write user note: {e}"))?;

        push_notification(
            context,
            format!(
                "\u{1f4dd} User note [{}] for {}: {}",
                params.category, params.user_id, params.content
            ),
        );

        Ok(WriteUserNoteResult {
            status:   "ok".to_owned(),
            note_id:  entry.id.to_string(),
            user_id:  params.user_id.clone(),
            category: params.category,
            message:  format!(
                "Note recorded for user '{}' under category '{}'.",
                params.user_id, "category"
            ),
        })
    }
}
