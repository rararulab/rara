use async_trait::async_trait;
use rara_kernel::tool::{AgentTool, ToolContext, ToolOutput};
use serde_json::json;

use super::shared::ComposioShared;

/// List available Composio actions, optionally filtered by app and search
/// query.
pub(super) struct ComposioListTool {
    shared: ComposioShared,
}

impl ComposioListTool {
    pub(super) fn new(shared: ComposioShared) -> Self { Self { shared } }
}

#[async_trait]
impl AgentTool for ComposioListTool {
    fn name(&self) -> &str { "composio_list" }

    fn description(&self) -> &str {
        "List available actions/tools on Composio. Filter by app name (e.g. 'gmail', 'notion', \
         'github') and/or search query to find specific actions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "app": {
                    "type": "string",
                    "description": "App/toolkit to filter by, e.g. 'gmail', 'notion', 'github', 'slack'"
                },
                "query": {
                    "type": "string",
                    "description": "Search keyword to filter actions by name or description"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let app = params.get("app").and_then(|v| v.as_str());
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_lowercase());

        match self.shared.client.list_actions(app).await {
            Ok(mut actions) => {
                // Client-side query filter when provided
                if let Some(ref q) = query {
                    actions.retain(|a| {
                        a.name.to_lowercase().contains(q)
                            || a.description
                                .as_deref()
                                .is_some_and(|d| d.to_lowercase().contains(q))
                    });
                }
                Ok(json!({
                    "total": actions.len(),
                    "actions": actions,
                })
                .into())
            }
            Err(error) => Ok(json!({ "error": format!("failed to list actions: {error}") }).into()),
        }
    }
}
