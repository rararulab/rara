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

//! Layer 2 service tools for managing MCP servers at runtime.

use std::collections::HashMap;

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_mcp::manager::{
    mgr::McpManager,
    registry::{McpServerConfig, TransportType},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// InstallMcpServerTool
// ---------------------------------------------------------------------------

/// Input parameters for the install-mcp-server tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct InstallMcpServerParams {
    /// Unique name for the MCP server.
    server_name: String,
    /// Command to run the MCP server (e.g. 'npx', 'uvx', 'node').
    command:     String,
    /// Command-line arguments for the server.
    args:        Option<Vec<String>>,
    /// Environment variables to pass to the server process.
    env:         Option<HashMap<String, String>>,
    /// Transport type (default: stdio).
    transport:   Option<String>,
    /// URL for SSE transport (required when transport is 'sse').
    url:         Option<String>,
}

/// Tool that installs (adds + starts) a new MCP server at runtime.
#[derive(ToolDef)]
#[tool(
    name = "install-mcp-server",
    description = "Install and start an MCP server. The server's tools become available \
                   immediately for subsequent agent runs without restart.",
    bypass_interceptor
)]
pub struct InstallMcpServerTool {
    manager: McpManager,
}

impl InstallMcpServerTool {
    pub fn new(manager: McpManager) -> Self { Self { manager } }
}

#[async_trait]
impl ToolExecute for InstallMcpServerTool {
    type Output = Value;
    type Params = InstallMcpServerParams;

    async fn run(
        &self,
        params: InstallMcpServerParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        let transport = match params.transport.as_deref() {
            Some("sse") => TransportType::Sse,
            _ => TransportType::Stdio,
        };

        let config = McpServerConfig {
            command: params.command,
            args: params.args.unwrap_or_default(),
            env: params.env.unwrap_or_default(),
            enabled: true,
            transport,
            url: params.url,
            ..Default::default()
        };

        self.manager
            .add_server(params.server_name.clone(), config, true)
            .await
            .map_err(|e| {
                anyhow::anyhow!("failed to install MCP server '{}': {e}", params.server_name)
            })?;

        Ok(json!({
            "status": "installed",
            "server_name": params.server_name,
            "message": format!("MCP server '{}' installed and started. Its tools are now available.", params.server_name),
        }))
    }
}

// ---------------------------------------------------------------------------
// ListMcpServersTool
// ---------------------------------------------------------------------------

/// Input parameters for the list-mcp-servers tool (no parameters required).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMcpServersParams {}

/// Tool that lists all registered MCP servers and their tools.
#[derive(ToolDef)]
#[tool(
    name = "list-mcp-servers",
    description = "List all registered MCP servers with their status (enabled, connected) and \
                   available tools.",
    bypass_interceptor
)]
pub struct ListMcpServersTool {
    manager: McpManager,
}

impl ListMcpServersTool {
    pub fn new(manager: McpManager) -> Self { Self { manager } }
}

#[async_trait]
impl ToolExecute for ListMcpServersTool {
    type Output = Value;
    type Params = ListMcpServersParams;

    async fn run(
        &self,
        _params: ListMcpServersParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        let registry = self.manager.registry().await;
        let all_names = registry
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("failed to list MCP servers: {e}"))?;
        let connected = self.manager.connected_servers().await;

        let mut servers = Vec::new();
        for name in &all_names {
            let config = registry.get(name).await.ok().flatten();
            let is_connected = connected.contains(name);
            let enabled = config.as_ref().map(|c| c.enabled).unwrap_or(false);

            let tools: Vec<Value> = if is_connected {
                match self.manager.list_tools(name).await {
                    Ok(tool_list) => tool_list
                        .iter()
                        .map(|t| {
                            json!({
                                "name": t.name.to_string(),
                                "description": t.description.as_deref().unwrap_or(""),
                            })
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                }
            } else {
                Vec::new()
            };

            servers.push(json!({
                "name": name,
                "enabled": enabled,
                "connected": is_connected,
                "transport": config.as_ref().map(|c| format!("{:?}", c.transport).to_lowercase()),
                "command": config.as_ref().map(|c| &c.command),
                "tools": tools,
                "tool_count": tools.len(),
            }));
        }

        let total = servers.len();
        let connected_count = servers
            .iter()
            .filter(|s| {
                s.get("connected")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .count();

        Ok(json!({
            "servers": servers,
            "total": total,
            "connected": connected_count,
        }))
    }
}

// ---------------------------------------------------------------------------
// RemoveMcpServerTool
// ---------------------------------------------------------------------------

/// Input parameters for the remove-mcp-server tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveMcpServerParams {
    /// Name of the MCP server to remove.
    server_name: String,
}

/// Tool that removes an MCP server from the registry and stops it.
#[derive(ToolDef)]
#[tool(
    name = "remove-mcp-server",
    description = "Remove an MCP server from the registry and stop it. Its tools will no longer \
                   be available.",
    bypass_interceptor
)]
pub struct RemoveMcpServerTool {
    manager: McpManager,
}

impl RemoveMcpServerTool {
    pub fn new(manager: McpManager) -> Self { Self { manager } }
}

#[async_trait]
impl ToolExecute for RemoveMcpServerTool {
    type Output = Value;
    type Params = RemoveMcpServerParams;

    async fn run(
        &self,
        params: RemoveMcpServerParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        let removed = self
            .manager
            .remove_server(&params.server_name)
            .await
            .map_err(|e| {
                anyhow::anyhow!("failed to remove MCP server '{}': {e}", params.server_name)
            })?;

        if removed {
            Ok(json!({
                "status": "removed",
                "server_name": params.server_name,
                "message": format!("MCP server '{}' removed and stopped.", params.server_name),
            }))
        } else {
            Ok(json!({
                "status": "not_found",
                "server_name": params.server_name,
                "message": format!("MCP server '{}' was not found in the registry.", params.server_name),
            }))
        }
    }
}
