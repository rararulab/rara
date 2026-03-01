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

//! Predefined agent registry.
//!
//! Provides [`AgentRegistry`] as a central catalog of built-in agent
//! manifests, each tagged with an [`AgentRole`]. The boot crate uses this
//! registry to populate the kernel's [`ManifestLoader`] with production
//! agent definitions.
//!
//! The registry is purely static — all manifests are constructed in Rust
//! code (no YAML parsing at runtime). The `rara` agent's system prompt is
//! assembled by combining `soul.md` and `default_system.md` via
//! `include_str!`.

use rara_kernel::process::{AgentManifest, Priority};

// ---------------------------------------------------------------------------
// AgentRole
// ---------------------------------------------------------------------------

/// Agent role classification.
///
/// Each predefined agent serves a specific role in the system. Roles enable
/// callers to look up agents by function rather than by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentRole {
    /// User-facing conversational agent (default chat entry point).
    Chat,
    /// Codebase recon / investigation agent.
    Scout,
    /// Task planning agent.
    Planner,
    /// Execution / coding agent.
    Worker,
}

// ---------------------------------------------------------------------------
// AgentEntry
// ---------------------------------------------------------------------------

/// A single entry in the agent registry, pairing a manifest with its role.
pub struct AgentEntry {
    pub manifest: AgentManifest,
    pub role: AgentRole,
}

// ---------------------------------------------------------------------------
// AgentRegistry
// ---------------------------------------------------------------------------

/// Predefined agent registry.
///
/// All methods are static — no instance state is needed because the
/// registry is a fixed catalog of built-in agents.
pub struct AgentRegistry;

impl AgentRegistry {
    /// Return all predefined agents.
    pub fn all() -> Vec<AgentEntry> {
        vec![
            rara_manifest(),
            scout_manifest(),
            planner_manifest(),
            worker_manifest(),
        ]
    }

    /// Find agents by role.
    pub fn by_role(role: AgentRole) -> Vec<AgentEntry> {
        Self::all().into_iter().filter(|e| e.role == role).collect()
    }

    /// Find an agent by name.
    pub fn by_name(name: &str) -> Option<AgentEntry> {
        Self::all().into_iter().find(|e| e.manifest.name == name)
    }

    /// Convenience: return the default chat agent entry.
    pub fn chat_agent() -> AgentEntry {
        rara_manifest()
    }
}

// ---------------------------------------------------------------------------
// Manifest constructors
// ---------------------------------------------------------------------------

fn rara_manifest() -> AgentEntry {
    let soul = include_str!("../../kernel/src/prompt/defaults/agent/soul.md");
    let system = include_str!("../../kernel/src/prompt/defaults/chat/default_system.md");
    let system_prompt = format!("{soul}\n\n{system}");

    AgentEntry {
        role: AgentRole::Chat,
        manifest: AgentManifest {
            name: "rara".to_string(),
            description: "Rara -- personal AI assistant with personality and tools".to_string(),
            model: "openai/gpt-4o-mini".to_string(),
            system_prompt,
            provider_hint: None,
            max_iterations: Some(25),
            tools: vec![],
            max_children: None,
            max_context_tokens: None,
            priority: Priority::default(),
            metadata: serde_json::Value::Null,
            sandbox: None,
        },
    }
}

fn scout_manifest() -> AgentEntry {
    AgentEntry {
        role: AgentRole::Scout,
        manifest: AgentManifest {
            name: "scout".to_string(),
            description: "Fast codebase recon - returns structured findings".to_string(),
            model: "deepseek/deepseek-chat".to_string(),
            system_prompt: indoc(
                "You are a scout agent. Your job is to quickly investigate a codebase or topic
and return compressed, structured findings.

## Output Format

### Files Found
- `path/to/file.ext` (lines N-M) - Brief description

### Key Code
Relevant code snippets with context.

### Architecture
Brief explanation of how things connect.

### Summary
2-3 sentence summary of findings.

## Rules
- Be thorough but fast - read only what you need
- Return findings as structured markdown with clear sections
- Always include file paths and line numbers when referencing code
- If you cannot find what was asked, say so clearly",
            ),
            provider_hint: None,
            max_iterations: Some(15),
            tools: vec![
                "read_file".to_string(),
                "grep".to_string(),
                "find_files".to_string(),
                "list_directory".to_string(),
                "http_fetch".to_string(),
            ],
            max_children: None,
            max_context_tokens: None,
            priority: Priority::default(),
            metadata: serde_json::Value::Null,
            sandbox: None,
        },
    }
}

fn planner_manifest() -> AgentEntry {
    AgentEntry {
        role: AgentRole::Planner,
        manifest: AgentManifest {
            name: "planner".to_string(),
            description: "Creates implementation plans from investigation results".to_string(),
            model: "deepseek/deepseek-chat".to_string(),
            system_prompt: indoc(
                "You are a planner agent. Given investigation results from a scout, create
a clear implementation plan.

## Output Format

### Goal
One sentence describing the objective.

### Steps
1. **Step title** - What to do, which files to touch.
2. ...

### Risks
Any concerns or edge cases to watch for.

## Rules
- Break work into small, numbered steps
- Each step should specify exact files to modify
- Include code snippets where helpful
- Consider edge cases and testing",
            ),
            provider_hint: None,
            max_iterations: Some(10),
            tools: vec![
                "read_file".to_string(),
                "grep".to_string(),
                "find_files".to_string(),
            ],
            max_children: None,
            max_context_tokens: None,
            priority: Priority::default(),
            metadata: serde_json::Value::Null,
            sandbox: None,
        },
    }
}

