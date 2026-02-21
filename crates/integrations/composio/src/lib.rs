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

use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use parking_lot::RwLock;
use reqwest::Client;
use serde::{Deserialize, Serialize};

mod auth;
mod v2;
mod v3;
pub use auth::{
    ComposioAuth, ComposioAuthProvider, EnvComposioAuthProvider, StaticComposioAuthProvider,
};

/// V2 Composio API marker for typestate client.
pub struct V2;

/// V3 Composio API marker for typestate client.
pub struct V3;

fn build_http_client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

/// Version-typed Composio API client.
struct ComposioApi<V> {
    _v: PhantomData<V>,
}

impl<V> ComposioApi<V> {
    fn new() -> Self { Self { _v: PhantomData } }
}

/// Public facade used by tools: owns both v2/v3 typestate clients and handles
/// fallback.
#[derive(Clone)]
pub struct ComposioClient {
    inner: Arc<ComposioClientInner>,
}

struct ComposioClientInner {
    http: Client,
    auth_provider: Arc<dyn ComposioAuthProvider>,
    recent_connected_accounts: RwLock<HashMap<String, String>>,
    v2: ComposioApi<V2>,
    v3: ComposioApi<V3>,
}

impl ComposioClient {
    /// Create a Composio client facade with shared v2/v3 state.
    ///
    /// `default_entity_id` is used when callers omit `entity_id` for methods
    /// that support multi-user routing.
    pub fn new(api_key: &str, default_entity_id: Option<&str>) -> Self {
        Self::with_auth_provider(Arc::new(StaticComposioAuthProvider::new(
            api_key,
            default_entity_id,
        )))
    }

