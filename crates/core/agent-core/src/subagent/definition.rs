//! Agent definition types and markdown parser.
//!
//! An agent definition is a markdown file with YAML frontmatter that describes
//! a specialized sub-agent: its name, LLM model, available tools, iteration
//! limit, and system prompt. The format mirrors the skills system.
//!
//! # File Format
//!
//! ```markdown
//! ---
//! name: scout                          # required, unique identifier
//! description: "Fast codebase recon"   # optional, shown in tool schema
//! model: "deepseek/deepseek-chat"      # optional, falls back to default
//! tools:                               # optional, empty = all parent tools
//!   - read_file
//!   - grep
//! max_iterations: 15                   # optional, default 15
//! ---
//!
//! System prompt body goes here (markdown).
//! ```
//!
//! # Loading
//!
//! Definitions are loaded from two directories (user overrides bundled):
//! 1. Bundled: `<project_root>/agents/` — shipped with the binary
//! 2. User: `<data_dir>/agents/` — user-defined, can override bundled by name

use std::{collections::HashMap, path::Path};

use serde::Deserialize;
use tracing::warn;

use crate::err::prelude::*;

/// Internal deserialization target for the YAML frontmatter section.
/// All fields except `name` are optional with serde defaults.
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

/// A fully parsed agent definition, combining frontmatter metadata with the
/// markdown body as the system prompt.
///
/// Created by [`AgentDefinition::parse`] from raw markdown content, or loaded
/// in bulk via [`AgentDefinitionRegistry::load_dir`].
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    /// Unique identifier for this agent (e.g. "scout", "planner", "worker").
    /// Used as the lookup key in [`AgentDefinitionRegistry`] and as the
    /// `agent` field in [`SubagentParams`](super::tool::SubagentParams).
    pub name:           String,
    /// Human-readable description shown in the SubagentTool's JSON schema,
    /// helping the LLM understand what this agent specializes in.
    pub description:    String,
    /// LLM model to use for this agent (e.g. "deepseek/deepseek-chat").
    /// When `None`, the executor falls back to the default model configured
    /// in settings (typically the Chat scenario model).
    pub model:          Option<String>,
    /// Tool whitelist — only these tools are available to the sub-agent.
    /// When empty, all parent tools are inherited (minus "subagent" itself).
    /// Tool names must match those registered in the parent [`ToolRegistry`].
    pub tools:          Vec<String>,
    /// Maximum LLM round-trips before the sub-agent gives up.
    /// When `None`, defaults to 15 in the executor.
    pub max_iterations: Option<usize>,
    /// The markdown body below the frontmatter, used as the sub-agent's
    /// system prompt. This defines the agent's persona, output format,
    /// and behavioral guidelines.
    pub system_prompt:  String,
}

impl AgentDefinition {
    /// Parse a markdown string with YAML frontmatter into an
    /// [`AgentDefinition`].
    ///
    /// The content must start with `---`, followed by valid YAML, then a
    /// closing `---` line. Everything after the closing delimiter becomes
    /// the `system_prompt`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The content doesn't start with `---` (missing frontmatter)
    /// - No closing `---` delimiter is found
    /// - The YAML frontmatter is malformed or missing required `name` field
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

/// In-memory registry of named agent definitions.
///
/// Holds all loaded [`AgentDefinition`]s keyed by name. Definitions are
/// registered at startup from the `agents/` directory and queried at runtime
/// by the [`SubagentTool`](super::tool::SubagentTool) when the LLM requests
/// a sub-agent by name.
///
/// Registering a definition with a name that already exists will silently
/// overwrite the previous one. This allows user-defined agents to override
/// bundled ones.
#[derive(Debug, Clone, Default)]
pub struct AgentDefinitionRegistry {
    defs: HashMap<String, AgentDefinition>,
}

impl AgentDefinitionRegistry {
    pub fn new() -> Self { Self::default() }

    /// Register (or replace) an agent definition.
    pub fn register(&mut self, def: AgentDefinition) { self.defs.insert(def.name.clone(), def); }

    /// Look up an agent definition by name.
    pub fn get(&self, name: &str) -> Option<&AgentDefinition> { self.defs.get(name) }

    /// Return all registered definitions (unordered).
    pub fn list(&self) -> Vec<&AgentDefinition> { self.defs.values().collect() }

    /// Scan a directory for `.md` files and parse each as an agent definition.
    ///
    /// Non-existent directories are silently ignored (returns empty registry).
    /// Individual files that fail to parse are logged as warnings and skipped.
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

/// Split markdown content at `---` delimiters into `(frontmatter_yaml, body)`.
///
/// Expects the content to start with `---` (after optional leading whitespace),
/// followed by YAML content, then a line containing only `---`, then the body.
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
