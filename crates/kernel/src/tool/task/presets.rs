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

//! Predefined task type configurations for the `task` tool.
//!
//! Each preset bundles a system prompt, tool allowlist/denylist, and iteration
//! limit so the LLM only needs to pick a type name (e.g. `"bash"`) instead of
//! writing raw configuration.

use std::sync::LazyLock;

use crate::{tool::RECURSIVE_TOOL_DENYLIST, tool_names};

/// Configuration for a predefined task type.
///
/// Each preset defines the system prompt, tool constraints, and iteration
/// budget for a particular class of delegated work.
#[derive(Debug, Clone)]
pub struct TaskTypeConfig {
    /// Short identifier used to select this preset (e.g. `"bash"`).
    pub name:             &'static str,
    /// Human-readable one-line description of this task type.
    pub description:      &'static str,
    /// System prompt injected into the child agent conversation.
    pub system_prompt:    &'static str,
    /// Tools the child agent is allowed to use. Empty means inherit all
    /// parent tools (minus `disallowed_tools`).
    pub allowed_tools:    Vec<String>,
    /// Tools the child agent must never use (e.g. recursive spawning tools).
    pub disallowed_tools: Vec<String>,
    /// Maximum number of agent loop iterations before the task is stopped.
    pub max_iterations:   usize,
}

/// System prompt for the general-purpose task type.
const GENERAL_PURPOSE_PROMPT: &str = "\
You are a task-focused worker agent. Complete the assigned task efficiently and accurately.

Rules:
- Focus exclusively on the task described in the user message.
- Use tools immediately — do not narrate what you plan to do.
- If a tool call fails, diagnose the error and retry with a corrected approach.
- When the task is complete, respond with a concise summary of what was done.
- Respond in the same language as the task description.";

/// System prompt for the bash task type.
const BASH_PROMPT: &str = "\
You are a command-line specialist. Accomplish tasks using shell commands and file operations.

Rules:
- Always use absolute paths — never rely on the working directory.
- Chain independent commands with && for efficiency.
- When a command fails, read the error output and diagnose the root cause before retrying.
- Prefer targeted file reads (offset/limit) over reading entire large files.
- When the task is complete, respond with a concise summary of what was done.
- Respond in the same language as the task description.";

static PRESETS: LazyLock<Vec<TaskTypeConfig>> = LazyLock::new(|| {
    let disallowed: Vec<String> = RECURSIVE_TOOL_DENYLIST
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

    vec![
        TaskTypeConfig {
            name:             "general-purpose",
            description:      "Complex multi-step tasks; inherits all parent tools",
            system_prompt:    GENERAL_PURPOSE_PROMPT,
            allowed_tools:    vec![],
            disallowed_tools: disallowed.clone(),
            max_iterations:   25,
        },
        TaskTypeConfig {
            name:             "bash",
            description:      "Shell/CLI specialist for command-line tasks",
            system_prompt:    BASH_PROMPT,
            allowed_tools:    vec![
                tool_names::BASH.into(),
                tool_names::READ_FILE.into(),
                tool_names::WRITE_FILE.into(),
                tool_names::EDIT_FILE.into(),
                tool_names::LIST_DIRECTORY.into(),
                tool_names::GREP.into(),
            ],
            disallowed_tools: disallowed,
            max_iterations:   15,
        },
    ]
});

/// Look up a preset by name.
///
/// Returns `None` if no preset matches the given name.
pub fn get_preset(name: &str) -> Option<&'static TaskTypeConfig> {
    PRESETS.iter().find(|p| p.name == name)
}

/// Return the names of all available presets.
pub fn list_preset_names() -> Vec<&'static str> { PRESETS.iter().map(|p| p.name).collect() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_general_purpose_preset() {
        let preset = get_preset("general-purpose").expect("general-purpose preset should exist");
        assert_eq!(preset.name, "general-purpose");
        assert!(
            preset.allowed_tools.is_empty(),
            "general-purpose should inherit all parent tools"
        );
        assert_eq!(preset.max_iterations, 25);
        assert!(
            preset.disallowed_tools.contains(&"task".to_owned()),
            "must disallow recursive task spawning"
        );
        assert!(
            !preset.system_prompt.is_empty(),
            "system prompt must not be empty"
        );
    }

    #[test]
    fn lookup_bash_preset() {
        let preset = get_preset("bash").expect("bash preset should exist");
        assert_eq!(preset.name, "bash");
        assert_eq!(preset.max_iterations, 15);
        assert!(
            preset.allowed_tools.contains(&"bash".to_owned()),
            "bash preset must include the bash tool"
        );
        assert!(
            preset.allowed_tools.contains(&"read-file".to_owned()),
            "bash preset must include read-file"
        );
        assert!(
            preset
                .disallowed_tools
                .contains(&"spawn-background".to_owned()),
            "must disallow spawn-background"
        );
        assert!(
            preset.disallowed_tools.contains(&"create-plan".to_owned()),
            "must disallow create-plan"
        );
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(
            get_preset("nonexistent").is_none(),
            "unknown preset name should return None"
        );
    }

    #[test]
    fn list_presets_returns_all() {
        let names = list_preset_names();
        assert!(names.contains(&"general-purpose"));
        assert!(names.contains(&"bash"));
        assert_eq!(names.len(), 2, "should have exactly 2 presets");
    }
}
