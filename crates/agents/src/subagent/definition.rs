use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use tracing::warn;

use crate::err::prelude::*;

/// YAML frontmatter from an agent definition markdown file.
#[derive(Debug, Clone, Deserialize)]
struct AgentFrontmatter {
    name:           String,
    #[serde(default)]
    description:    String,
    #[serde(default)]
    model:          Option<String>,
    #[serde(default)]
    tools:          Vec<String>,
    #[serde(default)]
    max_iterations: Option<usize>,
}

/// A parsed agent definition: frontmatter metadata + system prompt body.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub name:           String,
    pub description:    String,
    pub model:          Option<String>,
    pub tools:          Vec<String>,
    pub max_iterations: Option<usize>,
    pub system_prompt:  String,
}

impl AgentDefinition {
    /// Parse a markdown string with YAML frontmatter into an AgentDefinition.
    pub fn parse(content: &str) -> Result<Self> {
        let (frontmatter, body) = split_frontmatter(content)?;
        let meta: AgentFrontmatter =
            serde_yaml::from_str(&frontmatter).map_err(|e| Error::Other {
                message: format!("invalid agent definition frontmatter: {e}").into(),
            })?;
        Ok(Self {
            name:           meta.name,
            description:    meta.description,
            model:          meta.model,
            tools:          meta.tools,
            max_iterations: meta.max_iterations,
            system_prompt:  body,
        })
    }
}

/// Registry holding named agent definitions.
#[derive(Debug, Clone, Default)]
pub struct AgentDefinitionRegistry {
    defs: HashMap<String, AgentDefinition>,
}

impl AgentDefinitionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, def: AgentDefinition) {
        self.defs.insert(def.name.clone(), def);
    }

    pub fn get(&self, name: &str) -> Option<&AgentDefinition> {
        self.defs.get(name)
    }

    pub fn list(&self) -> Vec<&AgentDefinition> {
        self.defs.values().collect()
    }

    /// Load all `.md` files from a directory as agent definitions.
    pub fn load_dir(dir: &Path) -> Result<Self> {
        let mut registry = Self::new();
        if !dir.is_dir() {
            return Ok(registry);
        }
        let entries = std::fs::read_dir(dir).map_err(|e| Error::IO {
            source:   e,
            location: snafu::Location::new(file!(), line!(), 0),
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md") {
                let content = std::fs::read_to_string(&path).map_err(|e| Error::IO {
                    source:   e,
                    location: snafu::Location::new(file!(), line!(), 0),
                })?;
                match AgentDefinition::parse(&content) {
                    Ok(def) => {
                        registry.register(def);
                    }
                    Err(err) => {
                        warn!(
                            path = %path.display(),
                            error = %err,
                            "skipping invalid agent definition"
                        );
                    }
                }
            }
        }
        Ok(registry)
    }
}

/// Split markdown content at `---` delimiters into (frontmatter, body).
fn split_frontmatter(content: &str) -> Result<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(Error::Other {
            message: "agent definition missing frontmatter (must start with ---)".into(),
        });
    }
    let after_open = &trimmed[3..];
    let close_pos = after_open.find("\n---").ok_or_else(|| Error::Other {
        message: "agent definition missing closing --- delimiter".into(),
    })?;
    let frontmatter = after_open[..close_pos].trim().to_string();
    let body = after_open[close_pos + 4..].trim().to_string();
    Ok((frontmatter, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_definition() {
        let content = r#"---
name: scout
description: "Fast codebase recon"
model: "deepseek/deepseek-chat"
tools:
  - read_file
  - grep
  - find_files
max_iterations: 15
---

You are a scout. Quickly investigate and return structured findings.
"#;
        let def = AgentDefinition::parse(content).unwrap();
        assert_eq!(def.name, "scout");
        assert_eq!(def.description, "Fast codebase recon");
        assert_eq!(def.model.as_deref(), Some("deepseek/deepseek-chat"));
        assert_eq!(def.tools, vec!["read_file", "grep", "find_files"]);
        assert_eq!(def.max_iterations, Some(15));
        assert!(def.system_prompt.contains("You are a scout"));
    }

    #[test]
    fn parse_minimal_definition() {
        let content = "---\nname: worker\ndescription: General worker\n---\nDo the work.\n";
        let def = AgentDefinition::parse(content).unwrap();
        assert_eq!(def.name, "worker");
        assert!(def.model.is_none());
        assert!(def.tools.is_empty());
        assert!(def.max_iterations.is_none());
    }

    #[test]
    fn parse_missing_frontmatter_fails() {
        let content = "# No frontmatter\nJust markdown.";
        assert!(AgentDefinition::parse(content).is_err());
    }

    #[test]
    fn registry_load_and_get() {
        let content = "---\nname: scout\ndescription: Recon\n---\nPrompt.\n";
        let mut registry = AgentDefinitionRegistry::new();
        registry.register(AgentDefinition::parse(content).unwrap());
        assert!(registry.get("scout").is_some());
        assert!(registry.get("nonexistent").is_none());
        assert_eq!(registry.list().len(), 1);
    }
}
