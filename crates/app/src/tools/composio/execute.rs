use rara_kernel::tool::{ToolContext, ToolOutput};
use rara_tool_macro::ToolDef;
use serde_json::json;

use super::shared::ComposioShared;

/// Execute a Composio action on a connected app.
#[derive(ToolDef)]
#[tool(
    name = "composio_execute",
    description = "Execute an action on a connected app via Composio. Requires the tool_slug \
                   (from composio_list) and action parameters. The connected_account_id is \
                   auto-resolved when omitted.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub(super) struct ComposioExecuteTool {
    shared: ComposioShared,
}

impl ComposioExecuteTool {
    pub(super) fn new(shared: ComposioShared) -> Self { Self { shared } }

    fn schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "tool_slug": {
                    "type": "string",
                    "description": "The action/tool identifier to execute (from composio_list results)"
                },
                "params": {
                    "type": "object",
                    "description": "Parameters to pass to the action"
                },
                "app": {
                    "type": "string",
                    "description": "App hint to help resolve the connected account (e.g. 'gmail', 'notion')"
                },
                "entity_id": {
                    "type": "string",
                    "description": "Entity/user ID for multi-user setups (defaults to config value)"
                },
                "connected_account_id": {
                    "type": "string",
                    "description": "Specific connected account ID (auto-resolved when omitted)"
                }
            },
            "required": ["tool_slug"]
        })
    }

    async fn exec(
        &self,
        params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let tool_slug = params
            .get("tool_slug")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: tool_slug"))?;
        let action_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        let app = params.get("app").and_then(|v| v.as_str());
        let entity_id = self.shared.resolve_entity_id_async(&params).await;
        let connected_account_id = params.get("connected_account_id").and_then(|v| v.as_str());

        match self
            .shared
            .client
            .execute_action(
                tool_slug,
                app,
                action_params,
                Some(&entity_id),
                connected_account_id,
            )
            .await
        {
            Ok(result) => Ok(json!({ "result": result }).into()),
            Err(error) => {
                Ok(json!({ "error": format!("action execution failed: {error}") }).into())
            }
        }
    }
}
