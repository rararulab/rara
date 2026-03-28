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

//! Meta tool for deferred tool and skill discovery.
//!
//! Searches both the deferred tool catalog and the skill registry by keyword,
//! returning matching entries so the LLM can activate tools or read skill
//! instructions.

use rara_kernel::tool::{
    DiscoverToolsResult, DiscoverToolsStatus, DiscoveredSkillEntry, DiscoveredToolEntry,
    ToolContext, ToolExecute, summarize_parameters,
};
use rara_skills::registry::InMemoryRegistry;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;

/// Parameters for the discover-tools meta tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiscoverToolsParams {
    /// Plain keyword to search for in tool/skill names and descriptions.
    /// Must be a simple word — do NOT include JSON syntax or special
    /// characters. Examples: "schedule", "email", "browser", "memory",
    /// "skill"
    query: String,
}

/// Discovers deferred tools and skills by keyword search.
///
/// Reads the live tool registry from [`ToolContext`] and the skill registry
/// stored on this struct, so dynamically registered entries are always visible.
#[derive(ToolDef)]
#[tool(
    name = "discover-tools",
    description = "Search for and activate additional tools or find available skills; pass a \
                   keyword to search. Activated tools become available immediately. For skills, \
                   read the SKILL.md at the returned path."
)]
pub struct DiscoverToolsTool {
    skill_registry: InMemoryRegistry,
}

impl DiscoverToolsTool {
    /// Create a new discover-tools instance with access to the skill registry.
    pub fn new(skill_registry: InMemoryRegistry) -> Self { Self { skill_registry } }
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
        // Strip stray JSON syntax characters that LLMs sometimes leak into the
        // query (e.g. "}schedule" instead of "schedule").
        let query = params
            .query
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
            .to_lowercase();

        // --- Search deferred tools ---
        // TODO: pass actual activation state so already-activated tools are excluded.
        // For now, always show the full catalog — harmless since re-activation is a
        // no-op.
        let empty = std::collections::HashSet::new();
        let catalog = context
            .tool_registry
            .as_ref()
            .map(|registry| registry.deferred_catalog(&empty))
            .unwrap_or_default();

        let tool_matches: Vec<DiscoveredToolEntry> = catalog
            .iter()
            .filter(|(name, desc)| {
                name.to_lowercase().contains(&query) || desc.to_lowercase().contains(&query)
            })
            .map(|(name, desc)| {
                let parameters = context
                    .tool_registry
                    .as_ref()
                    .and_then(|reg| reg.get(name))
                    .map(|tool| summarize_parameters(&tool.parameters_schema()))
                    .unwrap_or_default();
                DiscoveredToolEntry {
                    name: name.to_string(),
                    description: desc.to_string(),
                    parameters,
                }
            })
            .collect();

        // --- Search skills ---
        let skill_matches: Vec<DiscoveredSkillEntry> = self
            .skill_registry
            .list_all()
            .into_iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&query)
                    || s.description.to_lowercase().contains(&query)
            })
            .map(|s| DiscoveredSkillEntry {
                name:        s.name,
                description: s.description,
                path:        s.path.display().to_string(),
            })
            .collect();

        let tool_count = tool_matches.len();
        let skill_count = skill_matches.len();

        if tool_count == 0 && skill_count == 0 {
            let mut cats: Vec<&str> = catalog
                .iter()
                .filter_map(|(name, _)| name.find(['-', '.', '_']).map(|pos| &name[..pos]))
                .collect();
            cats.sort_unstable();
            cats.dedup();
            cats.truncate(10);
            let hint = cats.join(", ");
            let result = DiscoverToolsResult {
                status:  DiscoverToolsStatus::NoMatches,
                tools:   vec![],
                skills:  vec![],
                message: format!("No tools or skills match '{query}'. Try: {hint}"),
            };
            return serde_json::to_value(&result).map_err(Into::into);
        }

        let mut parts = Vec::new();
        if tool_count > 0 {
            parts.push(format!(
                "{tool_count} tool(s) activated — call them directly now using the parameters \
                 shown"
            ));
        }
        if skill_count > 0 {
            parts.push(format!(
                "{skill_count} skill(s) found — read SKILL.md at the returned path to use"
            ));
        }
        let result = DiscoverToolsResult {
            status:  if tool_count > 0 {
                DiscoverToolsStatus::Activated
            } else {
                DiscoverToolsStatus::SkillsOnly
            },
            tools:   tool_matches,
            skills:  skill_matches,
            message: format!("{}.", parts.join("; ")),
        };
        serde_json::to_value(&result).map_err(Into::into)
    }
}
