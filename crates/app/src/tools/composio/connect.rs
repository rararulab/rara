use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::shared::ComposioShared;

/// Input parameters for the composio_connect tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComposioConnectParams {
    /// The app to connect, e.g. 'gmail', 'notion', 'github', 'slack'.
    app:            Option<String>,
    /// Specific Composio auth config ID (auto-resolved from app when omitted).
    auth_config_id: Option<String>,
    /// Entity/user ID for multi-user setups (defaults to config value).
    entity_id:      Option<String>,
}

/// Get an OAuth connection URL for a Composio-supported app.
#[derive(ToolDef)]
#[tool(
    name = "composio_connect",
    description = "Get an OAuth connection URL to authorize a third-party app via Composio. \
                   Returns a redirect URL the user should open in their browser to complete \
                   authorization."
)]
pub(super) struct ComposioConnectTool {
    shared: ComposioShared,
}

impl ComposioConnectTool {
    pub(super) fn new(shared: ComposioShared) -> Self { Self { shared } }
}

#[async_trait]
impl ToolExecute for ComposioConnectTool {
    type Output = Value;
    type Params = ComposioConnectParams;

    async fn run(
        &self,
        params: ComposioConnectParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        if params.app.is_none() && params.auth_config_id.is_none() {
            return Ok(json!({ "error": "provide 'app' (e.g. 'gmail') or 'auth_config_id'" }));
        }

        let entity_id = self
            .shared
            .resolve_entity_id(params.entity_id.as_deref())
            .await;

        match self
            .shared
            .client
            .get_connection_url(
                params.app.as_deref(),
                params.auth_config_id.as_deref(),
                &entity_id,
            )
            .await
        {
            Ok(link) => Ok(json!({
                "redirect_url": link.redirect_url,
                "connected_account_id": link.connected_account_id,
            })),
            Err(error) => Ok(json!({ "error": format!("failed to get connection URL: {error}") })),
        }
    }
}
