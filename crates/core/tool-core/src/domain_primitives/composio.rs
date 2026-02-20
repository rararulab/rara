// Copyright 2025 Crrow
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

//! Composio primitive.
//!
//! This wrapper keeps tool-facing concerns in `tool-core` while delegating all
//! HTTP API behavior to the `rara-composio` client crate.

use std::sync::Arc;

use async_trait::async_trait;
use rara_composio::{ComposioAuthProvider, ComposioClient};
use serde_json::json;

use crate::AgentTool;

/// Layer 1 primitive: call Composio managed tools.
pub struct ComposioTool {
    client: ComposioClient,
}

impl ComposioTool {
    /// Create the primitive wrapper from an already configured Composio client.
    pub fn new(client: ComposioClient) -> Self { Self { client } }

    /// Build the Composio tool from an injected auth provider.
    pub fn from_auth_provider(auth_provider: Arc<dyn ComposioAuthProvider>) -> Self {
        Self::new(ComposioClient::with_auth_provider(auth_provider))
    }
}

#[async_trait]
impl AgentTool for ComposioTool {
    fn name(&self) -> &str { "composio" }

    fn description(&self) -> &str {
        "Execute actions on apps via Composio. Supports list, list_accounts, execute, and connect."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Operation to perform",
                    "enum": ["list", "list_accounts", "connected_accounts", "execute", "connect"]
                },
                "app": {
                    "type": "string",
                    "description": "Toolkit/app slug (e.g. gmail, notion, github)"
                },
                "action_name": {
                    "type": "string",
                    "description": "Action/tool identifier to execute"
                },
                "tool_slug": {
                    "type": "string",
                    "description": "Preferred v3 tool slug to execute (alias of action_name)"
                },
                "params": {
                    "type": "object",
                    "description": "Parameters passed to the Composio action"
                },
                "entity_id": {
                    "type": "string",
                    "description": "Entity/user ID (defaults to COMPOSIO_ENTITY_ID or 'default')"
                },
                "auth_config_id": {
                    "type": "string",
                    "description": "Optional Composio auth config id for connect flow"
                },
                "connected_account_id": {
                    "type": "string",
                    "description": "Optional connected account ID for execute flow"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;
        let app = params.get("app").and_then(|v| v.as_str());
        let entity_id = params.get("entity_id").and_then(|v| v.as_str());
        let resolved_entity_id = match entity_id {
            Some(value) => value.to_owned(),
            None => self
                .client
                .default_entity_id()
                .await
                .unwrap_or_else(|_| "default".to_owned()),
        };

        match action {
            "list" => match self.client.list_actions(app).await {
                Ok(actions) => Ok(json!({
                    "total": actions.len(),
                    "actions": actions,
                })),
                Err(error) => Ok(json!({ "error": format!("failed to list actions: {error}") })),
            },
            "list_accounts" | "connected_accounts" => {
                match self
                    .client
                    .list_connected_accounts(app, Some(&resolved_entity_id))
                    .await
                {
                    Ok(accounts) => Ok(json!({
                        "total": accounts.len(),
                        "entity_id": resolved_entity_id,
                        "accounts": accounts,
                    })),
                    Err(error) => Ok(json!({
                        "error": format!("failed to list connected accounts: {error}")
                    })),
                }
            }
            "execute" => {
                let action_name = params
                    .get("tool_slug")
                    .or_else(|| params.get("action_name"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("missing required parameter: action_name or tool_slug")
                    })?;
                let action_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
                let connected_account_id =
                    params.get("connected_account_id").and_then(|v| v.as_str());

                match self
                    .client
                    .execute_action(
                        action_name,
                        app,
                        action_params,
                        Some(&resolved_entity_id),
                        connected_account_id,
                    )
                    .await
                {
                    Ok(result) => Ok(json!({ "result": result })),
                    Err(error) => {
                        Ok(json!({ "error": format!("action execution failed: {error}") }))
                    }
                }
            }
            "connect" => {
                let auth_config_id = params.get("auth_config_id").and_then(|v| v.as_str());
                if app.is_none() && auth_config_id.is_none() {
                    return Ok(json!({
                        "error": "missing required parameter: app or auth_config_id"
                    }));
                }

                match self
                    .client
                    .get_connection_url(app, auth_config_id, &resolved_entity_id)
                    .await
                {
                    Ok(link) => Ok(json!({
                        "redirect_url": link.redirect_url,
                        "connected_account_id": link.connected_account_id,
                    })),
                    Err(error) => {
                        Ok(json!({ "error": format!("failed to get connection URL: {error}") }))
                    }
                }
            }
            other => Ok(json!({
                "error": format!(
                    "unknown action '{other}'. Use list, list_accounts, execute, or connect."
                ),
            })),
        }
    }
}
