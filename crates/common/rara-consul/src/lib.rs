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

use std::convert::Infallible;

use config::{Map, Value, ValueKind};
pub use rs_consul::Config;
use rs_consul::{ConsulBuilder, types::ReadKeyRequest};

// ---------------------------------------------------------------------------
// ConsulSource — config::AsyncSource implementation
// ---------------------------------------------------------------------------

const PREFIX: &str = "rara/config/";

/// Consul KV as a [`config`] crate async source.
///
/// Fetches all keys under `rara/config/` using [`rs_consul::Consul`]
/// and maps KV paths to nested config keys. For example,
/// `rara/config/database/database_url` becomes `database.database_url`.
#[derive(Debug)]
pub struct ConsulSource {
    config: Config,
}

impl ConsulSource {
    pub fn new(config: Config) -> Self { Self { config } }
}

/// Build a [`hyper_rustls::HttpsConnector`] that trusts the OS certificate
/// store (macOS Keychain, Windows cert store, Linux system CAs) via
/// [`rustls_platform_verifier`].
fn platform_https_connector()
-> hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector> {
    use rustls_platform_verifier::ConfigVerifierExt;

    let tls_config = rustls::ClientConfig::with_platform_verifier()
        .expect("failed to build rustls ClientConfig with platform verifier");

    hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .build()
}

/// Build a custom [`rs_consul::HttpsClient`] that uses the platform
/// certificate verifier instead of the default `webpki-roots`.
fn build_platform_https_client(config: &Config) -> rs_consul::HttpsClient {
    let connector = platform_https_connector();
    config
        .hyper_builder
        .build::<_, http_body_util::combinators::BoxBody<bytes::Bytes, Infallible>>(connector)
}

#[async_trait::async_trait]
impl config::AsyncSource for ConsulSource {
    async fn collect(&self) -> Result<Map<String, Value>, config::ConfigError> {
        let https_client = build_platform_https_client(&self.config);
        let consul = ConsulBuilder::new(self.config.clone())
            .with_https_client(https_client)
            .build();

        let request = ReadKeyRequest {
            key: PREFIX,
            recurse: true,
            ..Default::default()
        };

        let entries = match consul.read_key(request).await {
            Ok(response) => response.response,
            Err(rs_consul::ConsulError::UnexpectedResponseCode(status, _))
                if status.as_u16() == 404 =>
            {
                tracing::warn!(prefix = PREFIX, "Consul KV prefix not found (empty)");
                return Ok(Map::new());
            }
            Err(e) => return Err(config::ConfigError::Foreign(Box::new(e))),
        };

        let mut map = Map::new();
        for entry in &entries {
            // rs-consul already decodes base64 values
            let Some(ref value) = entry.value else {
                continue; // directory marker
            };

            let Some(config_key) = kv_path_to_config_key(&entry.key, PREFIX) else {
                continue;
            };

            tracing::info!(key = %config_key, "Loaded config key from Consul KV");

            map.insert(
                config_key,
                Value::new(
                    Some(&"consul".to_string()),
                    ValueKind::String(value.clone()),
                ),
            );
        }

        tracing::info!(count = map.len(), "Loaded config entries from Consul KV");
        Ok(map)
    }
}

// ---------------------------------------------------------------------------
// Path mapping helper
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
