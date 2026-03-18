use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::shared::ComposioShared;

/// Input parameters for the composio_accounts tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComposioAccountsParams {
    /// Filter by app, e.g. 'gmail', 'notion', 'github'.
    app:       Option<String>,
    /// Entity/user ID for multi-user setups (defaults to config value).
    entity_id: Option<String>,
}

/// List OAuth-connected accounts on Composio.
#[derive(ToolDef)]
#[tool(
    name = "composio_accounts",
    description = "List connected OAuth accounts on Composio. Shows which third-party apps \
                   (Gmail, Notion, GitHub, etc.) have been authorized and their connection status."
)]
pub(super) struct ComposioAccountsTool {
    shared: ComposioShared,
}

impl ComposioAccountsTool {
    pub(super) fn new(shared: ComposioShared) -> Self { Self { shared } }
}

#[async_trait]
impl ToolExecute for ComposioAccountsTool {
    type Output = Value;
    type Params = ComposioAccountsParams;

    async fn run(
        &self,
        params: ComposioAccountsParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        let entity_id = self
            .shared
            .resolve_entity_id(params.entity_id.as_deref())
            .await;

        match self
            .shared
            .client
            .list_connected_accounts(params.app.as_deref(), Some(&entity_id))
            .await
        {
            Ok(accounts) => Ok(json!({
                "total": accounts.len(),
                "entity_id": entity_id,
                "accounts": accounts,
            })),
            Err(error) => {
                Ok(json!({ "error": format!("failed to list connected accounts: {error}") }))
            }
        }
    }
}
