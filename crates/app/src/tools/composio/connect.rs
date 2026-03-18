use rara_kernel::tool::{ToolContext, ToolOutput};
use rara_tool_macro::ToolDef;
use serde_json::json;

use super::shared::ComposioShared;

/// Get an OAuth connection URL for a Composio-supported app.
#[derive(ToolDef)]
#[tool(
    name = "composio_connect",
    description = "Get an OAuth connection URL to authorize a third-party app via Composio. \
                   Returns a redirect URL the user should open in their browser to complete \
                   authorization.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub(super) struct ComposioConnectTool {
    shared: ComposioShared,
}

impl ComposioConnectTool {
    pub(super) fn new(shared: ComposioShared) -> Self { Self { shared } }

    fn schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "app": {
                    "type": "string",
                    "description": "The app to connect, e.g. 'gmail', 'notion', 'github', 'slack'"
                },
                "auth_config_id": {
                    "type": "string",
                    "description": "Specific Composio auth config ID (auto-resolved from app when omitted)"
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
        let auth_config_id = params.get("auth_config_id").and_then(|v| v.as_str());

        if app.is_none() && auth_config_id.is_none() {
            return Ok(
                json!({ "error": "provide 'app' (e.g. 'gmail') or 'auth_config_id'" }).into(),
            );
        }

        let entity_id = self.shared.resolve_entity_id_async(&params).await;

        match self
            .shared
            .client
            .get_connection_url(app, auth_config_id, &entity_id)
            .await
        {
            Ok(link) => Ok(json!({
                "redirect_url": link.redirect_url,
                "connected_account_id": link.connected_account_id,
            })
            .into()),
            Err(error) => {
                Ok(json!({ "error": format!("failed to get connection URL: {error}") }).into())
            }
        }
    }
}
