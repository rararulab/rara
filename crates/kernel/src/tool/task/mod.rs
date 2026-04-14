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

//! Background task tool — delegates work to a child agent with preset
//! configurations.

pub(crate) mod presets;

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    agent::{AgentManifest, AgentRole, Priority},
    handle::KernelHandle,
    session::SessionKey,
    tool::{ToolContext, ToolExecute, spawn_background::slug_from_description},
};

/// Builtin tool that launches a background task using a predefined agent type.
///
/// Instead of requiring raw manifest parameters, callers pick a `task_type`
/// (e.g. `"general-purpose"`, `"bash"`) and the tool resolves all configuration
/// from the preset registry.
#[derive(ToolDef)]
#[tool(
    name = "task",
    description = "Launch a background task using a predefined agent type. Pick a task_type: \
                   'explore' for codebase analysis and research, 'bash' for shell/CLI operations, \
                   'general-purpose' for complex multi-step tasks with full tool access. The \
                   agent runs independently and results are delivered when complete. Only the \
                   final result enters your context — intermediate tool calls stay in the \
                   child.\n\nWHEN TO USE:\n- Research spanning 3+ queries or multiple files\n- \
                   Tasks whose intermediate output would flood your context\n- Independent \
                   workstreams that can run in parallel\n- Reasoning-heavy subtasks (debugging, \
                   code review, analysis)\n\nWHEN NOT TO USE:\n- Single tool call — just call the \
                   tool directly\n- Tasks needing user interaction — child agents cannot ask the \
                   user\n\nIMPORTANT: The child agent has NO memory of your conversation. Pass \
                   ALL relevant context (file paths, error messages, constraints) in the prompt.",
    tier = "deferred"
)]
pub struct TaskTool {
    handle:      KernelHandle,
    session_key: SessionKey,
}

impl TaskTool {
    /// Create a new `TaskTool` bound to a parent session.
    pub fn new(handle: KernelHandle, session_key: SessionKey) -> Self {
        Self {
            handle,
            session_key,
        }
    }
}

/// Parameters for the `task` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskParams {
    /// Short human-readable description of the task (3-5 words, shown in
    /// status).
    description: String,
    /// The task instruction to send to the background agent.
    prompt:      String,
    /// Predefined agent type: 'general-purpose' (full tool access, complex
    /// tasks) or 'bash' (shell/CLI specialist).
    task_type:   String,
}

#[async_trait]
impl ToolExecute for TaskTool {
    type Output = serde_json::Value;
    type Params = TaskParams;

    async fn run(&self, p: TaskParams, context: &ToolContext) -> anyhow::Result<serde_json::Value> {
        let preset = presets::get_preset(&p.task_type).ok_or_else(|| {
            let available = presets::list_preset_names().join(", ");
            anyhow::anyhow!(
                "unknown task_type '{}'. Available types: {}",
                p.task_type,
                available
            )
        })?;

        let agent_name = slug_from_description(&p.description);

        // Build manifest from preset configuration.
        let manifest = AgentManifest {
            name:                   agent_name,
            role:                   AgentRole::Worker,
            description:            p.description,
            model:                  None,
            system_prompt:          preset.system_prompt.to_owned(),
            soul_prompt:            None,
            provider_hint:          None,
            max_iterations:         Some(preset.max_iterations),
            tools:                  preset.allowed_tools.clone(),
            excluded_tools:         preset.disallowed_tools.clone(),
            max_children:           Some(0),
            max_context_tokens:     None,
            priority:               Priority::default(),
            metadata:               serde_json::Value::Null,
            sandbox:                None,
            default_execution_mode: None,
            tool_call_limit:        None,
            worker_timeout_secs:    Some(preset.max_iterations as u64 * 60),
            max_continuations:      Some(0),
        };

        let mut result = super::background_common::spawn_and_register_background(
            &self.handle,
            &self.session_key,
            manifest,
            p.prompt,
            context,
        )
        .await?;

        // Merge task_type into the shared response payload.
        if let serde_json::Value::Object(ref mut map) = result {
            map.insert(
                "task_type".to_string(),
                serde_json::Value::String(p.task_type),
            );
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_params_deserializes_valid_json() {
        let json = serde_json::json!({
            "description": "test task",
            "prompt": "Do something",
            "task_type": "general-purpose"
        });
        let params: TaskParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.description, "test task");
        assert_eq!(params.prompt, "Do something");
        assert_eq!(params.task_type, "general-purpose");
    }

    #[test]
    fn task_params_rejects_missing_fields() {
        let json = serde_json::json!({
            "description": "test task"
            // missing prompt and task_type
        });
        assert!(serde_json::from_value::<TaskParams>(json).is_err());
    }

    #[test]
    fn preset_builds_valid_manifest() {
        let preset = presets::get_preset("general-purpose").unwrap();
        let manifest = AgentManifest {
            name:                   "test-agent".to_string(),
            role:                   AgentRole::Worker,
            description:            "test".to_string(),
            model:                  None,
            system_prompt:          preset.system_prompt.to_string(),
            soul_prompt:            None,
            provider_hint:          None,
            max_iterations:         Some(preset.max_iterations),
            tools:                  preset.allowed_tools.clone(),
            excluded_tools:         preset.disallowed_tools.clone(),
            max_children:           Some(0),
            max_context_tokens:     None,
            priority:               Priority::default(),
            metadata:               serde_json::Value::Null,
            sandbox:                None,
            default_execution_mode: None,
            tool_call_limit:        None,
            worker_timeout_secs:    Some(preset.max_iterations as u64 * 60),
            max_continuations:      Some(0),
        };
        assert_eq!(manifest.role, AgentRole::Worker);
        assert_eq!(manifest.max_children, Some(0));
        assert!(
            manifest
                .excluded_tools
                .contains(&crate::tool::ToolName::new("task"))
        );
        assert!(
            manifest
                .excluded_tools
                .contains(&crate::tool::ToolName::new("spawn-background"))
        );
        // general-purpose inherits all tools
        assert!(manifest.tools.is_empty());
    }

    #[test]
    fn bash_preset_has_explicit_tools() {
        let preset = presets::get_preset("bash").unwrap();
        let manifest_tools = &preset.allowed_tools;
        assert!(manifest_tools.contains(&crate::tool::ToolName::new("bash")));
        assert!(manifest_tools.contains(&crate::tool::ToolName::new("read-file")));
        // bash preset should NOT include task
        assert!(!manifest_tools.contains(&crate::tool::ToolName::new("task")));
    }
}