    /// Create a client that reads auth data from the given provider.
    pub fn with_auth_provider(auth_provider: Arc<dyn ComposioAuthProvider>) -> Self {
        let inner = ComposioClientInner {
            http: build_http_client(),
            auth_provider,
            recent_connected_accounts: RwLock::new(HashMap::new()),
            v2: ComposioApi::new(),
            v3: ComposioApi::new(),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Returns the default entity id from the current auth snapshot.
    pub async fn default_entity_id(&self) -> anyhow::Result<String> {
        let auth = self.inner.auth_provider.acquire_auth().await?;
        Ok(auth.default_entity_id)
    }

    /// List actions/tools available to the API key.
    ///
    /// Uses v3 first and falls back to v2 when v3 fails.
    pub async fn list_actions(
        &self,
        app_name: Option<&str>,
    ) -> anyhow::Result<Vec<ComposioAction>> {
        let auth = self.inner.auth_provider.acquire_auth().await?;
        match self
            .inner
            .v3
            .list_actions(&self.inner, &auth, app_name)
            .await
        {
            Ok(items) => Ok(items),
            Err(v3_err) => match self
                .inner
                .v2
                .list_actions(&self.inner, &auth, app_name)
                .await
            {
                Ok(items) => Ok(items),
                Err(v2_err) => anyhow::bail!(
                    "Composio action listing failed on v3 ({v3_err}) and v2 fallback ({v2_err})"
                ),
            },
        }
    }

    pub async fn list_connected_accounts(
        &self,
        app_name: Option<&str>,
        entity_id: Option<&str>,
    ) -> anyhow::Result<Vec<ComposioConnectedAccount>> {
        let auth = self.inner.auth_provider.acquire_auth().await?;
        let resolved_entity_id = normalize_entity_id(entity_id.unwrap_or(&auth.default_entity_id));
        self.inner
            .v3
            .list_connected_accounts(&self.inner, &auth, app_name, Some(&resolved_entity_id))
            .await
    }

    /// Execute a Composio action.
    ///
    /// Prefers v3 `tool_slug` execution and falls back to v2 legacy action
    /// names when needed.
    pub async fn execute_action(
        &self,
        action_name: &str,
        app_name_hint: Option<&str>,
        params: serde_json::Value,
        entity_id: Option<&str>,
        connected_account_ref: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let auth = self.inner.auth_provider.acquire_auth().await?;
        let app_hint = app_name_hint
            .map(normalize_app_slug)
            .filter(|app| !app.is_empty())
            .or_else(|| infer_app_slug_from_action_name(action_name));
        let normalized_entity_id = Some(normalize_entity_id(
            entity_id.unwrap_or(&auth.default_entity_id),
        ));

        let explicit_account_ref = connected_account_ref.and_then(|candidate| {
            let trimmed = candidate.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        });
        let resolved_account_ref = if explicit_account_ref.is_some() {
            explicit_account_ref
        } else {
            self.resolve_connected_account_ref(
                &auth,
                app_hint.as_deref(),
                normalized_entity_id.as_deref(),
            )
            .await?
        };

        // Build V3 candidates: try the normalized slug first, then the original
        // name as-is. Some Composio tools only respond to one format.
        let tool_slug = normalize_tool_slug(action_name);
        let original_trimmed = action_name.trim().to_string();
        let mut v3_candidates = vec![tool_slug];
        if !v3_candidates.contains(&original_trimmed) {
            v3_candidates.push(original_trimmed);
        }

        let mut v3_errors = Vec::new();
        for candidate in &v3_candidates {
            match self
                .inner
                .v3
                .execute_action(
                    &self.inner,
                    &auth,
                    candidate,
                    params.clone(),
                    normalized_entity_id.as_deref(),
                    resolved_account_ref.as_deref(),
                )
                .await
            {
                Ok(result) => return Ok(result),
                Err(e) => v3_errors.push(format!("{candidate}: {e}")),
            }
        }

        // V3 exhausted — fall back to V2 with legacy name formats.
        let mut v2_candidates = vec![action_name.trim().to_string()];
        let legacy_action_name = normalize_legacy_action_name(action_name);
        if !legacy_action_name.is_empty() && !v2_candidates.contains(&legacy_action_name) {
            v2_candidates.push(legacy_action_name);
        }

        let mut v2_errors = Vec::new();
        for candidate in v2_candidates {
            match self
                .inner
                .v2
                .execute_action(
                    &self.inner,
                    &auth,
                    &candidate,
                    params.clone(),
                    normalized_entity_id.as_deref(),
                )
                .await
            {
                Ok(result) => return Ok(result),
                Err(v2_err) => v2_errors.push(format!("{candidate}: {v2_err}")),
            }
        }

        anyhow::bail!(
            "Composio execute failed on v3 ({}) and v2 fallback ({}){}",
            v3_errors.join(" | "),
            v2_errors.join(" | "),
            build_connected_account_hint(
                app_hint.as_deref(),
                normalized_entity_id.as_deref(),
                resolved_account_ref.as_deref(),
            )
        );
    }

    /// Build the OAuth connection link for an app/auth config.
    ///
    /// Uses v3 first and falls back to v2 connect flow when possible.
    pub async fn get_connection_url(
        &self,
        app_name: Option<&str>,
        auth_config_id: Option<&str>,
        entity_id: &str,
    ) -> anyhow::Result<ComposioConnectionLink> {
        let auth = self.inner.auth_provider.acquire_auth().await?;
        match self
            .inner
            .v3
            .get_connection_url(&self.inner, &auth, app_name, auth_config_id, entity_id)
            .await
        {
            Ok(url) => Ok(url),
            Err(v3_err) => {
                let app = app_name.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Composio v3 connect failed ({v3_err}) and v2 fallback requires 'app'"
                    )
                })?;
                match self
                    .inner
                    .v2
                    .get_connection_url(&self.inner, &auth, app, entity_id)
                    .await
                {
                    Ok(url) => Ok(url),
                    Err(v2_err) => anyhow::bail!(
                        "Composio connect failed on v3 ({v3_err}) and v2 fallback ({v2_err})"
                    ),
                }
            }
        }
    }

    fn cache_connected_account(&self, app_name: &str, entity_id: &str, connected_account_id: &str) {
        let key = connected_account_cache_key(app_name, entity_id);
        self.inner
            .recent_connected_accounts
            .write()
            .insert(key, connected_account_id.to_string());
    }

    fn get_cached_connected_account(&self, app_name: &str, entity_id: &str) -> Option<String> {
        let key = connected_account_cache_key(app_name, entity_id);
        self.inner
            .recent_connected_accounts
            .read()
            .get(&key)
            .cloned()
    }

    async fn resolve_connected_account_ref(
        &self,
        auth: &ComposioAuth,
        app_name: Option<&str>,
        entity_id: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let app = app_name
            .map(normalize_app_slug)
            .filter(|candidate| !candidate.is_empty());
        let entity = entity_id
            .map(normalize_entity_id)
            .or_else(|| Some(auth.default_entity_id.clone()));
        let (Some(app), Some(entity)) = (app, entity) else {
            return Ok(None);
        };

        if let Some(cached) = self.get_cached_connected_account(&app, &entity) {
            return Ok(Some(cached));
        }

        let accounts = self
            .inner
            .v3
            .list_connected_accounts(&self.inner, auth, Some(&app), Some(&entity))
            .await?;
        let Some(first) = accounts
            .into_iter()
            .find(ComposioConnectedAccount::is_usable)
        else {
            return Ok(None);
        };

        self.cache_connected_account(&app, &entity, &first.id);
        Ok(Some(first.id))
    }
}

