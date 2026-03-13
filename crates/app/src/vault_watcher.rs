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

//! Background task that polls Vault for config changes and syncs to settings
//! KV.

use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use rara_domain_shared::settings::SettingsProvider;
use rara_vault::{VaultClient, VaultConfig};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub fn spawn_vault_watcher(
    vault_config: VaultConfig,
    settings: Arc<dyn SettingsProvider>,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        let interval = vault_config.watch_interval;
        let client = match VaultClient::new(vault_config) {
            Ok(client) => client,
            Err(error) => {
                warn!(error = %error, "vault watcher: failed to build client");
                return;
            }
        };

        let mut last_versions = HashMap::new();
        match client.login().await {
            Ok(()) => match fetch_versions(&client).await {
                Ok(versions) => {
                    last_versions = versions;
                }
                Err(error) => {
                    warn!(error = %error, "vault watcher: failed to read initial metadata");
                }
            },
            Err(error) => {
                warn!(error = %error, "vault watcher: initial login failed, will retry");
            }
        }

        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("vault watcher stopped");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(error) = poll_and_sync(&client, &settings, &mut last_versions).await {
                        warn!(error = %error, "vault watcher: poll failed, will retry next interval");
                    }
                }
            }
        }
    });
}

fn build_settings_patches(
    current: &HashMap<String, String>,
    vault_pairs: &[(String, String)],
) -> HashMap<String, Option<String>> {
    let mut patches = HashMap::new();
    for (key, value) in vault_pairs {
        match current.get(key) {
            Some(existing) if existing == value => {}
            _ => {
                patches.insert(key.clone(), Some(value.clone()));
            }
        }
    }
    patches
}

fn versions_changed(previous: &HashMap<String, u64>, current: &HashMap<String, u64>) -> bool {
    previous != current
}

async fn poll_and_sync(
    client: &VaultClient,
    settings: &Arc<dyn SettingsProvider>,
    last_versions: &mut HashMap<String, u64>,
) -> anyhow::Result<()> {
    ensure_authenticated(client).await?;

    let current_versions = fetch_versions(client).await?;
    if !versions_changed(last_versions, &current_versions) {
        debug!("vault watcher: no metadata changes detected");
        return Ok(());
    }

    let vault_pairs = client.pull_all().await?;
    let current_settings = settings.list().await;
    let patches = build_settings_patches(&current_settings, &vault_pairs);
    if patches.is_empty() {
        debug!("vault watcher: metadata changed but settings payload is unchanged");
    } else {
        info!(
            count = patches.len(),
            "vault watcher: applying config changes from vault"
        );
        settings
            .batch_update(patches)
            .await
            .context("failed to apply vault settings patches")?;
    }

    *last_versions = current_versions;
    Ok(())
}

async fn ensure_authenticated(client: &VaultClient) -> Result<(), rara_vault::VaultError> {
    if client.token_needs_renewal().await {
        match client.renew_token().await {
            Ok(()) => {}
            Err(error) => {
                warn!(error = %error, "vault watcher: token renewal failed, re-logging in");
                client.login().await?;
            }
        }
    }
    Ok(())
}

async fn fetch_versions(
    client: &VaultClient,
) -> Result<HashMap<String, u64>, rara_vault::VaultError> {
    let mut versions = HashMap::new();
    for prefix in ["config", "secrets"] {
        for key in client.list_secrets(prefix).await? {
            if key.ends_with('/') {
                continue;
            }
            let path = format!("{prefix}/{key}");
            let metadata = client.get_metadata(&path).await?;
            versions.insert(path, metadata.version);
        }
    }
    Ok(versions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_settings_patches_updates_changed_values() {
        let current = HashMap::from([
            ("llm.default_provider".to_string(), "local".to_string()),
            (
                "llm.providers.local.base_url".to_string(),
                "http://localhost:11434".to_string(),
            ),
        ]);
        let vault_pairs = vec![
            ("llm.default_provider".to_string(), "vault".to_string()),
            (
                "llm.providers.vault.base_url".to_string(),
                "http://vault:1234".to_string(),
            ),
        ];

        let patches = build_settings_patches(&current, &vault_pairs);

        assert_eq!(
            patches.get("llm.default_provider"),
            Some(&Some("vault".to_string())),
        );
        assert_eq!(
            patches.get("llm.providers.vault.base_url"),
            Some(&Some("http://vault:1234".to_string())),
        );
    }

    #[test]
    fn versions_changed_detects_metadata_updates() {
        let previous = HashMap::from([
            ("config/llm".to_string(), 1_u64),
            ("secrets/telegram".to_string(), 1_u64),
        ]);
        let current = HashMap::from([
            ("config/llm".to_string(), 2_u64),
            ("secrets/telegram".to_string(), 1_u64),
        ]);

        assert!(versions_changed(&previous, &current));
    }
}
