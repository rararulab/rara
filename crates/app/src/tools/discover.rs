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
    DiscoverToolsResult, DiscoveredSkillEntry, DiscoveredToolEntry, ToolContext, ToolExecute,
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
        let has_registry = context.tool_registry.is_some();
        let catalog = context
            .tool_registry
            .as_ref()
            .map(|registry| registry.deferred_catalog(&empty))
            .unwrap_or_default();
        tracing::debug!(
            %query,
            has_registry,
            catalog_size = catalog.len(),
            "discover-tools: deferred catalog state"
        );

        let tool_matches: Vec<DiscoveredToolEntry> = catalog
            .iter()
            .filter(|(name, desc)| {
                name.to_lowercase().contains(&query) || desc.to_lowercase().contains(&query)
            })
            .map(|(name, desc)| DiscoveredToolEntry {
                name:        name.to_string(),
                description: desc.to_string(),
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
                status:  "no_matches".to_string(),
                tools:   vec![],
                skills:  vec![],
                message: format!("No tools or skills match '{query}'. Try: {hint}"),
            };
            return serde_json::to_value(&result).map_err(Into::into);
        }

        let mut parts = Vec::new();
        if tool_count > 0 {
            parts.push(format!("{tool_count} tool(s) activated"));
        }
        if skill_count > 0 {
            parts.push(format!(
                "{skill_count} skill(s) found — read SKILL.md at the returned path to use"
            ));
        }
        let result = DiscoverToolsResult {
            status:  if tool_count > 0 {
                "activated".to_string()
            } else {
                "skills_only".to_string()
            },
            tools:   tool_matches,
            skills:  skill_matches,
            message: format!("{}.", parts.join("; ")),
        };
        serde_json::to_value(&result).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rara_kernel::tool::{AgentTool, ToolContext, ToolOutput, ToolRegistry, ToolTier};

    use super::*;

    /// Minimal deferred tool for testing.
    struct FakeDeferred;
    #[async_trait::async_trait]
    impl AgentTool for FakeDeferred {
        fn name(&self) -> &str { "marketplace" }

        fn description(&self) -> &str { "Manage skills from clawhub.ai." }

        fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({}) }

        async fn execute(
            &self,
            _: serde_json::Value,
            _: &ToolContext,
        ) -> anyhow::Result<ToolOutput> {
            unimplemented!()
        }

        fn tier(&self) -> ToolTier { ToolTier::Deferred }
    }

    fn make_context_with_deferred() -> ToolContext {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(FakeDeferred));
        let queue = rara_kernel::queue::ShardedEventQueue::new(
            rara_kernel::queue::ShardedEventQueueConfig::default(),
        );
        ToolContext {
            user_id:               "test".to_string(),
            session_key:           rara_kernel::session::SessionKey::new(),
            origin_endpoint:       None,
            event_queue:           Arc::new(queue),
            rara_message_id:       rara_kernel::io::MessageId::new(),
            context_window_tokens: 0,
            tool_registry:         Some(Arc::new(reg)),
            stream_handle:         None,
            tool_call_id:          None,
        }
    }

    #[tokio::test]
    async fn discover_finds_deferred_marketplace() {
        let tool = DiscoverToolsTool::new(InMemoryRegistry::new());
        let ctx = make_context_with_deferred();
        let params = DiscoverToolsParams {
            query: "marketplace".to_string(),
        };
        let result = tool.run(params, &ctx).await.unwrap();
        let parsed: DiscoverToolsResult = serde_json::from_value(result).unwrap();
        assert_eq!(
            parsed.status, "activated",
            "expected activated, got: {}",
            parsed.status
        );
        assert!(
            parsed.tools.iter().any(|t| t.name == "marketplace"),
            "marketplace not found in: {:?}",
            parsed.tools
        );
    }

    #[tokio::test]
    async fn discover_empty_query_returns_all_deferred() {
        let tool = DiscoverToolsTool::new(InMemoryRegistry::new());
        let ctx = make_context_with_deferred();
        let params = DiscoverToolsParams {
            query: String::new(),
        };
        let result = tool.run(params, &ctx).await.unwrap();
        let parsed: DiscoverToolsResult = serde_json::from_value(result).unwrap();
        assert_eq!(parsed.status, "activated");
        assert_eq!(parsed.tools.len(), 1);
    }

    #[tokio::test]
    async fn discover_no_registry_returns_no_tools() {
        let tool = DiscoverToolsTool::new(InMemoryRegistry::new());
        let mut ctx = make_context_with_deferred();
        ctx.tool_registry = None;
        let params = DiscoverToolsParams {
            query: "marketplace".to_string(),
        };
        let result = tool.run(params, &ctx).await.unwrap();
        let parsed: DiscoverToolsResult = serde_json::from_value(result).unwrap();
        assert_eq!(parsed.status, "no_matches");
        assert!(parsed.tools.is_empty());
    }
}
