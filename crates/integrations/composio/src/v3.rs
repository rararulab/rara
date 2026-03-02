// Copyright 2025 Rararulab
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
use reqwest::Url;
use serde::Deserialize;
use serde_json::json;

use super::{
    ComposioAction, ComposioApi, ComposioAuth, ComposioClientInner, ComposioConnectedAccount,
    ComposioConnectedAccountsResponse, ComposioConnectionLink, ComposioToolkitRef, V3,
    extract_connected_account_id, extract_redirect_url, normalize_app_slug, response_error,
};

const COMPOSIO_API_BASE_V3: &str = "https://backend.composio.dev/api/v3";
const COMPOSIO_TOOL_VERSION_LATEST: &str = "latest";

impl ComposioApi<V3> {
    pub(super) async fn list_actions(
        &self,
        client: &ComposioClientInner,
        auth: &ComposioAuth,
        app_name: Option<&str>,
    ) -> anyhow::Result<Vec<ComposioAction>> {
        let mut url = Url::parse(&format!("{COMPOSIO_API_BASE_V3}/tools"))
            .context("Failed to build Composio v3 tools URL")?;
        for (key, value) in build_list_actions_v3_query(app_name) {
            url.query_pairs_mut().append_pair(&key, &value);
        }

        let resp = client
            .http
            .get(url)
            .header("x-api-key", &auth.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 API error: {err}");
        }

        let body: ComposioClientsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v3 tools response")?;
        Ok(map_v3_tools_to_actions(body.items))
    }

    pub(super) async fn list_connected_accounts(
        &self,
        client: &ComposioClientInner,
        auth: &ComposioAuth,
        app_name: Option<&str>,
        entity_id: Option<&str>,
    ) -> anyhow::Result<Vec<ComposioConnectedAccount>> {
        let mut url = Url::parse(&format!("{COMPOSIO_API_BASE_V3}/connected_accounts"))
            .context("Failed to build Composio v3 connected_accounts URL")?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", "50");
            query.append_pair("order_by", "updated_at");
            query.append_pair("order_direction", "desc");
            query.append_pair("statuses", "INITIALIZING");
            query.append_pair("statuses", "ACTIVE");
            query.append_pair("statuses", "INITIATED");
        }

        if let Some(app) = app_name
            .map(normalize_app_slug)
            .filter(|candidate| !candidate.is_empty())
        {
            url.query_pairs_mut()
                .append_pair("toolkit_slugs", app.as_str());
        }
        if let Some(entity) = entity_id {
            url.query_pairs_mut().append_pair("user_ids", entity);
        }

        let resp = client
            .http
            .get(url)
            .header("x-api-key", &auth.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 connected accounts lookup failed: {err}");
        }

