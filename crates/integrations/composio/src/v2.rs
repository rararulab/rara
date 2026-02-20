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

use anyhow::Context;
use serde_json::json;

use super::{
    ComposioAction, ComposioActionsResponse, ComposioApi, ComposioAuth, ComposioClientInner,
    ComposioConnectionLink, V2, extract_connected_account_id, extract_redirect_url, response_error,
};

const COMPOSIO_API_BASE_V2: &str = "https://backend.composio.dev/api/v2";

impl ComposioApi<V2> {
    pub(super) async fn list_actions(
        &self,
        client: &ComposioClientInner,
        auth: &ComposioAuth,
        app_name: Option<&str>,
    ) -> anyhow::Result<Vec<ComposioAction>> {
        let mut url = format!("{COMPOSIO_API_BASE_V2}/actions");
        if let Some(app) = app_name {
            url = format!("{url}?appNames={app}");
        }

        let resp = client
            .http
            .get(&url)
            .header("x-api-key", &auth.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v2 API error: {err}");
        }

        let body: ComposioActionsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v2 actions response")?;
        Ok(body.items)
    }

    pub(super) async fn execute_action(
        &self,
        client: &ComposioClientInner,
        auth: &ComposioAuth,
        action_name: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{COMPOSIO_API_BASE_V2}/actions/{action_name}/execute");

        let mut body = json!({
            "input": params,
        });
        if let Some(entity) = entity_id {
            body["entityId"] = json!(entity);
        }

        let resp = client
            .http
            .post(&url)
            .header("x-api-key", &auth.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v2 action execution failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v2 execute response")?;
        Ok(result)
    }

    pub(super) async fn get_connection_url(
        &self,
        client: &ComposioClientInner,
        auth: &ComposioAuth,
        app_name: &str,
        entity_id: &str,
    ) -> anyhow::Result<ComposioConnectionLink> {
        let url = format!("{COMPOSIO_API_BASE_V2}/connectedAccounts");

        let body = json!({
            "integrationId": app_name,
            "entityId": entity_id,
        });

        let resp = client
            .http
            .post(&url)
            .header("x-api-key", &auth.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v2 connect failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v2 connect response")?;
        let redirect_url = extract_redirect_url(&result)
            .ok_or_else(|| anyhow::anyhow!("No redirect URL in Composio v2 response"))?;

        Ok(ComposioConnectionLink {
            redirect_url,
            connected_account_id: extract_connected_account_id(&result),
        })
    }
}
