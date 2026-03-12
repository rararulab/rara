use async_trait::async_trait;
use rara_kernel::tool::{AgentTool, ToolContext, ToolOutput};
use serde_json::json;

use super::shared::ComposioShared;

/// List OAuth-connected accounts on Composio.
pub(super) struct ComposioAccountsTool {
    shared: ComposioShared,
}

impl ComposioAccountsTool {
    pub(super) fn new(shared: ComposioShared) -> Self { Self { shared } }
}

#[async_trait]
impl AgentTool for ComposioAccountsTool {
    fn name(&self) -> &str { "composio_accounts" }

    fn description(&self) -> &str {
        "List connected OAuth accounts on Composio. Shows which third-party apps \
         (Gmail, Notion, GitHub, etc.) have been authorized and their connection status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "app": {
                    "type": "string",
                    "description": "Filter by app, e.g. 'gmail', 'notion', 'github'"
                },
                "entity_id": {
                    "type": "string",
                    "description": "Entity/user ID for multi-user setups (defaults to config value)"
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
        let entity_id = self.shared.resolve_entity_id_async(&params).await;

        match self
            .shared
            .client
            .list_connected_accounts(app, Some(&entity_id))
            .await
        {
            Ok(accounts) => Ok(json!({
                "total": accounts.len(),
                "entity_id": entity_id,
                "accounts": accounts,
            })
            .into()),
            Err(error) => Ok(
                json!({ "error": format!("failed to list connected accounts: {error}") }).into(),
            ),
        }
    }
}
