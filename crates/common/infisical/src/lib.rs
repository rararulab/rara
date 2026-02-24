pub mod config_types;
pub mod error;

use config::{Map, Value, ValueKind};
use snafu::ResultExt;

/// Infisical as a `config` crate async source.
///
/// Fetches secrets from Infisical and provides them as nested config keys.
/// Secret keys use `__` as a nesting separator (e.g. `DATABASE__DATABASE_URL`
/// becomes `database.database_url` in the config map).
#[derive(Debug)]
pub struct InfisicalSource {
    cfg: config_types::InfisicalConfig,
}

impl InfisicalSource {
    pub fn new(cfg: config_types::InfisicalConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait::async_trait]
impl config::AsyncSource for InfisicalSource {
    async fn collect(&self) -> Result<Map<String, Value>, config::ConfigError> {
        let secrets = fetch_secrets(&self.cfg)
            .await
            .map_err(|e| config::ConfigError::Foreign(Box::new(e)))?;

        let mut map = Map::new();
        for secret in &secrets {
            // Convert secret key to nested config format:
            // "DATABASE__DATABASE_URL" → "database.database_url"
            // The config crate uses "." for nesting in Map keys.
            let key = secret.secret_key.to_lowercase();
            let nested_key = key.replace("__", ".");
            map.insert(
                nested_key,
                Value::new(
                    Some(&"infisical".to_string()),
                    ValueKind::String(secret.secret_value.clone()),
                ),
            );
        }

        tracing::info!("Loaded {} secrets from Infisical", map.len());
        Ok(map)
    }
}

/// Internal: fetch secrets from Infisical.
async fn fetch_secrets(
    cfg: &config_types::InfisicalConfig,
) -> Result<Vec<infisical::secrets::Secret>, error::InfisicalError> {
    let mut client = infisical::Client::builder()
        .base_url(&cfg.base_url)
        .build()
        .await
        .context(error::ClientBuildSnafu)?;

    let auth =
        infisical::AuthMethod::new_universal_auth(&cfg.client_id, &cfg.client_secret);
    client.login(auth).await.context(error::AuthSnafu)?;

    let request = infisical::secrets::ListSecretsRequest::builder(
        &cfg.project_id,
        &cfg.environment,
    )
    .path(&cfg.secret_path)
    .recursive(true)
    .build();

    client
        .secrets()
        .list(request)
        .await
        .context(error::ListSecretsSnafu)
}
