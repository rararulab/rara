//! ACP agent management tools — install, list, remove agents at runtime.

use std::collections::HashMap;

use rara_acp::registry::{AcpAgentConfig, AcpRegistryRef};
use rara_kernel::tool::{ToolContext, ToolOutput};
use rara_tool_macro::ToolDef;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// InstallAcpAgentTool
// ---------------------------------------------------------------------------

/// Tool that registers a new ACP agent in the registry.
#[derive(ToolDef)]
#[tool(
    name = "install-acp-agent",
    description = "Register a new ACP agent so it becomes available for delegation via \
                   acp-delegate. The agent is not started immediately — ACP agents are spawned on \
                   demand.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct InstallAcpAgentTool {
    registry: AcpRegistryRef,
}

impl InstallAcpAgentTool {
    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_name": {
                    "type": "string",
                    "description": "Unique name for the ACP agent"
                },
                "command": {
                    "type": "string",
                    "description": "Command to run the ACP agent (e.g. 'npx', 'node', 'gemini')"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Command-line arguments for the agent"
                },
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Environment variables to pass to the agent process"
                }
            },
            "required": ["agent_name", "command"]
        })
    }

    async fn exec(&self, params: Value, _context: &ToolContext) -> anyhow::Result<ToolOutput> {
        let agent_name = params
            .get("agent_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: agent_name"))?;

        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: command"))?;

        let args: Vec<String> = params
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        let env: HashMap<String, String> = params
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                    .collect()
            })
            .unwrap_or_default();

        let config = AcpAgentConfig {
            command: command.to_owned(),
            args,
            env,
            enabled: true,
            ..Default::default()
        };

        self.registry
            .add(agent_name.to_owned(), config)
            .await
            .map_err(|e| anyhow::anyhow!("failed to install ACP agent '{agent_name}': {e}"))?;

        Ok(json!({
            "status": "installed",
            "agent_name": agent_name,
            "message": format!(
                "ACP agent '{agent_name}' registered. Use acp-delegate to run it."
            ),
        })
        .into())
    }
}

// ---------------------------------------------------------------------------
// ListAcpAgentsTool
// ---------------------------------------------------------------------------

/// Tool that lists all registered ACP agents.
#[derive(ToolDef)]
#[tool(
    name = "list-acp-agents",
    description = "List all registered ACP agents with their status (enabled, builtin) and spawn \
                   command.",
    params_schema = "Self::schema_list()",
    execute_fn = "self.exec_list"
)]
pub struct ListAcpAgentsTool {
    registry: AcpRegistryRef,
}

impl ListAcpAgentsTool {
    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }

    fn schema_list() -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn exec_list(
        &self,
        _params: Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let all = self
            .registry
            .all_agents()
            .await
            .map_err(|e| anyhow::anyhow!("failed to list ACP agents: {e}"))?;

        let mut agents = Vec::new();
        let mut enabled_count = 0usize;
        for (name, config) in &all {
            if config.enabled {
                enabled_count += 1;
            }
            agents.push(json!({
                "name": name,
                "command": config.command,
                "args": config.args,
                "enabled": config.enabled,
                "builtin": config.builtin,
            }));
        }

        Ok(json!({
            "agents": agents,
            "total": agents.len(),
            "enabled": enabled_count,
        })
        .into())
    }
}

// ---------------------------------------------------------------------------
// RemoveAcpAgentTool
// ---------------------------------------------------------------------------

/// Tool that removes an ACP agent from the registry.
#[derive(ToolDef)]
#[tool(
    name = "remove-acp-agent",
    description = "Remove an ACP agent from the registry. Built-in agents cannot be removed.",
    params_schema = "Self::schema_remove()",
    execute_fn = "self.exec_remove"
)]
pub struct RemoveAcpAgentTool {
    registry: AcpRegistryRef,
}

impl RemoveAcpAgentTool {
    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }

    fn schema_remove() -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_name": {
                    "type": "string",
                    "description": "Name of the ACP agent to remove"
                }
            },
            "required": ["agent_name"]
        })
    }

    async fn exec_remove(
        &self,
        params: Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let agent_name = params
            .get("agent_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: agent_name"))?;

        let removed = self
            .registry
            .remove(agent_name)
            .await
            .map_err(|e| anyhow::anyhow!("failed to remove ACP agent '{agent_name}': {e}"))?;

        if removed {
            Ok(json!({
                "status": "removed",
                "agent_name": agent_name,
                "message": format!("ACP agent '{agent_name}' removed."),
            })
            .into())
        } else {
            Ok(json!({
                "status": "not_found",
                "agent_name": agent_name,
                "message": format!("ACP agent '{agent_name}' was not found in the registry."),
            })
            .into())
        }
    }
}
