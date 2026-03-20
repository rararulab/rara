//! ACP agent management tools — install, list, remove agents at runtime.

use std::collections::HashMap;

use async_trait::async_trait;
use rara_acp::registry::{AcpAgentConfig, AcpRegistryRef};
use rara_kernel::tool::{EmptyParams, ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// InstallAcpAgentTool
// ---------------------------------------------------------------------------

/// Parameters for installing a new ACP agent.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct InstallAcpAgentParams {
    /// Unique name for the ACP agent.
    agent_name: String,
    /// Command to run the ACP agent (e.g. 'npx', 'node', 'gemini').
    command:    String,
    /// Command-line arguments for the agent.
    #[serde(default)]
    args:       Vec<String>,
    /// Environment variables to pass to the agent process.
    #[serde(default)]
    env:        HashMap<String, String>,
}

/// Result of installing an ACP agent.
#[derive(Debug, Serialize)]
pub struct InstallAcpAgentResult {
    status:     String,
    agent_name: String,
    message:    String,
}

/// Tool that registers a new ACP agent in the registry.
#[derive(ToolDef)]
#[tool(
    name = "install-acp-agent",
    description = "Register a new ACP agent so it becomes available for delegation via \
                   acp-delegate. The agent is not started immediately — ACP agents are spawned on \
                   demand.",
    tier = "deferred"
)]
pub struct InstallAcpAgentTool {
    registry: AcpRegistryRef,
}

impl InstallAcpAgentTool {
    /// Create a new instance backed by the given agent registry.
    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for InstallAcpAgentTool {
    type Output = InstallAcpAgentResult;
    type Params = InstallAcpAgentParams;

    async fn run(
        &self,
        params: InstallAcpAgentParams,
        _context: &ToolContext,
    ) -> anyhow::Result<InstallAcpAgentResult> {
        let config = AcpAgentConfig {
            command: params.command,
            args: params.args,
            env: params.env,
            enabled: true,
            ..Default::default()
        };

        self.registry
            .add(params.agent_name.clone(), config)
            .await
            .map_err(|e| {
                anyhow::anyhow!("failed to install ACP agent '{}': {e}", params.agent_name)
            })?;

        Ok(InstallAcpAgentResult {
            status:     "installed".to_owned(),
            message:    format!(
                "ACP agent '{}' registered. Use acp-delegate to run it.",
                params.agent_name
            ),
            agent_name: params.agent_name,
        })
    }
}

// ---------------------------------------------------------------------------
// ListAcpAgentsTool
// ---------------------------------------------------------------------------

/// Summary of a single registered ACP agent.
#[derive(Debug, Serialize)]
pub struct AcpAgentInfo {
    name:    String,
    command: String,
    args:    Vec<String>,
    enabled: bool,
    builtin: bool,
}

/// Result of listing ACP agents.
#[derive(Debug, Serialize)]
pub struct ListAcpAgentsResult {
    agents:  Vec<AcpAgentInfo>,
    total:   usize,
    enabled: usize,
}

/// Tool that lists all registered ACP agents.
#[derive(ToolDef)]
#[tool(
    name = "list-acp-agents",
    description = "List all registered ACP agents with their status (enabled, builtin) and spawn \
                   command.",
    tier = "deferred"
)]
pub struct ListAcpAgentsTool {
    registry: AcpRegistryRef,
}

impl ListAcpAgentsTool {
    /// Create a new instance backed by the given agent registry.
    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for ListAcpAgentsTool {
    type Output = ListAcpAgentsResult;
    type Params = EmptyParams;

    async fn run(
        &self,
        _params: EmptyParams,
        _context: &ToolContext,
    ) -> anyhow::Result<ListAcpAgentsResult> {
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
            agents.push(AcpAgentInfo {
                name:    name.clone(),
                command: config.command.clone(),
                args:    config.args.clone(),
                enabled: config.enabled,
                builtin: config.builtin,
            });
        }

        Ok(ListAcpAgentsResult {
            total: agents.len(),
            enabled: enabled_count,
            agents,
        })
    }
}

// ---------------------------------------------------------------------------
// RemoveAcpAgentTool
// ---------------------------------------------------------------------------

/// Parameters for removing an ACP agent.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveAcpAgentParams {
    /// Name of the ACP agent to remove.
    agent_name: String,
}

/// Result of removing an ACP agent.
#[derive(Debug, Serialize)]
pub struct RemoveAcpAgentResult {
    status:     String,
    agent_name: String,
    message:    String,
}

/// Tool that removes an ACP agent from the registry.
#[derive(ToolDef)]
#[tool(
    name = "remove-acp-agent",
    description = "Remove an ACP agent from the registry. Built-in agents cannot be removed.",
    tier = "deferred"
)]
pub struct RemoveAcpAgentTool {
    registry: AcpRegistryRef,
}

impl RemoveAcpAgentTool {
    /// Create a new instance backed by the given agent registry.
    pub fn new(registry: AcpRegistryRef) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for RemoveAcpAgentTool {
    type Output = RemoveAcpAgentResult;
    type Params = RemoveAcpAgentParams;

    async fn run(
        &self,
        params: RemoveAcpAgentParams,
        _context: &ToolContext,
    ) -> anyhow::Result<RemoveAcpAgentResult> {
        let removed = self
            .registry
            .remove(&params.agent_name)
            .await
            .map_err(|e| {
                anyhow::anyhow!("failed to remove ACP agent '{}': {e}", params.agent_name)
            })?;

        if removed {
            Ok(RemoveAcpAgentResult {
                status:     "removed".to_owned(),
                message:    format!("ACP agent '{}' removed.", params.agent_name),
                agent_name: params.agent_name,
            })
        } else {
            Ok(RemoveAcpAgentResult {
                status:     "not_found".to_owned(),
                message:    format!(
                    "ACP agent '{}' was not found in the registry.",
                    params.agent_name
                ),
                agent_name: params.agent_name,
            })
        }
    }
}
