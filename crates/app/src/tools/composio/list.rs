use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::shared::ComposioShared;

/// Input parameters for the composio_list tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComposioListParams {
    /// App/toolkit to filter by, e.g. 'gmail', 'notion', 'github', 'slack'.
    app:   Option<String>,
    /// Search keyword to filter actions by name or description.
    query: Option<String>,
}

/// List available Composio actions, optionally filtered by app and search
/// query.
#[derive(ToolDef)]
#[tool(
    name = "composio_list",
    description = "List available actions/tools on Composio. Filter by app name (e.g. 'gmail', \
                   'notion', 'github') and/or search query to find specific actions.",
    tier = "deferred"
)]
pub(super) struct ComposioListTool {
    shared: ComposioShared,
}

impl ComposioListTool {
    pub(super) fn new(shared: ComposioShared) -> Self { Self { shared } }
}

#[async_trait]
impl ToolExecute for ComposioListTool {
    type Output = Value;
    type Params = ComposioListParams;

    async fn run(
        &self,
        params: ComposioListParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        let query = params.query.as_deref().map(|s| s.trim().to_lowercase());

        match self.shared.client.list_actions(params.app.as_deref()).await {
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
                }))
            }
            Err(error) => Ok(json!({ "error": format!("failed to list actions: {error}") })),
        }
    }
}
