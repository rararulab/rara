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

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use rara_skills::registry::InMemoryRegistry;
use serde_json::{json, Value};

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

/// Tool that lists all available skills with their metadata.
pub struct ListSkillsTool {
    registry: Arc<RwLock<InMemoryRegistry>>,
}

impl ListSkillsTool {
    pub fn new(registry: Arc<RwLock<InMemoryRegistry>>) -> Self { Self { registry } }
}

#[async_trait]
impl AgentTool for ListSkillsTool {
    fn name(&self) -> &str { "list_skills" }

    fn description(&self) -> &str {
        "List all available skills with their metadata (name, description, allowed_tools, source, \
         eligibility)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _params: Value) -> rara_agents::err::Result<Value> {
        let registry = self.registry.read().map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to acquire skill registry lock: {e}").into(),
            }
        })?;
        let skills: Vec<Value> = registry
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

/// Tool that creates a new skill by writing a `SKILL.md` file inside a skill
/// directory and inserting the parsed metadata into the registry.
pub struct CreateSkillTool {
    registry: Arc<RwLock<InMemoryRegistry>>,
}

impl CreateSkillTool {
    pub fn new(registry: Arc<RwLock<InMemoryRegistry>>) -> Self { Self { registry } }
}

#[async_trait]
impl AgentTool for CreateSkillTool {
    fn name(&self) -> &str { "create_skill" }

    fn description(&self) -> &str {
        "Create a new skill by writing a SKILL.md file with frontmatter and prompt body."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Unique skill name (lowercase, hyphens allowed)"
                },
                "description": {
                    "type": "string",
                    "description": "Short description of what the skill does"
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of tools this skill is allowed to use"
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt body (instructions for the agent)"
                }
            },
            "required": ["name", "description", "prompt"]
        })
    }

    async fn execute(&self, params: Value) -> rara_agents::err::Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: name".into(),
            })?;

        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: description".into(),
            })?;

        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: prompt".into(),
            })?;

        let allowed_tools: Vec<String> = params
            .get("allowed_tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        // Build the SKILL.md content.
        let content = format_skill_md(name, description, &allowed_tools, prompt);

        // Write to skills_dir()/{name}/SKILL.md.
        let skills_dir = rara_paths::skills_dir();
        let skill_dir = skills_dir.join(name);
        std::fs::create_dir_all(&skill_dir).map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to create skill directory: {e}").into(),
            }
        })?;

        let file_path = skill_dir.join("SKILL.md");
        std::fs::write(&file_path, &content).map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to write skill file: {e}").into(),
            }
        })?;

        // Parse the file back and insert into registry.
        let raw = std::fs::read_to_string(&file_path).map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to read skill file: {e}").into(),
            }
        })?;
        let mut meta =
            rara_skills::parse::parse_metadata(&raw, &skill_dir).map_err(|e| {
                rara_agents::err::Error::Other {
                    message: format!("failed to parse skill file: {e}").into(),
                }
            })?;
        meta.source = Some(rara_skills::types::SkillSource::Personal);

        let mut registry = self.registry.write().map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to acquire skill registry lock: {e}").into(),
            }
        })?;
        registry.insert(meta);

        Ok(json!({
            "created": name,
            "path": file_path.to_string_lossy(),
        }))
    }
}

// ---------------------------------------------------------------------------
// DeleteSkillTool
// ---------------------------------------------------------------------------

/// Tool that deletes a skill by removing its directory and unregistering it.
pub struct DeleteSkillTool {
    registry: Arc<RwLock<InMemoryRegistry>>,
}

impl DeleteSkillTool {
    pub fn new(registry: Arc<RwLock<InMemoryRegistry>>) -> Self { Self { registry } }
}

#[async_trait]
impl AgentTool for DeleteSkillTool {
    fn name(&self) -> &str { "delete_skill" }

    fn description(&self) -> &str {
        "Delete a skill by removing its directory and unregistering it."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill to delete"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, params: Value) -> rara_agents::err::Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: name".into(),
            })?;

        // Get the skill path before removing from registry.
        let skill_path = {
            let registry = self.registry.read().map_err(|e| {
                rara_agents::err::Error::Other {
                    message: format!("failed to acquire skill registry lock: {e}").into(),
                }
            })?;
            let meta = registry.get(name).ok_or_else(|| {
                rara_agents::err::Error::Other {
                    message: format!("skill not found: {name}").into(),
                }
            })?;
            meta.path.clone()
        };

        // Remove the directory (best-effort).
        if skill_path.exists() {
            let _ = std::fs::remove_dir_all(&skill_path);
        }

        // Remove from registry.
        let mut registry = self.registry.write().map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to acquire skill registry lock: {e}").into(),
            }
        })?;
        registry.remove(name);

        Ok(json!({ "deleted": name }))
    }
}
