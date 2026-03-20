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

//! Meta tool for deferred tool discovery.
//!
//! Returns matching deferred tools so the LLM can activate them. The actual
//! activation (adding names to the `activated_deferred` set) happens in the
//! agent loop after this tool returns.

use rara_kernel::tool::{ToolContext, ToolExecute, ToolRegistryRef};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;

/// Parameters for the discover-tools meta tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiscoverToolsParams {
    /// Keyword to search for in tool names and descriptions.
    /// Examples: "browser", "email", "skill", "dock", "mcp"
    query: String,
}

/// Discovers and activates deferred tools by keyword search.
#[derive(ToolDef)]
#[tool(
    name = "discover-tools",
    description = "Search for and activate additional tools not loaded by default. Use when you \
                   need capabilities beyond the core tools (e.g. browser navigation, email, \
                   skills management, dock canvas, MCP servers). Pass a keyword to search.",
    bypass_interceptor
)]
pub struct DiscoverToolsTool {
    registry: ToolRegistryRef,
}

impl DiscoverToolsTool {
    /// Create a new discover-tools instance backed by the given registry.
    pub fn new(registry: ToolRegistryRef) -> Self { Self { registry } }
}

#[async_trait::async_trait]
impl ToolExecute for DiscoverToolsTool {
    type Output = serde_json::Value;
    type Params = DiscoverToolsParams;

    #[tracing::instrument(skip_all)]
    async fn run(
        &self,
        params: Self::Params,
        _context: &ToolContext,
    ) -> anyhow::Result<Self::Output> {
        let query = params.query.to_lowercase();
        // TODO: pass actual activation state so already-activated tools are excluded.
        // For now, always show the full catalog — harmless since re-activation is a
        // no-op.
        let empty = std::collections::HashSet::new();
        let catalog = self.registry.deferred_catalog(&empty);

        let matches: Vec<_> = catalog
            .iter()
            .filter(|(name, desc)| {
                name.to_lowercase().contains(&query) || desc.to_lowercase().contains(&query)
            })
            .collect();

        if matches.is_empty() {
            // Build category hints dynamically from actual tool names.
            let mut categories: Vec<&str> = catalog
                .iter()
                .filter_map(|(name, _)| name.split('-').next())
                .collect();
            categories.sort_unstable();
            categories.dedup();
            categories.truncate(10);
            let hint = categories.join(", ");
            return Ok(serde_json::json!({
                "status": "no_matches",
                "tools": [],
                "message": format!(
                    "No deferred tools match '{query}'. Try one of: {hint}"
                ),
            }));
        }

        let activated: Vec<serde_json::Value> = matches
            .iter()
            .map(|(name, desc)| {
                serde_json::json!({
                    "name": name,
                    "description": desc,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "status": "activated",
            "tools": activated,
            "message": format!("Activated {} tool(s). They are now available for use.", activated.len()),
        }))
    }
}
