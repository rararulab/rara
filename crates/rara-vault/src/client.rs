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

use std::collections::HashMap;
use std::sync::Arc;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::config::VaultConfig;
use crate::error::{self, VaultError};

// ---------------------------------------------------------------------------
// Vault API response types
// ---------------------------------------------------------------------------

/// Wrapper for Vault KV v2 read responses.
#[derive(Debug, Deserialize)]
pub struct KvV2ReadResponse {
    pub data: KvV2Data,
}

/// Inner data envelope of a KV v2 read response.
#[derive(Debug, Deserialize)]
pub struct KvV2Data {
    pub data: HashMap<String, serde_json::Value>,
    pub metadata: KvV2Metadata,
}

/// Metadata attached to a KV v2 secret version.
#[derive(Debug, Deserialize)]
pub struct KvV2Metadata {
    pub version: u64,
    pub created_time: String,
    #[serde(default)]
    pub destroyed: bool,
}

/// Response from `LIST` on the metadata endpoint.
#[derive(Debug, Deserialize)]
pub struct ListResponse {
    pub data: ListKeys,
}

/// Key list inside a `LIST` response.
#[derive(Debug, Deserialize)]
pub struct ListKeys {
    pub keys: Vec<String>,
}

/// Vault auth/login response.
#[derive(Debug, Deserialize)]
struct AuthResponse {
    auth: AuthData,
}

#[derive(Debug, Deserialize)]
struct AuthData {
    client_token: String,
    lease_duration: u64,
}

/// Generic Vault error response body.
#[derive(Debug, Deserialize)]
struct VaultErrorBody {
    #[serde(default)]
    errors: Vec<String>,
}

/// Body sent to the KV v2 `data/` endpoint for writes.
#[derive(Debug, Serialize)]
struct KvV2WriteBody {
    data: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// VaultClient
// ---------------------------------------------------------------------------

/// Internal token state.
struct TokenState {
    token: String,
    /// Seconds remaining when the token was acquired.
    lease_duration: u64,
    /// When the token was acquired (monotonic).
    acquired_at: tokio::time::Instant,
}

/// HTTP client for HashiCorp Vault KV v2 with AppRole authentication.
pub struct VaultClient {
    config: VaultConfig,
    http: Client,
    token_state: Arc<RwLock<Option<TokenState>>>,
}

impl VaultClient {
    /// Create a new `VaultClient` from the given config.
    ///
    /// The client is *not* authenticated yet — call [`login()`](Self::login)
    /// before issuing any secret operations.
    pub fn new(config: VaultConfig) -> Result<Self, VaultError> {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .context(error::ConnectionSnafu)?;
        Ok(Self {
            config,
            http,
            token_state: Arc::new(RwLock::new(None)),
        })
    }

    // ------------------------------------------------------------------
    // Authentication
    // ------------------------------------------------------------------

