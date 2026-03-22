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

use rara_kernel::tool::{DiscoverToolsResult, DiscoveredToolEntry, ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;

/// Parameters for the discover-tools meta tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiscoverToolsParams {
    /// Keyword to search for in tool names and descriptions.
    /// Examples: "email", "skill", "dock", "mcp"
    query: String,
}

/// Discovers and activates deferred tools by keyword search.
///
/// Reads the live tool registry from [`ToolContext`] at query time, so
/// dynamically registered tools (e.g. MCP servers connected after boot) are
/// always visible in the catalog.
#[derive(ToolDef)]
#[tool(
    name = "discover-tools",
    description = "Search for and activate additional tools not loaded by default. Use when you \
                   need capabilities beyond the core tools (e.g. email, skills management, dock \
                   canvas, MCP servers). Pass a keyword to search."
)]
pub struct DiscoverToolsTool;

impl DiscoverToolsTool {
    /// Create a new discover-tools instance.
    pub fn new() -> Self { Self }
}

#[async_trait::async_trait]
impl ToolExecute for DiscoverToolsTool {
    type Output = serde_json::Value;
    type Params = DiscoverToolsParams;

    #[tracing::instrument(skip_all)]
    async fn run(
        &self,
        params: Self::Params,
        context: &ToolContext,
    ) -> anyhow::Result<Self::Output> {
        let registry = context
            .tool_registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("tool registry not available in context"))?;

        let query = params.query.to_lowercase();
        // TODO: pass actual activation state so already-activated tools are excluded.
        // For now, always show the full catalog — harmless since re-activation is a
        // no-op.
        let empty = std::collections::HashSet::new();
        let catalog = registry.deferred_catalog(&empty);

        let matches: Vec<_> = catalog
            .iter()
            .filter(|(name, desc)| {
                name.to_lowercase().contains(&query) || desc.to_lowercase().contains(&query)
            })
            .collect();

        if matches.is_empty() {
            // Extract category prefixes using any of `-`, `.`, `_` as separators.
            let mut categories: Vec<&str> = catalog
                .iter()
                .filter_map(|(name, _)| name.find(['-', '.', '_']).map(|pos| &name[..pos]))
                .collect();
            categories.sort_unstable();
            categories.dedup();
            categories.truncate(10);
            let hint = categories.join(", ");
            let result = DiscoverToolsResult {
                status:  "no_matches".to_string(),
                tools:   vec![],
                message: format!("No deferred tools match '{query}'. Try one of: {hint}"),
            };
            return serde_json::to_value(&result).map_err(Into::into);
        }

        let tools: Vec<DiscoveredToolEntry> = matches
            .iter()
            .map(|(name, desc)| DiscoveredToolEntry {
                name:        name.to_string(),
                description: desc.to_string(),
            })
            .collect();
        let count = tools.len();
        let result = DiscoverToolsResult {
            status: "activated".to_string(),
            tools,
            message: format!("Activated {count} tool(s). They are now available for use."),
        };
        serde_json::to_value(&result).map_err(Into::into)
    }
}
