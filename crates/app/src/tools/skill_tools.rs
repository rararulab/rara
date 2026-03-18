// Copyright 2025 Crrow
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

//! Layer 2 service tools for managing agent skills.

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_skills::registry::InMemoryRegistry;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Format a SKILL.md file with YAML frontmatter (new format).
fn format_skill_md(
    name: &str,
    description: &str,
    allowed_tools: &[String],
    prompt: &str,
) -> String {
    let mut md = String::from("---\n");
    md.push_str(&format!("name: {name}\n"));
    md.push_str(&format!(
        "description: \"{}\"\n",
        description.replace('"', "\\\"")
    ));
    if !allowed_tools.is_empty() {
        md.push_str("allowed-tools:\n");
        for tool in allowed_tools {
            md.push_str(&format!("  - {tool}\n"));
        }
    }
    md.push_str("---\n\n");
    md.push_str(prompt);
    md.push('\n');
    md
}

// ---------------------------------------------------------------------------
// ListSkillsTool
// ---------------------------------------------------------------------------

/// Input parameters for the list-skills tool (no parameters required).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSkillsParams {}

/// Tool that lists all available skills with their metadata.
#[derive(ToolDef)]
#[tool(
    name = "list-skills",
    description = "List all available skills with their metadata (name, description, \
                   allowed_tools, source, eligibility)."
)]
pub struct ListSkillsTool {
    registry: InMemoryRegistry,
}

impl ListSkillsTool {
    pub fn new(registry: InMemoryRegistry) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for ListSkillsTool {
    type Output = Value;
    type Params = ListSkillsParams;

    async fn run(
        &self,
        _params: ListSkillsParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        let skills: Vec<Value> = self
            .registry
            .list_all()
            .iter()
            .map(|meta| {
                let elig = rara_skills::requirements::check_requirements(meta);
                json!({
                    "name": meta.name,
                    "description": meta.description,
                    "allowed_tools": meta.allowed_tools,
                    "source": meta.source.as_ref().map(|s| format!("{s:?}").to_lowercase()),
                    "homepage": meta.homepage,
                    "license": meta.license,
                    "eligible": elig.eligible,
                })
            })
            .collect();
        let count = skills.len();
        Ok(json!({ "skills": skills, "count": count }))
    }
}

// ---------------------------------------------------------------------------
// CreateSkillTool
// ---------------------------------------------------------------------------

/// Input parameters for the create-skill tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateSkillParams {
    /// Unique skill name (lowercase, hyphens allowed).
    name:          String,
    /// Short description of what the skill does.
    description:   String,
    /// List of tools this skill is allowed to use.
    allowed_tools: Option<Vec<String>>,
    /// The prompt body (instructions for the agent).
    prompt:        String,
}

/// Tool that creates a new skill by writing a `SKILL.md` file inside a skill
/// directory and inserting the parsed metadata into the registry.
#[derive(ToolDef)]
#[tool(
    name = "create-skill",
    description = "Create a new skill by writing a SKILL.md file with frontmatter and prompt body."
)]
pub struct CreateSkillTool {
    registry: InMemoryRegistry,
}

impl CreateSkillTool {
    pub fn new(registry: InMemoryRegistry) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for CreateSkillTool {
    type Output = Value;
    type Params = CreateSkillParams;

    async fn run(
        &self,
        params: CreateSkillParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        let allowed_tools = params.allowed_tools.unwrap_or_default();

        // Build the SKILL.md content.
        let content = format_skill_md(
            &params.name,
            &params.description,
            &allowed_tools,
            &params.prompt,
        );

        // Write to skills_dir()/{name}/SKILL.md.
        let skills_dir = rara_paths::skills_dir();
        let skill_dir = skills_dir.join(&params.name);
        std::fs::create_dir_all(&skill_dir)
            .map_err(|e| anyhow::anyhow!("failed to create skill directory: {e}"))?;

        let file_path = skill_dir.join("SKILL.md");
        std::fs::write(&file_path, &content)
            .map_err(|e| anyhow::anyhow!("failed to write skill file: {e}"))?;

        // Parse the file back and insert into registry.
        let raw = std::fs::read_to_string(&file_path)
            .map_err(|e| anyhow::anyhow!("failed to read skill file: {e}"))?;
        let mut meta = rara_skills::parse::parse_metadata(&raw, &skill_dir)
            .map_err(|e| anyhow::anyhow!("failed to parse skill file: {e}"))?;
        meta.source = Some(rara_skills::types::SkillSource::Personal);

        self.registry.insert(meta);

        Ok(json!({
            "created": params.name,
            "path": file_path.to_string_lossy(),
        }))
    }
}

// ---------------------------------------------------------------------------
// DeleteSkillTool
// ---------------------------------------------------------------------------

/// Input parameters for the delete-skill tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteSkillParams {
    /// Name of the skill to delete.
    name: String,
}

/// Tool that deletes a skill by removing its directory and unregistering it.
#[derive(ToolDef)]
#[tool(
    name = "delete-skill",
    description = "Delete a skill by removing its directory and unregistering it."
)]
pub struct DeleteSkillTool {
    registry: InMemoryRegistry,
}

impl DeleteSkillTool {
    pub fn new(registry: InMemoryRegistry) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for DeleteSkillTool {
    type Output = Value;
    type Params = DeleteSkillParams;

    async fn run(
        &self,
        params: DeleteSkillParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        // Get the skill path before removing from registry.
        let meta = self
            .registry
            .get(&params.name)
            .ok_or_else(|| anyhow::anyhow!("skill not found: {}", params.name))?;
        let skill_path = meta.path.clone();

        // Remove the directory (best-effort).
        if skill_path.exists() {
            let _ = std::fs::remove_dir_all(&skill_path);
        }

        // Remove from registry.
        self.registry.remove(&params.name);

        Ok(json!({ "deleted": params.name }))
    }
}
