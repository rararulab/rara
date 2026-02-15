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
use rara_skills::registry::SkillRegistry;
use serde_json::{json, Value};

/// Format a skill as a `.md` file with YAML frontmatter.
fn format_skill_md(
    name: &str,
    description: &str,
    tools: &[String],
    trigger: Option<&str>,
    enabled: bool,
    prompt: &str,
) -> String {
    let mut md = String::from("---\n");
    md.push_str(&format!("name: {name}\n"));
    md.push_str(&format!(
        "description: \"{}\"\n",
        description.replace('"', "\\\"")
    ));
    if !tools.is_empty() {
        md.push_str("tools:\n");
        for tool in tools {
            md.push_str(&format!("  - {tool}\n"));
        }
    }
    if let Some(trigger) = trigger {
        md.push_str(&format!(
            "trigger: \"{}\"\n",
            trigger.replace('"', "\\\"")
        ));
    }
    if !enabled {
        md.push_str("enabled: false\n");
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
    registry: Arc<RwLock<SkillRegistry>>,
}

impl ListSkillsTool {
    pub fn new(registry: Arc<RwLock<SkillRegistry>>) -> Self { Self { registry } }
}

#[async_trait]
impl AgentTool for ListSkillsTool {
    fn name(&self) -> &str { "list_skills" }

    fn description(&self) -> &str {
        "List all available skills with their metadata (name, description, tools, trigger, enabled \
         status)."
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
            .map(|s| {
                json!({
                    "name": s.name(),
                    "description": s.description(),
                    "tools": s.tools(),
                    "trigger": s.trigger_pattern(),
                    "enabled": s.is_enabled(),
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

/// Tool that creates a new skill by writing a `.md` file and inserting it
/// into the registry.
pub struct CreateSkillTool {
    registry: Arc<RwLock<SkillRegistry>>,
}

impl CreateSkillTool {
    pub fn new(registry: Arc<RwLock<SkillRegistry>>) -> Self { Self { registry } }
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
                    "description": "Unique skill name (used as filename)"
                },
                "description": {
                    "type": "string",
                    "description": "Short description of what the skill does"
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of tool names this skill uses"
                },
                "trigger": {
                    "type": "string",
                    "description": "Regex pattern that activates this skill"
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

        let tools: Vec<String> = params
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        let trigger = params.get("trigger").and_then(|v| v.as_str());

        // Build the skill markdown content.
        let content = format_skill_md(name, description, &tools, trigger, true, prompt);

        // Write to skills_dir()/{name}.md.
        let skills_dir = rara_paths::skills_dir();
        std::fs::create_dir_all(skills_dir.as_path()).map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to create skills directory: {e}").into(),
            }
        })?;

        let file_path = skills_dir.join(format!("{name}.md"));
        std::fs::write(&file_path, &content).map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to write skill file: {e}").into(),
            }
        })?;

        // Parse the file back and insert into registry.
        let skill =
            rara_skills::loader::parse_skill_file(&file_path).map_err(|e| {
                rara_agents::err::Error::Other {
                    message: format!("failed to parse skill file: {e}").into(),
                }
            })?;

        let mut registry = self.registry.write().map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to acquire skill registry lock: {e}").into(),
            }
        })?;
        registry.insert(skill);

        Ok(json!({
            "created": name,
            "path": file_path.to_string_lossy(),
        }))
    }
}

// ---------------------------------------------------------------------------
// UpdateSkillTool
// ---------------------------------------------------------------------------

/// Tool that updates an existing skill by merging new fields with the current
/// metadata, re-writing the file, and refreshing the registry.
pub struct UpdateSkillTool {
    registry: Arc<RwLock<SkillRegistry>>,
}

impl UpdateSkillTool {
    pub fn new(registry: Arc<RwLock<SkillRegistry>>) -> Self { Self { registry } }
}

#[async_trait]
impl AgentTool for UpdateSkillTool {
    fn name(&self) -> &str { "update_skill" }

    fn description(&self) -> &str {
        "Update an existing skill. Merges provided fields with current values and re-writes the \
         skill file."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill to update"
                },
                "description": {
                    "type": "string",
                    "description": "New description (optional)"
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "New list of tool names (optional)"
                },
                "trigger": {
                    "type": "string",
                    "description": "New trigger regex pattern (optional)"
                },
                "prompt": {
                    "type": "string",
                    "description": "New prompt body (optional)"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Enable or disable the skill (optional)"
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

        // Read existing skill from registry.
        let (existing_description, existing_tools, existing_trigger, existing_enabled, existing_prompt, source_path) = {
            let registry = self.registry.read().map_err(|e| {
                rara_agents::err::Error::Other {
                    message: format!("failed to acquire skill registry lock: {e}").into(),
                }
            })?;
            let skill = registry.get(name).ok_or_else(|| {
                rara_agents::err::Error::Other {
                    message: format!("skill not found: {name}").into(),
                }
            })?;
            (
                skill.description().to_owned(),
                skill.tools().to_vec(),
                skill.trigger_pattern().map(ToOwned::to_owned),
                skill.is_enabled(),
                skill.prompt.clone(),
                skill.source_path.clone(),
            )
        };

        // Merge with provided fields.
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .map_or(existing_description, |s| s.to_owned());

        let tools: Vec<String> = params
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or(existing_tools);

        let trigger = match params.get("trigger") {
            Some(v) => v.as_str().map(ToOwned::to_owned),
            None => existing_trigger,
        };

        let enabled = params
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(existing_enabled);

        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .map_or(existing_prompt, |s| s.to_owned());

        // Write updated file.
        let content = format_skill_md(
            name,
            &description,
            &tools,
            trigger.as_deref(),
            enabled,
            &prompt,
        );

        std::fs::write(&source_path, &content).map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to write skill file: {e}").into(),
            }
        })?;

        // Re-parse and update registry.
        let skill =
            rara_skills::loader::parse_skill_file(&source_path).map_err(|e| {
                rara_agents::err::Error::Other {
                    message: format!("failed to parse updated skill file: {e}").into(),
                }
            })?;

        let mut registry = self.registry.write().map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to acquire skill registry lock: {e}").into(),
            }
        })?;
        registry.insert(skill);

        Ok(json!({ "updated": name }))
    }
}

// ---------------------------------------------------------------------------
// DeleteSkillTool
// ---------------------------------------------------------------------------

/// Tool that deletes a skill by removing its file and unregistering it.
pub struct DeleteSkillTool {
    registry: Arc<RwLock<SkillRegistry>>,
}

impl DeleteSkillTool {
    pub fn new(registry: Arc<RwLock<SkillRegistry>>) -> Self { Self { registry } }
}

#[async_trait]
impl AgentTool for DeleteSkillTool {
    fn name(&self) -> &str { "delete_skill" }

    fn description(&self) -> &str { "Delete a skill by removing its file and unregistering it." }

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

        // Get the source path before removing from registry.
        let source_path = {
            let registry = self.registry.read().map_err(|e| {
                rara_agents::err::Error::Other {
                    message: format!("failed to acquire skill registry lock: {e}").into(),
                }
            })?;
            let skill = registry.get(name).ok_or_else(|| {
                rara_agents::err::Error::Other {
                    message: format!("skill not found: {name}").into(),
                }
            })?;
            skill.source_path.clone()
        };

        // Remove the file.
        if source_path.exists() {
            std::fs::remove_file(&source_path).map_err(|e| {
                rara_agents::err::Error::Other {
                    message: format!("failed to remove skill file: {e}").into(),
                }
            })?;
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
