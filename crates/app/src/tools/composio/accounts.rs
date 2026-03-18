use rara_kernel::tool::{ToolContext, ToolOutput};
use rara_tool_macro::ToolDef;
use serde_json::json;

use super::shared::ComposioShared;

/// List OAuth-connected accounts on Composio.
#[derive(ToolDef)]
#[tool(
    name = "composio_accounts",
    description = "List connected OAuth accounts on Composio. Shows which third-party apps \
                   (Gmail, Notion, GitHub, etc.) have been authorized and their connection status.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub(super) struct ComposioAccountsTool {
    shared: ComposioShared,
}

impl ComposioAccountsTool {
    pub(super) fn new(shared: ComposioShared) -> Self { Self { shared } }

    fn schema() -> serde_json::Value {
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

    async fn exec(
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
