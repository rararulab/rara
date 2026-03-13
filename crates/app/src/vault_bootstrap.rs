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

use rara_vault::VaultClient;
use tracing::info;

use crate::AppConfig;

/// Pull config from Vault via the already-authenticated client and merge
/// dynamic sections into AppConfig.
///
/// Returns `Ok(true)` if values were merged, `Ok(false)` if Vault returned
/// no data.
pub async fn pull_and_merge(
    config: &mut AppConfig,
    client: &VaultClient,
) -> Result<bool, rara_vault::VaultError> {
    let pairs = client.pull_all().await?;
    if pairs.is_empty() {
        return Ok(false);
    }
    merge_vault_pairs_into_config(config, &pairs);
    info!(count = pairs.len(), "vault config pulled and merged");
    Ok(true)
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

}
