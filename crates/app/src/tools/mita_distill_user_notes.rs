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

//! Mita-exclusive tool for distilling accumulated user notes into a compact
//! anchor summary.

use async_trait::async_trait;
use rara_kernel::{
    memory::{HandoffState, TapeService},
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::notify::push_notification;

/// Input parameters for the distill-user-notes tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DistillUserNotesParams {
    /// The user identifier whose notes to distill.
    user_id: String,
    /// The distilled summary of all accumulated knowledge about this user.
    summary: String,
}

/// Typed result returned by the distill-user-notes tool.
#[derive(Debug, Clone, Serialize)]
pub struct DistillUserNotesResult {
    /// Operation status.
    pub status:  String,
    /// User identifier.
    pub user_id: String,
    /// Human-readable confirmation message.
    pub message: String,
}

/// Mita-exclusive tool: distill accumulated user notes into a compact anchor.
#[derive(ToolDef)]
#[tool(
    name = "distill-user-notes",
    description = "Distill accumulated user notes into a compact summary anchor using the \
                   structured profile template below. Use this when a user's tape has accumulated \
                   many notes that should be condensed.\n\nThe summary MUST follow this template \
                   (omit empty sections):\n\n## Identity\nName, role, background, timezone\n\n## \
                   Communication Style\nLanguage preference, verbosity, tone, interaction \
                   patterns\n\n## Expertise & Interests\nTechnical domains, skill levels, current \
                   learning areas\n\n## Key Facts\nProjects, relationships, important \
                   context\n\n## Active Context\nCurrent goals, pending tasks, recent focus \
                   areas\n\nRules:\n- Preserve all valid information from the previous anchor \
                   summary\n- When information conflicts, prefer the most recent note and note \
                   the change\n- Remove completed TODOs and outdated information\n- Omit sections \
                   with no information"
)]
pub struct DistillUserNotesTool {
    tape_service: TapeService,
}

impl DistillUserNotesTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl ToolExecute for DistillUserNotesTool {
    type Output = DistillUserNotesResult;
    type Params = DistillUserNotesParams;

    async fn run(
        &self,
        params: DistillUserNotesParams,
        context: &ToolContext,
    ) -> anyhow::Result<DistillUserNotesResult> {
        if params.user_id.trim().is_empty() {
            anyhow::bail!("user_id must not be empty");
        }
        if params.summary.trim().is_empty() {
            anyhow::bail!("summary must not be empty");
        }

        let user_tape = rara_kernel::memory::user_tape_name(&params.user_id);

        let handoff_state = HandoffState {
            summary: Some(params.summary),
            owner: Some("mita".into()),
            ..Default::default()
        };

        self.tape_service
            .handoff(&user_tape, "distill", handoff_state)
            .await
            .map_err(|e| anyhow::anyhow!("failed to write distillation anchor: {e}"))?;

        push_notification(
            context,
            format!(
                "\u{1f5dc}\u{fe0f} User notes distilled for {}",
                params.user_id
            ),
        );

        Ok(DistillUserNotesResult {
            status:  "ok".to_owned(),
            user_id: params.user_id.clone(),
            message: format!(
                "User notes distilled for '{}'. New anchor created.",
                params.user_id
            ),
        })
    }
}
