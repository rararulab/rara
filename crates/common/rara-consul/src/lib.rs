mod error;

use base64::Engine as _;
use config::{Map, Value, ValueKind};
use snafu::ResultExt;

// ---------------------------------------------------------------------------
// ConsulConfig — loaded from environment variables
// ---------------------------------------------------------------------------

/// Configuration for connecting to Consul KV.
///
/// Loaded from environment variables:
/// - `CONSUL_HTTP_ADDR` (default: `http://localhost:8500`)
/// - `CONSUL_TOKEN` (optional, for ACL)
/// - `CONSUL_KV_PREFIX` (default: `rara/config/`)
#[derive(Debug, Clone)]
pub struct ConsulConfig {
    /// Consul HTTP address.
    pub addr:   String,
    /// Optional ACL token.
    pub token:  Option<String>,
    /// KV prefix to fetch (with trailing slash).
    pub prefix: String,
}

impl ConsulConfig {
    /// Try to load from environment variables.
    ///
    /// Returns `Some` only when `CONSUL_HTTP_ADDR` is set (opt-in).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let addr = std::env::var("CONSUL_HTTP_ADDR").ok()?;

        let token = std::env::var("CONSUL_TOKEN").ok().filter(|t| !t.is_empty());

        let prefix = std::env::var("CONSUL_KV_PREFIX")
            .unwrap_or_else(|_| "rara/config/".to_owned());

        Some(Self {
            addr,
            token,
            prefix,
        })
    }
}

// ---------------------------------------------------------------------------
// Consul KV response types
// ---------------------------------------------------------------------------

/// A single entry returned by the Consul KV `?recurse` API.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct KvEntry {
    /// Full key path (e.g. `rara/config/database/database_url`).
    key:   String,
    /// Base64-encoded value.
    value: Option<String>,
}

// ---------------------------------------------------------------------------
// ConsulSource — config::AsyncSource implementation
// ---------------------------------------------------------------------------

/// Consul KV as a [`config`] crate async source.
///
/// Fetches all keys under the configured prefix and maps KV paths to nested
/// config keys. For example, `rara/config/database/database_url` becomes
/// `database.database_url` in the config map.
#[derive(Debug)]
pub struct ConsulSource {
    cfg:    ConsulConfig,
    client: reqwest::Client,
}

impl ConsulSource {
    pub fn new(cfg: ConsulConfig) -> Self {
        Self {
            cfg,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl config::AsyncSource for ConsulSource {
    async fn collect(&self) -> Result<Map<String, Value>, config::ConfigError> {
        let entries = fetch_kv_recursive(&self.client, &self.cfg)
            .await
            .map_err(|e| config::ConfigError::Foreign(Box::new(e)))?;

        let mut map = Map::new();
        for entry in &entries {
            let Some(ref b64_value) = entry.value else {
                continue; // directory marker, no value
            };

            // Strip the prefix to get the relative key path
            let relative = match entry.key.strip_prefix(&self.cfg.prefix) {
                Some(r) if !r.is_empty() => r,
                _ => continue,
            };

            // Map path segments to nested config key:
            //   database/database_url  ->  database.database_url
            let config_key = relative.replace('/', ".");

            // Decode base64 value
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(b64_value)
                .map_err(|e| {
                    config::ConfigError::Foreign(Box::new(error::ConsulError::Base64Decode {
                        key:    entry.key.clone(),
                        source: e,
                    }))
                })?;

            let value_str = String::from_utf8(decoded).map_err(|e| {
                config::ConfigError::Foreign(Box::new(error::ConsulError::Utf8Decode {
                    key:    entry.key.clone(),
                    source: e,
                }))
            })?;

            tracing::info!(key = %config_key, "Loaded config key from Consul KV");

            map.insert(
                config_key,
                Value::new(
                    Some(&"consul".to_string()),
                    ValueKind::String(value_str),
                ),
            );
        }

        tracing::info!(count = map.len(), "Loaded config entries from Consul KV");
        Ok(map)
    }
}

/// Fetch all KV entries under the configured prefix.
async fn fetch_kv_recursive(
    client: &reqwest::Client,
    cfg: &ConsulConfig,
) -> Result<Vec<KvEntry>, error::ConsulError> {
    let url = format!("{}/v1/kv/{}?recurse", cfg.addr, cfg.prefix);

    let mut req = client.get(&url);
    if let Some(ref token) = cfg.token {
        req = req.header("X-Consul-Token", token);
    }

    let resp = req.send().await.context(error::HttpRequestSnafu)?;

    // Consul returns 404 when the prefix has no keys
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        tracing::warn!(prefix = %cfg.prefix, "Consul KV prefix not found (empty)");
        return Ok(Vec::new());
    }

    let resp = resp
        .error_for_status()
        .map_err(|e| error::ConsulError::HttpStatus {
            message: e.to_string(),
        })?;

    let entries: Vec<KvEntry> = resp
        .json()
        .await
        .context(error::JsonDecodeSnafu)?;

    Ok(entries)
}

// ---------------------------------------------------------------------------
// Path mapping helpers (public for unit testing)
// ---------------------------------------------------------------------------

/// Convert a full Consul KV key to a nested config key by stripping the
/// prefix and replacing `/` with `.`.
///
/// Returns `None` if the key does not start with the prefix or is exactly
/// the prefix (a directory marker).
#[must_use]
pub fn kv_path_to_config_key(key: &str, prefix: &str) -> Option<String> {
    let relative = key.strip_prefix(prefix)?;
    if relative.is_empty() {
        return None;
    }
    Some(relative.replace('/', "."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_mapping_basic() {
        let prefix = "rara/config/";
        assert_eq!(
            kv_path_to_config_key("rara/config/database/database_url", prefix),
            Some("database.database_url".to_owned()),
        );
    }

    #[test]
    fn path_mapping_single_segment() {
        let prefix = "rara/config/";
        assert_eq!(
            kv_path_to_config_key("rara/config/some_key", prefix),
            Some("some_key".to_owned()),
        );
    }

    #[test]
    fn path_mapping_deep_nesting() {
        let prefix = "rara/config/";
        assert_eq!(
            kv_path_to_config_key("rara/config/a/b/c/d", prefix),
            Some("a.b.c.d".to_owned()),
        );
    }

    #[test]
    fn path_mapping_prefix_only_returns_none() {
        let prefix = "rara/config/";
        assert_eq!(kv_path_to_config_key("rara/config/", prefix), None);
    }

    #[test]
    fn path_mapping_wrong_prefix_returns_none() {
        let prefix = "rara/config/";
        assert_eq!(
            kv_path_to_config_key("other/prefix/key", prefix),
            None,
        );
    }

    #[test]
    fn path_mapping_custom_prefix() {
        let prefix = "myapp/settings/";
        assert_eq!(
            kv_path_to_config_key("myapp/settings/db/host", prefix),
            Some("db.host".to_owned()),
        );
    }
}