        let body: ComposioConnectedAccountsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v3 connected accounts response")?;
        Ok(body.items)
    }

    pub(super) async fn execute_action(
        &self,
        client: &ComposioClientInner,
        auth: &ComposioAuth,
        tool_slug: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
        connected_account_ref: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let (url, body) =
            build_execute_action_v3_request(tool_slug, params, entity_id, connected_account_ref);

        ensure_https(&url)?;
        let resp = client
            .http
            .post(&url)
            .header("x-api-key", &auth.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 action execution failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v3 execute response")?;
        Ok(result)
    }

    pub(super) async fn get_connection_url(
        &self,
        client: &ComposioClientInner,
        auth: &ComposioAuth,
        app_name: Option<&str>,
        auth_config_id: Option<&str>,
        entity_id: &str,
    ) -> anyhow::Result<ComposioConnectionLink> {
        let auth_config_id = match auth_config_id {
            Some(id) => id.to_string(),
            None => {
                let app = app_name.ok_or_else(|| {
                    anyhow::anyhow!("Missing 'app' or 'auth_config_id' for v3 connect")
                })?;
                self.resolve_auth_config_id(client, auth, app).await?
            }
        };

        let url = format!("{COMPOSIO_API_BASE_V3}/connected_accounts/link");
        let body = json!({
            "auth_config_id": auth_config_id,
            "user_id": entity_id,
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
            anyhow::bail!("Composio v3 connect failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v3 connect response")?;
        let redirect_url = extract_redirect_url(&result)
            .ok_or_else(|| anyhow::anyhow!("No redirect URL in Composio v3 response"))?;

        Ok(ComposioConnectionLink {
            redirect_url,
            connected_account_id: extract_connected_account_id(&result),
        })
    }

    async fn resolve_auth_config_id(
        &self,
        client: &ComposioClientInner,
        auth: &ComposioAuth,
        app_name: &str,
    ) -> anyhow::Result<String> {
        let mut url = Url::parse(&format!("{COMPOSIO_API_BASE_V3}/auth_configs"))
            .context("Failed to build Composio v3 auth_configs URL")?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("toolkit_slug", app_name);
            query.append_pair("show_disabled", "true");
            query.append_pair("limit", "25");
        }

        let resp = client
            .http
            .get(url)
            .header("x-api-key", &auth.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 auth config lookup failed: {err}");
        }

        let body: ComposioAuthConfigsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v3 auth configs response")?;

        if body.items.is_empty() {
            anyhow::bail!(
                "No auth config found for toolkit '{app_name}'. Create one in Composio first."
            );
        }

        let preferred = body
            .items
            .iter()
            .find(|cfg| cfg.is_enabled())
            .or_else(|| body.items.first())
            .context("No usable auth config returned by Composio")?;

        Ok(preferred.id.clone())
    }
}

fn ensure_https(url: &str) -> anyhow::Result<()> {
    if !url.starts_with("https://") {
        anyhow::bail!(
            "Refusing to transmit sensitive data over non-HTTPS URL: URL scheme must be https"
        );
    }
    Ok(())
}

fn build_list_actions_v3_query(app_name: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![
        ("limit".to_string(), "200".to_string()),
        (
            "toolkit_versions".to_string(),
            COMPOSIO_TOOL_VERSION_LATEST.to_string(),
        ),
    ];

    if let Some(app) = app_name
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
    {
        query.push(("toolkits".to_string(), app.to_string()));
        query.push(("toolkit_slug".to_string(), app.to_string()));
    }

    query
}

fn build_execute_action_v3_request(
    tool_slug: &str,
    params: serde_json::Value,
    entity_id: Option<&str>,
    connected_account_ref: Option<&str>,
) -> (String, serde_json::Value) {
    let url = format!("{COMPOSIO_API_BASE_V3}/tools/execute/{tool_slug}");
    let account_ref = connected_account_ref.and_then(|candidate| {
        let trimmed_candidate = candidate.trim();
        (!trimmed_candidate.is_empty()).then_some(trimmed_candidate)
    });

    let mut body = json!({
        "arguments": params,
        "version": COMPOSIO_TOOL_VERSION_LATEST,
    });

    if let Some(entity) = entity_id {
        body["user_id"] = json!(entity);
    }
    if let Some(account_ref) = account_ref {
        body["connected_account_id"] = json!(account_ref);
    }

    (url, body)
}

fn map_v3_tools_to_actions(items: Vec<ComposioV3Tool>) -> Vec<ComposioAction> {
    items
        .into_iter()
        .filter_map(|item| {
            let name = item.slug.or(item.name.clone())?;
            let app_name = item
                .toolkit
                .as_ref()
                .and_then(|toolkit| toolkit.slug.clone().or(toolkit.name.clone()))
                .or(item.app_name);
            let description = item.description.or(item.name);
            Some(ComposioAction {
                name,
                app_name,
                description,
                enabled: true,
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct ComposioClientsResponse {
    #[serde(default)]
    items: Vec<ComposioV3Tool>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComposioV3Tool {
    #[serde(default)]
    slug:        Option<String>,
    #[serde(default)]
    name:        Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(rename = "appName", default)]
    app_name:    Option<String>,
    #[serde(default)]
    toolkit:     Option<ComposioToolkitRef>,
}

#[derive(Debug, Deserialize)]
struct ComposioAuthConfigsResponse {
    #[serde(default)]
    items: Vec<ComposioAuthConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct ComposioAuthConfig {
    id:      String,
    #[serde(default)]
    status:  Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

impl ComposioAuthConfig {
    fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
            || self
                .status
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("enabled"))
    }
}