    /// Authenticate with Vault using AppRole credentials read from disk.
    pub async fn login(&self) -> Result<(), VaultError> {
        let role_id = tokio::fs::read_to_string(&self.config.auth.role_id_file)
            .await
            .context(error::CredentialFileSnafu {
                path: self.config.auth.role_id_file.display().to_string(),
            })?;
        let secret_id = tokio::fs::read_to_string(&self.config.auth.secret_id_file)
            .await
            .context(error::CredentialFileSnafu {
                path: self.config.auth.secret_id_file.display().to_string(),
            })?;

        let url = format!("{}/v1/auth/approle/login", self.config.address);
        let body = serde_json::json!({
            "role_id": role_id.trim(),
            "secret_id": secret_id.trim(),
        });

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context(error::AuthSnafu)?;

        let status = resp.status();
        if !status.is_success() {
            let msg = extract_error_message(resp).await;
            return Err(VaultError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let auth_resp: AuthResponse = resp.json().await.context(error::AuthSnafu)?;
        debug!(
            lease_duration = auth_resp.auth.lease_duration,
            "Vault AppRole login succeeded"
        );

        let mut state = self.token_state.write().await;
        *state = Some(TokenState {
            token: auth_resp.auth.client_token,
            lease_duration: auth_resp.auth.lease_duration,
            acquired_at: tokio::time::Instant::now(),
        });
        Ok(())
    }

    /// Renew the current client token in-place.
    pub async fn renew_token(&self) -> Result<(), VaultError> {
        let token = self.get_token_raw().await?;
        let url = format!("{}/v1/auth/token/renew-self", self.config.address);

        let resp = self
            .http
            .post(&url)
            .header("X-Vault-Token", &token)
            .send()
            .await
            .context(error::ConnectionSnafu)?;

        let status = resp.status();
        if !status.is_success() {
            let msg = extract_error_message(resp).await;
            warn!(status = status.as_u16(), msg, "Token renewal failed");
            return Err(VaultError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let auth_resp: AuthResponse = resp.json().await.context(error::ConnectionSnafu)?;
        let mut state = self.token_state.write().await;
        *state = Some(TokenState {
            token: auth_resp.auth.client_token,
            lease_duration: auth_resp.auth.lease_duration,
            acquired_at: tokio::time::Instant::now(),
        });
        debug!("Vault token renewed");
        Ok(())
    }

    /// Returns `true` when the token has passed half its lease duration.
    pub async fn token_needs_renewal(&self) -> bool {
        let state = self.token_state.read().await;
        match state.as_ref() {
            Some(ts) => {
                let elapsed = ts.acquired_at.elapsed().as_secs();
                elapsed >= ts.lease_duration / 2
            }
            None => true,
        }
    }

    // ------------------------------------------------------------------
    // Secret read operations
    // ------------------------------------------------------------------

    /// Read a single secret from the KV v2 store.
    pub async fn read_secret(&self, path: &str) -> Result<KvV2ReadResponse, VaultError> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/v1/{}/data/{}",
            self.config.address, self.config.mount_path, path
        );

        let resp = self
            .http
            .get(&url)
            .header("X-Vault-Token", &token)
            .send()
            .await
            .context(error::ConnectionSnafu)?;

        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(VaultError::NotFound {
                path: path.to_string(),
            });
        }
        if !status.is_success() {
            let msg = extract_error_message(resp).await;
            return Err(VaultError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let body = resp.text().await.context(error::ConnectionSnafu)?;
        serde_json::from_str(&body).context(error::DeserializeSnafu)
    }

    /// List child keys under a metadata path.
    pub async fn list_secrets(&self, path: &str) -> Result<Vec<String>, VaultError> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/v1/{}/metadata/{}",
            self.config.address, self.config.mount_path, path
        );

        let resp = self
            .http
            .request(reqwest::Method::from_bytes(b"LIST").expect("valid method"), &url)
            .header("X-Vault-Token", &token)
            .send()
            .await
            .context(error::ConnectionSnafu)?;

        let status = resp.status();
        if status.as_u16() == 404 {
            return Ok(vec![]);
        }
        if !status.is_success() {
            let msg = extract_error_message(resp).await;
            return Err(VaultError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let body = resp.text().await.context(error::ConnectionSnafu)?;
        let list: ListResponse = serde_json::from_str(&body).context(error::DeserializeSnafu)?;
        Ok(list.data.keys)
    }

    /// Pull all secrets under `config/` and `secrets/` and flatten them
    /// into dot-separated key-value pairs compatible with the settings
    /// store format used by `crates/app/src/flatten.rs`.
    ///
    /// Vault key names are mapped to settings key prefixes via
    /// [`vault_key_to_settings_prefix`]. For example, the Vault path
    /// `secrets/knowledge` maps to settings prefix `memory.knowledge`,
    /// so `{"embedding_model": "..."}` becomes
    /// `[("memory.knowledge.embedding_model", "...")]`.
    pub async fn pull_all(&self) -> Result<Vec<(String, String)>, VaultError> {
        let mut pairs = Vec::new();

        for prefix in &["config", "secrets"] {
            let keys = self.list_secrets(prefix).await?;
            for key in &keys {
                // Skip directory markers (trailing slash)
                if key.ends_with('/') {
                    continue;
                }
                let vault_path = format!("{prefix}/{key}");
                match self.read_secret(&vault_path).await {
                    Ok(resp) => {
                        let settings_prefix = vault_key_to_settings_prefix(key);
                        flatten_value(&settings_prefix, &serde_json::Value::Object(
                            resp.data.data.into_iter().collect(),
                        ), &mut pairs);
                    }
                    Err(VaultError::NotFound { .. }) => {
                        debug!(path = vault_path, "secret not found during pull_all, skipping");
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        Ok(pairs)
    }

    // ------------------------------------------------------------------
    // Secret write operations
    // ------------------------------------------------------------------

    /// Write a secret to the KV v2 store.
    pub async fn write_secret(
        &self,
        path: &str,
        data: HashMap<String, serde_json::Value>,
    ) -> Result<(), VaultError> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/v1/{}/data/{}",
            self.config.address, self.config.mount_path, path
        );

        let body = KvV2WriteBody { data };
        let resp = self
            .http
            .post(&url)
            .header("X-Vault-Token", &token)
            .json(&body)
            .send()
            .await
            .context(error::ConnectionSnafu)?;

        let status = resp.status();
        if !status.is_success() {
            let msg = extract_error_message(resp).await;
            return Err(VaultError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }
        Ok(())
    }

    /// Push flat key-value pairs back into Vault by unflattening them
    /// into the appropriate path structure.
    ///
    /// Keys are expected in the format `"section.field"` or
    /// `"section.nested.field"`. The first segment determines the Vault
    /// path under `config/`.
    pub async fn push_changes(&self, changes: Vec<(String, String)>) -> Result<(), VaultError> {
        let grouped = unflatten_to_vault_paths(changes);
        for (path, data) in grouped {
            self.write_secret(&path, data).await?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Metadata
    // ------------------------------------------------------------------

    /// Read metadata (version info) for a secret path.
    pub async fn get_metadata(&self, path: &str) -> Result<KvV2Metadata, VaultError> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/v1/{}/metadata/{}",
            self.config.address, self.config.mount_path, path
        );

        let resp = self
            .http
            .get(&url)
            .header("X-Vault-Token", &token)
            .send()
            .await
            .context(error::ConnectionSnafu)?;

        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(VaultError::NotFound {
                path: path.to_string(),
            });
        }
        if !status.is_success() {
            let msg = extract_error_message(resp).await;
            return Err(VaultError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        #[derive(Deserialize)]
        struct MetadataResp {
            data: MetadataInner,
        }
        #[derive(Deserialize)]
        struct MetadataInner {
            current_version: u64,
            created_time: String,
        }

        let body = resp.text().await.context(error::ConnectionSnafu)?;
        let meta: MetadataResp =
            serde_json::from_str(&body).context(error::DeserializeSnafu)?;
        Ok(KvV2Metadata {
            version: meta.data.current_version,
            created_time: meta.data.created_time,
            destroyed: false,
        })
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Return the cached token without triggering renewal.
    /// Used internally by `renew_token()` and `login()` to avoid recursion.
    async fn get_token_raw(&self) -> Result<String, VaultError> {
        let state = self.token_state.read().await;
        match state.as_ref() {
            Some(ts) => Ok(ts.token.clone()),
            None => Err(VaultError::TokenExpired),
        }
    }

    /// Return a valid token, renewing transparently if past half TTL.
    async fn get_token(&self) -> Result<String, VaultError> {
        {
            let state = self.token_state.read().await;
            match state.as_ref() {
                Some(ts) => {
                    let elapsed = ts.acquired_at.elapsed().as_secs();
                    if elapsed >= ts.lease_duration / 2 {
                        drop(state);
                        if let Err(e) = self.renew_token_or_relogin().await {
                            warn!(error = %e, "token renewal failed, returning cached token");
                        }
                    }
                }
                None => return Err(VaultError::TokenExpired),
            }
        }
        self.get_token_raw().await
    }

    /// Try to renew the token; if renewal fails, fall back to a full re-login.
    async fn renew_token_or_relogin(&self) -> Result<(), VaultError> {
        debug!("token past half TTL, attempting renewal");
        match self.renew_token().await {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!(error = %e, "token renewal failed, attempting full re-login");
                self.login().await
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Flatten / unflatten helpers
// ---------------------------------------------------------------------------

/// Map a Vault key name to the settings key prefix used by the existing
/// Settings KV store.
///
/// Most Vault keys map 1:1 (e.g. `"http"` → `"http"`), but some have
/// different prefixes in the settings store:
/// - `"knowledge"` → `"memory.knowledge"` (settings keys: `memory.knowledge.*`)
/// - `"composio"` → uses constants from `rara_domain_shared::settings::keys`
///   but those happen to match `"composio.*"` pattern
///
/// The reverse mapping is in [`settings_prefix_to_vault_key`].
fn vault_key_to_settings_prefix(vault_key: &str) -> String {
    match vault_key {
        "knowledge" => "memory.knowledge".into(),
        other => other.into(),
    }
}

/// Map a settings key prefix back to the Vault key name.
///
/// Inverse of [`vault_key_to_settings_prefix`].
fn settings_prefix_to_vault_key(settings_key: &str) -> &str {
    if settings_key.starts_with("memory.knowledge") {
        "knowledge"
    } else {
        // For most keys, the first segment is the Vault key name.
        settings_key.split('.').next().unwrap_or(settings_key)
    }
}

/// Recursively flatten a JSON value into dot-separated key-value pairs.
///
/// The `prefix` is the top-level section name (e.g. `"http"` or `"llm"`).
pub(crate) fn flatten_value(
    prefix: &str,
    value: &serde_json::Value,
    out: &mut Vec<(String, String)>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let key = format!("{prefix}.{k}");
                flatten_value(&key, v, out);
            }
        }
        serde_json::Value::Array(arr) => {
            let joined: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect();
            out.push((prefix.to_string(), joined.join(",")));
        }
        serde_json::Value::String(s) => {
            out.push((prefix.to_string(), s.clone()));
        }
        serde_json::Value::Number(n) => {
            out.push((prefix.to_string(), n.to_string()));
        }
        serde_json::Value::Bool(b) => {
            out.push((prefix.to_string(), b.to_string()));
        }
        serde_json::Value::Null => {}
    }
}

/// Settings keys that must be routed to `secrets/` instead of `config/`.
///
/// This list defines which flattened key prefixes contain sensitive data
/// (API keys, tokens, passwords) and must be written to the `secrets/`
/// path in Vault, where stricter ACL policies apply.
const SECRET_KEY_PREFIXES: &[&str] = &[
    "telegram.bot_token",
    "composio.api_key",
    "composio.entity_id",
    "gmail.app_password",
    "memory.memos.token",
];

/// Check if a flattened settings key contains a secret value that should
/// be stored under `secrets/` rather than `config/`.
fn is_secret_key(key: &str) -> bool {
    // Exact match on known secret keys
    if SECRET_KEY_PREFIXES.contains(&key) {
        return true;
    }
    // LLM provider api_key fields: "llm.providers.{name}.api_key"
    if key.starts_with("llm.providers.") && key.ends_with(".api_key") {
        return true;
    }
    // Symphony tracker api_key
    if key.starts_with("symphony.") && key.contains("api_key") {
        return true;
    }
    false
}

/// Group flat key-value pairs into Vault write paths.
///
/// Sensitive keys (API keys, tokens) are routed to `secrets/{section}`,
/// everything else goes to `config/{section}`. This maintains the
/// permission separation between config/ and secrets/ in Vault.
pub(crate) fn unflatten_to_vault_paths(
    pairs: Vec<(String, String)>,
) -> HashMap<String, HashMap<String, serde_json::Value>> {
    let mut grouped: HashMap<String, HashMap<String, serde_json::Value>> = HashMap::new();

    for (key, value) in pairs {
        if let Some(dot_pos) = key.find('.') {
            let vault_key = settings_prefix_to_vault_key(&key);
            let vault_prefix = if is_secret_key(&key) { "secrets" } else { "config" };
            let path = format!("{vault_prefix}/{vault_key}");

            // Strip the settings prefix to get the field path within
            // the Vault secret. For "memory.knowledge.embedding_model",
            // vault_key is "knowledge", so we need the part after
            // "memory.knowledge." → "embedding_model".
            let settings_prefix = vault_key_to_settings_prefix(vault_key);
            let rest = key.strip_prefix(&format!("{settings_prefix}.")).unwrap_or(&key[dot_pos + 1..]);

            let entry = grouped.entry(path).or_default();
            set_nested_value(entry, rest, serde_json::Value::String(value));
        }
    }

    grouped
}

/// Set a nested value in a `HashMap` using a dot-separated key.
///
/// For `"providers.openrouter.api_key"` it creates nested objects:
/// `{"providers": {"openrouter": {"api_key": value}}}`.
fn set_nested_value(
    map: &mut HashMap<String, serde_json::Value>,
    dotted_key: &str,
    value: serde_json::Value,
) {
    let parts: Vec<&str> = dotted_key.splitn(2, '.').collect();
    if parts.len() == 1 {
        map.insert(parts[0].to_string(), value);
    } else {
        let entry = map
            .entry(parts[0].to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(inner) = entry {
            let mut inner_map: HashMap<String, serde_json::Value> = inner
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            set_nested_value(&mut inner_map, parts[1], value);
            *entry = serde_json::Value::Object(inner_map.into_iter().collect());
        }
    }
}

/// Extract a human-readable error message from a Vault error response.
async fn extract_error_message(resp: reqwest::Response) -> String {
    resp.text().await.unwrap_or_else(|_| "unknown error".into())
}

// ---------------------------------------------------------------------------
// URL builder (for testing)
// ---------------------------------------------------------------------------

impl VaultClient {
    /// Build the URL for reading a secret (exposed for unit tests).
    #[cfg(test)]
    fn read_secret_url(&self, path: &str) -> String {
        format!(
            "{}/v1/{}/data/{}",
            self.config.address, self.config.mount_path, path
        )
    }

    /// Build the URL for listing secrets (exposed for unit tests).
    #[cfg(test)]
    fn list_secrets_url(&self, path: &str) -> String {
        format!(
            "{}/v1/{}/metadata/{}",
            self.config.address, self.config.mount_path, path
        )
    }

    /// Build the URL for the AppRole login endpoint (exposed for unit tests).
    #[cfg(test)]
    fn login_url(&self) -> String {
        format!("{}/v1/auth/approle/login", self.config.address)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{VaultAuthConfig, VaultConfig};
    use std::time::Duration;

    fn test_config() -> VaultConfig {
        VaultConfig {
            address: "http://10.0.0.5:30820".into(),
            mount_path: "secret/rara".into(),
            auth: VaultAuthConfig {
                method: "approle".into(),
                role_id_file: "/etc/rara/vault-role-id".into(),
                secret_id_file: "/etc/rara/vault-secret-id".into(),
            },
            watch_interval: Duration::from_secs(30),
            timeout: Duration::from_secs(5),
            fallback_to_local: true,
        }
    }

    #[test]
    fn url_construction() {
        let client = VaultClient::new(test_config()).unwrap();

        assert_eq!(
            client.read_secret_url("config/http"),
            "http://10.0.0.5:30820/v1/secret/rara/data/config/http"
        );
        assert_eq!(
            client.list_secrets_url("config"),
            "http://10.0.0.5:30820/v1/secret/rara/metadata/config"
        );
        assert_eq!(
            client.login_url(),
            "http://10.0.0.5:30820/v1/auth/approle/login"
        );
    }

    #[test]
    fn flatten_simple_object() {
        let data = serde_json::json!({
            "bind_address": "127.0.0.1:25555",
            "port": 8080
        });
        let mut pairs = Vec::new();
        flatten_value("http", &data, &mut pairs);
        pairs.sort();

        assert_eq!(pairs.len(), 2);
        assert!(pairs.contains(&("http.bind_address".into(), "127.0.0.1:25555".into())));
        assert!(pairs.contains(&("http.port".into(), "8080".into())));
    }

    #[test]
    fn flatten_nested_object() {
        let data = serde_json::json!({
            "providers": {
                "openrouter": {
                    "api_key": "sk-xxx",
                    "base_url": "https://openrouter.ai/api/v1"
                }
            }
        });
        let mut pairs = Vec::new();
        flatten_value("llm", &data, &mut pairs);
        pairs.sort();

        assert_eq!(pairs.len(), 2);
        assert!(pairs.contains(&(
            "llm.providers.openrouter.api_key".into(),
            "sk-xxx".into()
        )));
        assert!(pairs.contains(&(
            "llm.providers.openrouter.base_url".into(),
            "https://openrouter.ai/api/v1".into()
        )));
    }

    #[test]
    fn flatten_array_values() {
        let data = serde_json::json!({
            "fallback_models": ["qwen3:14b", "llama3:8b"]
        });
        let mut pairs = Vec::new();
        flatten_value("llm.providers.ollama", &data, &mut pairs);

        assert_eq!(pairs.len(), 1);
        assert_eq!(
            pairs[0],
            (
                "llm.providers.ollama.fallback_models".into(),
                "qwen3:14b,llama3:8b".into()
            )
        );
    }

    #[test]
    fn unflatten_routes_secrets_correctly() {
        let input = vec![
            ("http.bind_address".into(), "127.0.0.1:25555".into()),
            ("http.port".into(), "8080".into()),
            // api_key should go to secrets/, not config/
            (
                "llm.providers.openrouter.api_key".into(),
                "sk-xxx".into(),
            ),
            // base_url is not a secret, goes to config/
            (
                "llm.providers.openrouter.base_url".into(),
                "https://openrouter.ai/api/v1".into(),
            ),
            // telegram bot_token should go to secrets/
            ("telegram.bot_token".into(), "123:ABC".into()),
            // telegram chat_id is not a secret
            ("telegram.chat_id".into(), "456".into()),
        ];

        let grouped = unflatten_to_vault_paths(input);

        // http.bind_address → config/http
        let http_data = grouped.get("config/http").expect("config/http");
        assert_eq!(
            http_data.get("bind_address"),
            Some(&serde_json::Value::String("127.0.0.1:25555".into()))
        );

        // llm api_key → secrets/llm
        let llm_secrets = grouped.get("secrets/llm").expect("secrets/llm");
        let providers = llm_secrets.get("providers").expect("providers");
        let openrouter = providers.get("openrouter").expect("openrouter");
        assert_eq!(
            openrouter.get("api_key"),
            Some(&serde_json::Value::String("sk-xxx".into()))
        );

        // llm base_url → config/llm (not secrets)
        let llm_config = grouped.get("config/llm").expect("config/llm");
        let providers = llm_config.get("providers").expect("providers");
        let openrouter = providers.get("openrouter").expect("openrouter");
        assert_eq!(
            openrouter.get("base_url"),
            Some(&serde_json::Value::String("https://openrouter.ai/api/v1".into()))
        );

        // telegram.bot_token → secrets/telegram
        let tg_secrets = grouped.get("secrets/telegram").expect("secrets/telegram");
        assert_eq!(
            tg_secrets.get("bot_token"),
            Some(&serde_json::Value::String("123:ABC".into()))
        );

        // telegram.chat_id → config/telegram
        let tg_config = grouped.get("config/telegram").expect("config/telegram");
        assert_eq!(
            tg_config.get("chat_id"),
            Some(&serde_json::Value::String("456".into()))
        );
    }

    #[test]
    fn unflatten_knowledge_uses_vault_key() {
        // Settings key is "memory.knowledge.embedding_model"
        // but Vault path should be "config/knowledge" with field "embedding_model"
        let input = vec![
            (
                "memory.knowledge.embedding_model".into(),
                "text-embedding-3-small".into(),
            ),
            (
                "memory.knowledge.embedding_dimensions".into(),
                "1536".into(),
            ),
        ];

        let grouped = unflatten_to_vault_paths(input);

        let know_data = grouped.get("config/knowledge").expect("config/knowledge");
        assert_eq!(
            know_data.get("embedding_model"),
            Some(&serde_json::Value::String("text-embedding-3-small".into()))
        );
        assert_eq!(
            know_data.get("embedding_dimensions"),
            Some(&serde_json::Value::String("1536".into()))
        );
    }

    #[test]
    fn vault_key_mapping_roundtrip() {
        // knowledge ↔ memory.knowledge
        assert_eq!(vault_key_to_settings_prefix("knowledge"), "memory.knowledge");
        assert_eq!(settings_prefix_to_vault_key("memory.knowledge.embedding_model"), "knowledge");

        // Most keys are identity mappings
        assert_eq!(vault_key_to_settings_prefix("http"), "http");
        assert_eq!(settings_prefix_to_vault_key("http.bind_address"), "http");
        assert_eq!(vault_key_to_settings_prefix("llm"), "llm");
        assert_eq!(settings_prefix_to_vault_key("llm.default_provider"), "llm");
    }

    #[test]
    fn set_nested_creates_structure() {
        let mut map = HashMap::new();
        set_nested_value(
            &mut map,
            "a.b.c",
            serde_json::Value::String("deep".into()),
        );

        let a = map.get("a").expect("a");
        let b = a.get("b").expect("b");
        let c = b.get("c").expect("c");
        assert_eq!(c, &serde_json::Value::String("deep".into()));
    }
}
