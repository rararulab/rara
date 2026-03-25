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

//! Mita-exclusive tool for writing skill draft files.
//!
//! Mita analyzes completed sessions during heartbeat cycles and writes
//! structured drafts to disk. Rara later reads these drafts and creates
//! full skills from them.

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::info;

use super::notify::push_notification;

/// Input parameters for the write-skill-draft tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteSkillDraftParams {
    /// The source session key this draft was derived from.
    session_id: String,
    /// The full draft content (frontmatter + markdown body).
    content:    String,
}

/// Result returned by the write-skill-draft tool.
#[derive(Debug, Clone, Serialize)]
pub struct WriteSkillDraftResult {
    /// Operation status.
    pub status:  String,
    /// Absolute path to the written draft file.
    pub path:    String,
    /// Human-readable message.
    pub message: String,
}

/// Mita-exclusive tool: write a skill draft to the skill-drafts directory.
///
/// Mita generates structured drafts during heartbeat analysis of completed
/// sessions. The draft is written to `<data_dir>/skill-drafts/{session_id}.md`
/// for Rara to later read and convert into a full skill.
#[derive(ToolDef)]
#[tool(
    name = "write-skill-draft",
    description = "Write a skill draft file for a session that contained a reusable procedure. \
                   The draft is saved to disk so Rara can later read it, refine it, and create a \
                   proper skill. Provide the source session_id and the full draft content (YAML \
                   frontmatter with scoring + markdown body with task summary, approach, tool \
                   chain, and pitfalls)."
)]
pub struct WriteSkillDraftTool;

impl WriteSkillDraftTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for WriteSkillDraftTool {
    type Output = WriteSkillDraftResult;
    type Params = WriteSkillDraftParams;

    async fn run(
        &self,
        params: WriteSkillDraftParams,
        context: &ToolContext,
    ) -> anyhow::Result<WriteSkillDraftResult> {
        if params.session_id.trim().is_empty() {
            anyhow::bail!("session_id must not be empty");
        }
        if params.content.trim().is_empty() {
            anyhow::bail!("content must not be empty");
        }

        let drafts_dir = rara_paths::skill_drafts_dir();
        std::fs::create_dir_all(&drafts_dir)
            .map_err(|e| anyhow::anyhow!("failed to create skill-drafts directory: {e}"))?;

        // Sanitize session_id for use as filename.
        let safe_name: String = params
            .session_id
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        let draft_path = drafts_dir.join(format!("{safe_name}.md"));
        std::fs::write(&draft_path, &params.content)
            .map_err(|e| anyhow::anyhow!("failed to write skill draft: {e}"))?;

        let path_str = draft_path.display().to_string();

        info!(
            session_id = %params.session_id,
            path = %path_str,
            "skill draft written"
        );

        push_notification(
            context,
            format!(
                "\u{1f4cb} Skill draft written for session {}",
                params.session_id
            ),
        );

        Ok(WriteSkillDraftResult {
            status:  "ok".to_owned(),
            path:    path_str.clone(),
            message: format!("Skill draft written to {path_str}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use rara_kernel::tool::AgentTool;

    use super::*;

    #[test]
    fn tool_has_required_params() {
        let tool = WriteSkillDraftTool::new();
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "session_id"));
        assert!(required.iter().any(|v| v == "content"));
    }
}
