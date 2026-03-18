use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::shared::ComposioShared;

/// Input parameters for the composio_execute tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComposioExecuteParams {
    /// The action/tool identifier to execute (from composio_list results).
    tool_slug:            String,
    /// Parameters to pass to the action.
    params:               Option<Value>,
    /// App hint to help resolve the connected account (e.g. 'gmail', 'notion').
    app:                  Option<String>,
    /// Entity/user ID for multi-user setups (defaults to config value).
    entity_id:            Option<String>,
    /// Specific connected account ID (auto-resolved when omitted).
    connected_account_id: Option<String>,
}

/// Execute a Composio action on a connected app.
#[derive(ToolDef)]
#[tool(
    name = "composio_execute",
    description = "Execute an action on a connected app via Composio. Requires the tool_slug \
                   (from composio_list) and action parameters. The connected_account_id is \
                   auto-resolved when omitted."
)]
pub(super) struct ComposioExecuteTool {
    shared: ComposioShared,
}

impl ComposioExecuteTool {
    pub(super) fn new(shared: ComposioShared) -> Self { Self { shared } }
}

#[async_trait]
impl ToolExecute for ComposioExecuteTool {
    type Output = Value;
    type Params = ComposioExecuteParams;

    async fn run(
        &self,
        params: ComposioExecuteParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        let action_params = params.params.unwrap_or_else(|| json!({}));
        let entity_id = self
            .shared
            .resolve_entity_id(params.entity_id.as_deref())
            .await;

        match self
            .shared
            .client
            .execute_action(
                &params.tool_slug,
                params.app.as_deref(),
                action_params,
                Some(&entity_id),
                params.connected_account_id.as_deref(),
            )
            .await
        {
            Ok(result) => Ok(json!({ "result": result })),
            Err(error) => Ok(json!({ "error": format!("action execution failed: {error}") })),
        }
    }
}