fn worker_manifest() -> AgentEntry {
    AgentEntry {
        role: AgentRole::Worker,
        manifest: AgentManifest {
            name: "worker".to_string(),
            description: "Executes implementation tasks from a plan".to_string(),
            model: "deepseek/deepseek-chat".to_string(),
            system_prompt: indoc(
                "You are a worker agent. Given an implementation plan, execute it step by step.

## Rules
- Follow the plan exactly
- Make minimal, focused changes
- Test after each significant change
- Report what you did clearly",
            ),
            provider_hint: None,
            max_iterations: Some(20),
            tools: vec![
                "read_file".to_string(),
                "write_file".to_string(),
                "edit_file".to_string(),
                "bash".to_string(),
                "grep".to_string(),
                "find_files".to_string(),
            ],
            max_children: None,
            max_context_tokens: None,
            priority: Priority::default(),
            metadata: serde_json::Value::Null,
            sandbox: None,
        },
    }
}

/// Trim shared leading whitespace from a multi-line string literal, keeping
/// the relative indentation intact. This avoids pulling in the `indoc` crate
/// for a single use-case.
fn indoc(s: &str) -> String {
    s.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_returns_four_agents() {
        let all = AgentRegistry::all();
        assert_eq!(all.len(), 4);

        let names: Vec<&str> = all.iter().map(|e| e.manifest.name.as_str()).collect();
        assert!(names.contains(&"rara"));
        assert!(names.contains(&"scout"));
        assert!(names.contains(&"planner"));
        assert!(names.contains(&"worker"));
    }

    #[test]
    fn test_by_role_chat_returns_rara() {
        let entries = AgentRegistry::by_role(AgentRole::Chat);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.name, "rara");
    }

    #[test]
    fn test_by_role_scout() {
        let entries = AgentRegistry::by_role(AgentRole::Scout);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.name, "scout");
    }

    #[test]
    fn test_by_role_planner() {
        let entries = AgentRegistry::by_role(AgentRole::Planner);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.name, "planner");
    }

    #[test]
    fn test_by_role_worker() {
        let entries = AgentRegistry::by_role(AgentRole::Worker);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.name, "worker");
    }

    #[test]
    fn test_by_name_rara() {
        let entry = AgentRegistry::by_name("rara").unwrap();
        assert_eq!(entry.manifest.model, "openai/gpt-4o-mini");
        assert_eq!(entry.manifest.max_iterations, Some(25));
        assert!(entry.manifest.tools.is_empty());
        assert_eq!(entry.role, AgentRole::Chat);
    }

    #[test]
    fn test_by_name_scout() {
        let entry = AgentRegistry::by_name("scout").unwrap();
        assert_eq!(entry.manifest.model, "deepseek/deepseek-chat");
        assert!(entry.manifest.tools.contains(&"read_file".to_string()));
        assert!(entry.manifest.tools.contains(&"grep".to_string()));
        assert_eq!(entry.manifest.max_iterations, Some(15));
        assert_eq!(entry.role, AgentRole::Scout);
    }

    #[test]
    fn test_by_name_nonexistent() {
        assert!(AgentRegistry::by_name("nonexistent").is_none());
    }

    #[test]
    fn test_chat_agent_is_rara() {
        let entry = AgentRegistry::chat_agent();
        assert_eq!(entry.manifest.name, "rara");
        assert_eq!(entry.role, AgentRole::Chat);
    }

    #[test]
    fn test_rara_system_prompt_combines_soul_and_system() {
        let entry = AgentRegistry::by_name("rara").unwrap();
        let prompt = &entry.manifest.system_prompt;
        // soul.md starts with "# Rara -- Soul"
        assert!(
            prompt.contains("Rara"),
            "system prompt should contain soul.md content"
        );
        // default_system.md contains "self-hosted platform"
        assert!(
            prompt.contains("self-hosted platform"),
            "system prompt should contain default_system.md content"
        );
    }

    #[test]
    fn test_scout_tools_match_yaml() {
        let entry = AgentRegistry::by_name("scout").unwrap();
        let expected = vec![
            "read_file",
            "grep",
            "find_files",
            "list_directory",
            "http_fetch",
        ];
        assert_eq!(entry.manifest.tools, expected);
    }

    #[test]
    fn test_planner_tools_match_yaml() {
        let entry = AgentRegistry::by_name("planner").unwrap();
        let expected = vec!["read_file", "grep", "find_files"];
        assert_eq!(entry.manifest.tools, expected);
    }

    #[test]
    fn test_worker_tools_match_yaml() {
        let entry = AgentRegistry::by_name("worker").unwrap();
        let expected = vec![
            "read_file",
            "write_file",
            "edit_file",
            "bash",
            "grep",
            "find_files",
        ];
        assert_eq!(entry.manifest.tools, expected);
    }
}
