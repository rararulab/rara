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

//! Mita-exclusive tool for triggering soul evolution.
//!
//! Mita (the LLM) generates the proposed soul content herself, then passes
//! it to this tool for validation, snapshotting, and writing.

use rara_kernel::tool::{ToolContext, ToolOutput};
use rara_tool_macro::ToolDef;
use serde_json::json;
use tracing::info;

use super::notify::push_notification;

/// Mita-exclusive tool: evolve an agent's soul.md.
///
/// Mita generates the new soul content and passes it as `proposed_soul`.
/// The tool validates boundaries, snapshots the old soul, and writes the new
/// one.
#[derive(ToolDef)]
#[tool(
    name = "evolve-soul",
    description = "Write an evolved soul.md for an agent. You (Mita) must generate the full \
                   proposed soul content (frontmatter + markdown body) based on the accumulated \
                   state signals (emerged traits, style drift, discovered interests, relationship \
                   stage). The tool validates boundaries, snapshots the current soul, and writes \
                   the new version. The proposed_soul must preserve all immutable_traits and \
                   respect formality bounds from the current soul's boundaries section.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct EvolveSoulTool;

impl EvolveSoulTool {
    pub fn new() -> Self { Self }

    fn schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Target agent name (e.g. \"rara\")"
                },
                "proposed_soul": {
                    "type": "string",
                    "description": "Full proposed soul.md content (YAML frontmatter + markdown body). Must be valid soul format with --- delimiters."
                },
                "reason": {
                    "type": "string",
                    "description": "Brief summary of what changed and why (e.g. \"added playful humor based on observed style drift; relationship upgraded to friend\")"
                }
            },
            "required": ["agent", "proposed_soul", "reason"]
        })
    }

    async fn exec(
        &self,
        params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let agent = params
            .get("agent")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: agent"))?;
        let proposed_content = params
            .get("proposed_soul")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: proposed_soul"))?;
        let reason = params
            .get("reason")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: reason"))?;

        // 1. Load current soul.
        let loaded = rara_soul::load_soul(agent)
            .map_err(|e| anyhow::anyhow!("failed to load soul file: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("no soul file found for agent '{agent}'"))?;

        let current_soul = loaded.soul;
        let current_version = current_soul.frontmatter.version;

        // 2. Parse proposed soul.
        let mut proposed_soul = rara_soul::SoulFile::parse(proposed_content)
            .map_err(|e| anyhow::anyhow!("proposed soul is not valid: {e}"))?;

        // 3. Validate boundaries.
        let violations =
            rara_soul::validate_boundaries(&current_soul.frontmatter, &proposed_soul.frontmatter);
        if !violations.is_empty() {
            return Ok(json!({
                "status": "rejected",
                "agent": agent,
                "reason": "boundary violations",
                "violations": violations,
            })
            .into());
        }

        // 4. Snapshot current soul.
        let snapshots_dir = rara_soul::loader::snapshots_dir(agent);
        let snapshot_path = rara_soul::create_snapshot(&current_soul, &snapshots_dir)
            .map_err(|e| anyhow::anyhow!("failed to create snapshot: {e}"))?;

        // 5. Bump version and write new soul.
        proposed_soul.frontmatter.version = current_version + 1;
        let new_content = proposed_soul
            .to_string()
            .map_err(|e| anyhow::anyhow!("failed to serialize proposed soul: {e}"))?;

        let soul_file_path = rara_soul::soul_path(agent);
        if let Some(parent) = soul_file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&soul_file_path, &new_content)?;

        // 6. Log evolution event in state history.
        if let Ok(Some(mut state)) = rara_soul::loader::load_state(agent) {
            state.append_history(rara_soul::state::HistoryEntry {
                timestamp:   jiff::Timestamp::now(),
                r#type:      "soul_evolved".to_string(),
                description: format!(
                    "v{} \u{2192} v{}: {reason}",
                    current_version,
                    current_version + 1,
                ),
            });
            let _ = rara_soul::loader::save_state(agent, &state);
        }

        let new_version = current_version + 1;
        info!(
            agent,
            old_version = current_version,
            new_version,
            snapshot = %snapshot_path.display(),
            "soul evolved successfully"
        );

        push_notification(
            _context,
            format!(
                "\u{1f9ec} Soul evolved: {agent} v{current_version} \u{2192} \
                 v{new_version}\n{reason}"
            ),
        );

        Ok(json!({
            "status": "evolved",
            "agent": agent,
            "old_version": current_version,
            "new_version": new_version,
            "snapshot_path": snapshot_path.display().to_string(),
            "soul_path": soul_file_path.display().to_string(),
        })
        .into())
    }
}

#[cfg(test)]
mod tests {
    use rara_kernel::tool::AgentTool;

    use super::*;

    #[test]
    fn tool_has_required_params() {
        let tool = EvolveSoulTool::new();
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "agent"));
        assert!(required.iter().any(|v| v == "proposed_soul"));
    }
}
