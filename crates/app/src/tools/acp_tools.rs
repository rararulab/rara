//! ACP agent management tools — install, list, remove agents at runtime.

use std::collections::HashMap;

use async_trait::async_trait;
use rara_acp::registry::{AcpAgentConfig, AcpRegistryRef};
use rara_kernel::tool::{AgentTool, ToolOutput};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// InstallAcpAgentTool
// ---------------------------------------------------------------------------

/// Tool that registers a new ACP agent in the registry.
pub struct InstallAcpAgentTool {
    registry: AcpRegistryRef,
}

impl InstallAcpAgentTool {
    pub const NAME: &str = "install-acp-agent";

    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }
}

#[async_trait]
impl AgentTool for InstallAcpAgentTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Register a new ACP agent so it becomes available for delegation via acp-delegate. The \
         agent is not started immediately — ACP agents are spawned on demand."
    }

    fn parameters_schema(&self) -> Value {
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

    async fn execute(
        &self,
        params: Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<ToolOutput> {
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
                "ACP agent '{agent_name}' registered. It can now be used with the acp-delegate tool."
            ),
        })
        .into())
    }
}

// ---------------------------------------------------------------------------
// ListAcpAgentsTool
// ---------------------------------------------------------------------------

/// Tool that lists all registered ACP agents.
pub struct ListAcpAgentsTool {
    registry: AcpRegistryRef,
}

impl ListAcpAgentsTool {
    pub const NAME: &str = "list-acp-agents";

    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }
}

#[async_trait]
impl AgentTool for ListAcpAgentsTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "List all registered ACP agents with their status (enabled, builtin) and spawn command."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let all_names = self
            .registry
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("failed to list ACP agents: {e}"))?;

        let mut agents = Vec::new();
        for name in &all_names {
            if let Ok(Some(config)) = self.registry.get(name).await {
                agents.push(json!({
                    "name": name,
                    "command": config.command,
                    "args": config.args,
                    "enabled": config.enabled,
                    "builtin": config.builtin,
                }));
            }
        }

        let total = agents.len();
        let enabled_count = agents
            .iter()
            .filter(|a| a.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false))
            .count();

        Ok(json!({
            "agents": agents,
            "total": total,
            "enabled": enabled_count,
        })
        .into())
    }
}

// ---------------------------------------------------------------------------
// RemoveAcpAgentTool
// ---------------------------------------------------------------------------

/// Tool that removes an ACP agent from the registry.
pub struct RemoveAcpAgentTool {
    registry: AcpRegistryRef,
}

impl RemoveAcpAgentTool {
    pub const NAME: &str = "remove-acp-agent";

    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }
}

#[async_trait]
impl AgentTool for RemoveAcpAgentTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Remove an ACP agent from the registry. Built-in agents cannot be removed."
    }

    fn parameters_schema(&self) -> Value {
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

    async fn execute(
        &self,
        params: Value,
        _context: &rara_kernel::tool::ToolContext,
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