fn normalize_entity_id(entity_id: &str) -> String {
    let trimmed = entity_id.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_tool_slug(action_name: &str) -> String {
    action_name.trim().replace('_', "-").to_ascii_lowercase()
}

fn normalize_legacy_action_name(action_name: &str) -> String {
    action_name.trim().replace('-', "_").to_ascii_uppercase()
}

fn normalize_app_slug(app_name: &str) -> String {
    app_name
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn infer_app_slug_from_action_name(action_name: &str) -> Option<String> {
    let trimmed = action_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let raw = if trimmed.contains('-') {
        trimmed.split('-').next()
    } else if trimmed.contains('_') {
        trimmed.split('_').next()
    } else {
        None
    }?;

    let app = normalize_app_slug(raw);
    (!app.is_empty()).then_some(app)
}

fn connected_account_cache_key(app_name: &str, entity_id: &str) -> String {
    format!(
        "{}:{}",
        normalize_entity_id(entity_id),
        normalize_app_slug(app_name)
    )
}

fn build_connected_account_hint(
    app_hint: Option<&str>,
    entity_id: Option<&str>,
    connected_account_ref: Option<&str>,
) -> String {
    if connected_account_ref.is_some() {
        return String::new();
    }

    let Some(entity) = entity_id else {
        return String::new();
    };

    if let Some(app) = app_hint {
        format!(
            " Hint: use action='list_accounts' with app='{app}' and entity_id='{entity}' to \
             retrieve connected_account_id."
        )
    } else {
        format!(
            " Hint: use action='list_accounts' with entity_id='{entity}' to retrieve \
             connected_account_id."
        )
    }
}

fn extract_redirect_url(result: &serde_json::Value) -> Option<String> {
    result
        .get("redirect_url")
        .and_then(|v| v.as_str())
        .or_else(|| result.get("redirectUrl").and_then(|v| v.as_str()))
        .or_else(|| {
            result
                .get("data")
                .and_then(|v| v.get("redirect_url"))
                .and_then(|v| v.as_str())
        })
        .map(ToString::to_string)
}

fn extract_connected_account_id(result: &serde_json::Value) -> Option<String> {
    result
        .get("connected_account_id")
        .and_then(|v| v.as_str())
        .or_else(|| result.get("connectedAccountId").and_then(|v| v.as_str()))
        .or_else(|| {
            result
                .get("data")
                .and_then(|v| v.get("connected_account_id"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            result
                .get("data")
                .and_then(|v| v.get("connectedAccountId"))
                .and_then(|v| v.as_str())
        })
        .map(ToString::to_string)
}

async fn response_error(resp: reqwest::Response) -> String {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if body.trim().is_empty() {
        return format!("HTTP {}", status.as_u16());
    }

    if let Some(api_error) = extract_api_error_message(&body) {
        return format!(
            "HTTP {}: {}",
            status.as_u16(),
            sanitize_error_message(&api_error)
        );
    }

    format!("HTTP {}", status.as_u16())
}

fn sanitize_error_message(message: &str) -> String {
    let mut sanitized = message.replace('\n', " ");
    for marker in [
        "connected_account_id",
        "connectedAccountId",
        "entity_id",
        "entityId",
        "user_id",
        "userId",
    ] {
        sanitized = sanitized.replace(marker, "[redacted]");
    }

    let max_chars = 240;
    if sanitized.chars().count() <= max_chars {
        sanitized
    } else {
        let mut end = max_chars;
        while end > 0 && !sanitized.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &sanitized[..end])
    }
}

fn extract_api_error_message(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    parsed
        .get("error")
        .and_then(|v| v.get("message"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            parsed
                .get("message")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
        })
}

#[derive(Debug, Deserialize)]
struct ComposioActionsResponse {
    #[serde(default)]
    items: Vec<ComposioAction>,
}

#[derive(Debug, Deserialize)]
struct ComposioConnectedAccountsResponse {
    #[serde(default)]
    items: Vec<ComposioConnectedAccount>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComposioConnectedAccount {
    pub id:      String,
    #[serde(default)]
    pub status:  String,
    #[serde(default)]
    pub toolkit: Option<ComposioToolkitRef>,
}

impl ComposioConnectedAccount {
    /// Returns true when the account is in a state suitable for execution.
    pub fn is_usable(&self) -> bool {
        self.status.eq_ignore_ascii_case("INITIALIZING")
            || self.status.eq_ignore_ascii_case("ACTIVE")
            || self.status.eq_ignore_ascii_case("INITIATED")
    }

    /// Returns the toolkit slug if present on the connected account payload.
    pub fn toolkit_slug(&self) -> Option<&str> {
        self.toolkit
            .as_ref()
            .and_then(|toolkit| toolkit.slug.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComposioToolkitRef {
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ComposioConnectionLink {
    pub redirect_url:         String,
    pub connected_account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioAction {
    pub name:        String,
    #[serde(rename = "appName")]
    pub app_name:    Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub enabled:     bool,
}
