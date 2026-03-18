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

use rara_kernel::tool::{ToolContext, ToolOutput};
use rara_mcp::manager::{
    mgr::McpManager,
    registry::{McpServerConfig, TransportType},
};
use rara_tool_macro::ToolDef;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// InstallMcpServerTool
// ---------------------------------------------------------------------------

/// Tool that installs (adds + starts) a new MCP server at runtime.
#[derive(ToolDef)]
#[tool(
    name = "install-mcp-server",
    description = "Install and start an MCP server. The server's tools become available \
                   immediately for subsequent agent runs without restart.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct InstallMcpServerTool {
    manager: McpManager,
}

impl InstallMcpServerTool {
    pub fn new(manager: McpManager) -> Self { Self { manager } }

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "server_name": {
                    "type": "string",
                    "description": "Unique name for the MCP server"
                },
                "command": {
                    "type": "string",
                    "description": "Command to run the MCP server (e.g. 'npx', 'uvx', 'node')"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Command-line arguments for the server"
                },
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Environment variables to pass to the server process"
                },
                "transport": {
                    "type": "string",
                    "enum": ["stdio", "sse"],
                    "description": "Transport type (default: stdio)"
                },
                "url": {
                    "type": "string",
                    "description": "URL for SSE transport (required when transport is 'sse')"
                }
            },
            "required": ["server_name", "command"]
        })
    }

    async fn exec(&self, params: Value, _context: &ToolContext) -> anyhow::Result<ToolOutput> {
        let server_name = params
            .get("server_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: server_name"))?;

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

        let transport = match params.get("transport").and_then(|v| v.as_str()) {
            Some("sse") => TransportType::Sse,
            _ => TransportType::Stdio,
        };

        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);

        let config = McpServerConfig {
            command: command.to_owned(),
            args,
            env,
            enabled: true,
            transport,
            url,
            ..Default::default()
        };

        self.manager
            .add_server(server_name.to_owned(), config, true)
            .await
            .map_err(|e| anyhow::anyhow!("failed to install MCP server '{server_name}': {e}"))?;

        Ok(json!({
            "status": "installed",
            "server_name": server_name,
            "message": format!("MCP server '{server_name}' installed and started. Its tools are now available."),
        }).into())
    }
}

// ---------------------------------------------------------------------------
// ListMcpServersTool
// ---------------------------------------------------------------------------

/// Tool that lists all registered MCP servers and their tools.
#[derive(ToolDef)]
#[tool(
    name = "list-mcp-servers",
    description = "List all registered MCP servers with their status (enabled, connected) and \
                   available tools.",
    params_schema = "Self::schema_list()",
    execute_fn = "self.exec_list"
)]
pub struct ListMcpServersTool {
    manager: McpManager,
}

impl ListMcpServersTool {
    pub fn new(manager: McpManager) -> Self { Self { manager } }

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
        })
        .into())
    }
}

// ---------------------------------------------------------------------------
// RemoveMcpServerTool
// ---------------------------------------------------------------------------

/// Tool that removes an MCP server from the registry and stops it.
#[derive(ToolDef)]
#[tool(
    name = "remove-mcp-server",
    description = "Remove an MCP server from the registry and stop it. Its tools will no longer \
                   be available.",
    params_schema = "Self::schema_remove()",
    execute_fn = "self.exec_remove"
)]
pub struct RemoveMcpServerTool {
    manager: McpManager,
}

impl RemoveMcpServerTool {
    pub fn new(manager: McpManager) -> Self { Self { manager } }

    fn schema_remove() -> Value {
        json!({
            "type": "object",
            "properties": {
                "server_name": {
                    "type": "string",
                    "description": "Name of the MCP server to remove"
                }
            },
            "required": ["server_name"]
        })
    }

    async fn exec_remove(
        &self,
        params: Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let server_name = params
            .get("server_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: server_name"))?;

        let removed = self
            .manager
            .remove_server(server_name)
            .await
            .map_err(|e| anyhow::anyhow!("failed to remove MCP server '{server_name}': {e}"))?;

        if removed {
            Ok(json!({
                "status": "removed",
                "server_name": server_name,
                "message": format!("MCP server '{server_name}' removed and stopped."),
            })
            .into())
        } else {
            Ok(json!({
                "status": "not_found",
                "server_name": server_name,
                "message": format!("MCP server '{server_name}' was not found in the registry."),
            })
            .into())
        }
    }
}
