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

//! Bridge between MCP server tools and the [`rara_kernel::tool::AgentTool`]
//! trait.
//!
//! Each [`McpToolBridge`] wraps a single MCP tool from a connected server and
//! implements [`AgentTool`] so it can be registered directly in a
//! `ToolRegistry`.

use anyhow::{Context, Result};
use async_trait::async_trait;
use rara_kernel::tool::{AgentTool, ToolOutput};
use tracing::instrument;

use crate::manager::mgr::McpManager;

/// Wraps a single MCP server tool as an [`AgentTool`].
pub struct McpToolBridge {
    server_name:  String,
    tool_name:    String,
    description:  String,
    input_schema: serde_json::Value,
    manager:      McpManager,
}

impl McpToolBridge {
    /// The name of the MCP server this tool belongs to.
    pub fn server_name(&self) -> &str { &self.server_name }

    /// Create bridges for all tools exposed by a specific server.
    #[instrument(skip(manager), fields(server = %server_name))]
    pub async fn from_server(manager: McpManager, server_name: &str) -> Result<Vec<Self>> {
        let tools = manager.list_tools(server_name).await.context(format!(
            "failed to list tools from MCP server '{server_name}'"
        ))?;

        Ok(tools
            .into_iter()
            .map(|t| Self {
                server_name:  server_name.to_owned(),
                tool_name:    t.name.to_string(),
                description:  t.description.as_deref().unwrap_or("").to_owned(),
                input_schema: serde_json::to_value(&*t.input_schema).unwrap_or_default(),
                manager:      manager.clone(),
            })
            .collect())
    }

    /// Create bridges for all tools from all connected MCP servers.
    #[instrument(skip(manager))]
    pub async fn from_manager(manager: McpManager) -> Result<Vec<Self>> {
        let servers = manager.connected_servers().await;
        let mut bridges = Vec::new();
        for server_name in servers {
            match Self::from_server(manager.clone(), &server_name).await {
                Ok(server_bridges) => bridges.extend(server_bridges),
                Err(e) => {
                    tracing::warn!(
                        server = %server_name,
                        error = %e,
                        "skipping MCP server tools due to error"
                    );
                }
            }
        }
        Ok(bridges)
    }
}

#[async_trait]
impl AgentTool for McpToolBridge {
    fn name(&self) -> &str { &self.tool_name }

    fn description(&self) -> &str { &self.description }

    fn parameters_schema(&self) -> serde_json::Value { self.input_schema.clone() }

    fn tier(&self) -> rara_kernel::tool::ToolTier { rara_kernel::tool::ToolTier::Deferred }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let result = self
            .manager
            .call_tool(&self.server_name, &self.tool_name, Some(params))
            .await
            .context(format!(
                "MCP call_tool failed: server={}, tool={}",
                self.server_name, self.tool_name
            ))?;

        // Convert CallToolResult to a JSON value.
        Ok(serde_json::to_value(&result)?.into())
    }
}
