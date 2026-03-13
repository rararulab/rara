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

//! Vault bootstrap: pull config from Vault at startup and merge into AppConfig.

use std::collections::HashMap;

use rara_vault::{VaultClient, VaultConfig};
use tracing::{error, info, warn};

use crate::AppConfig;

/// Attempt to pull config from Vault and merge dynamic sections into AppConfig.
pub async fn pull_and_merge(config: &mut AppConfig) -> Result<bool, rara_vault::VaultError> {
    let Some(vault_config) = config.vault.clone() else {
        return Ok(false);
    };

    match try_pull(&vault_config).await {
        Ok(pairs) => {
            merge_vault_pairs_into_config(config, &pairs);
            info!(count = pairs.len(), "vault config pulled and merged");
            Ok(true)
        }
        Err(error) if vault_config.fallback_to_local => {
            warn!(error = %error, "vault unreachable, falling back to local config");
            Ok(false)
        }
        Err(error) => {
            error!(error = %error, "vault unreachable and fallback_to_local is false");
            Err(error)
        }
    }
}

async fn try_pull(
    vault_config: &VaultConfig,
) -> Result<Vec<(String, String)>, rara_vault::VaultError> {
    let client = VaultClient::new(vault_config.clone())?;
    client.login().await?;
    client.pull_all().await
}

fn merge_vault_pairs_into_config(config: &mut AppConfig, pairs: &[(String, String)]) {
    let settings_map: HashMap<String, String> = pairs.iter().cloned().collect();
    let (llm, telegram, composio, knowledge) =
        crate::flatten::unflatten_from_settings(&settings_map);

    if llm.is_some() {
        config.llm = llm;
    }
    if telegram.is_some() {
        config.telegram = telegram;
    }
    if composio.is_some() {
        config.composio = composio;
    }
    if knowledge.is_some() {
        config.knowledge = knowledge;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_vault_pairs_overrides_llm() {
        let yaml = r#"
users:
  - name: test
    role: root
    platforms: []
http:
  bind_address: "127.0.0.1:25555"
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
mita:
  heartbeat_interval: "30m"
llm:
  default_provider: "local-ollama"
"#;
        let mut config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.llm.as_ref().unwrap().default_provider.as_deref(),
            Some("local-ollama")
        );

        let pairs = vec![
            (
                "llm.default_provider".to_string(),
                "vault-provider".to_string(),
            ),
            (
                "llm.providers.vault-provider.base_url".to_string(),
                "http://vault:1234".to_string(),
            ),
            (
                "llm.providers.vault-provider.api_key".to_string(),
                "sk-vault".to_string(),
            ),
            (
                "llm.providers.vault-provider.default_model".to_string(),
                "gpt-4".to_string(),
            ),
        ];
        merge_vault_pairs_into_config(&mut config, &pairs);

        assert_eq!(
            config.llm.as_ref().unwrap().default_provider.as_deref(),
            Some("vault-provider")
        );
        assert!(
            config
                .llm
                .as_ref()
                .unwrap()
                .providers
                .contains_key("vault-provider")
        );
    }

    #[test]
    fn merge_vault_pairs_does_not_override_when_empty() {
        let yaml = r#"
users:
  - name: test
    role: root
    platforms: []
http:
  bind_address: "127.0.0.1:25555"
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
mita:
  heartbeat_interval: "30m"
llm:
  default_provider: "local-ollama"
"#;
        let mut config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        merge_vault_pairs_into_config(&mut config, &[]);
        assert_eq!(
            config.llm.as_ref().unwrap().default_provider.as_deref(),
            Some("local-ollama")
        );
    }

    #[tokio::test]
    async fn pull_and_merge_returns_false_when_no_vault_configured() {
        let yaml = r#"
users:
  - name: test
    role: root
    platforms: []
http:
  bind_address: "127.0.0.1:25555"
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
mita:
  heartbeat_interval: "30m"
"#;
        let mut config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        let result = pull_and_merge(&mut config).await;
        assert!(matches!(result, Ok(false)));
    }
}
