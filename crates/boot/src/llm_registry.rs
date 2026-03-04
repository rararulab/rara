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

//! LLM driver registry construction.
//!
//! Auto-discovers providers from settings keys matching
//! `llm.providers.{name}.*` and registers an [`OpenAiDriver`] for each.

use std::{collections::BTreeSet, sync::Arc};

use tracing::info;

/// Build a [`DriverRegistry`](rara_kernel::llm::DriverRegistry) from
/// runtime settings.
///
/// Auto-discovers providers by scanning settings keys with the prefix
/// `llm.providers.`. For each discovered provider, registers an
/// `OpenAiDriver::from_settings` driver.
///
/// The `codex` provider is special-cased: it reads OAuth tokens from the
/// credential store rather than settings.
pub async fn build_driver_registry(
    settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
    credential_store: &dyn rara_keyring_store::KeyringStore,
) -> anyhow::Result<Arc<rara_kernel::llm::DriverRegistry>> {
    use rara_domain_shared::settings::keys;
    use rara_kernel::llm::{DriverRegistryBuilder, OpenAiDriver};

    let default_provider = settings
        .as_ref()
        .get_first(&[keys::LLM_DEFAULT_PROVIDER, keys::LLM_PROVIDER])
        .await
        .ok_or_else(|| {
            anyhow::anyhow!(
                "LLM default provider is not configured (checked: {}, {})",
                keys::LLM_DEFAULT_PROVIDER,
                keys::LLM_PROVIDER
            )
        })?;

    let mut builder = DriverRegistryBuilder::new(&default_provider);

    // -- auto-discover providers from settings --------------------------------

    let all_settings = settings.list().await;
    let provider_names: BTreeSet<&str> = all_settings
        .keys()
        .filter_map(|k| k.strip_prefix("llm.providers."))
        .filter_map(|k| k.split('.').next())
        .collect();

    for &name in &provider_names {
        builder = builder.driver(
            name,
            Arc::new(OpenAiDriver::from_settings(settings.clone(), name)),
        );

        // Read per-provider default_model and fallback_models
        let model_key = format!("llm.providers.{name}.default_model");
        if let Some(default_model) = all_settings.get(&model_key).filter(|v| !v.trim().is_empty())
        {
            let fallback_key = format!("llm.providers.{name}.fallback_models");
            let fallback_models: Vec<String> = all_settings
                .get(&fallback_key)
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            builder = builder.provider_model(name, default_model, fallback_models);
        }
    }

    // Validate that default_provider has a default_model configured
    let default_model_key = format!("llm.providers.{default_provider}.default_model");
    let default_model = all_settings
        .get(&default_model_key)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "LLM default model is not configured for provider '{default_provider}' \
                 (checked: {default_model_key})"
            )
        })?;

    // -- codex (OpenAI via OAuth) — special-cased -----------------------------

    match rara_codex_oauth::load_tokens(credential_store).await {
        Ok(Some(tokens)) => {
            builder = builder.driver(
                "codex",
                Arc::new(OpenAiDriver::new(
                    "https://api.openai.com/v1",
                    tokens.access_token,
                )),
            );
        }
        Ok(None) => {} // No tokens configured — skip
        Err(e) => tracing::warn!("failed to load codex OAuth tokens: {e}"),
    }

    info!(
        providers = ?provider_names,
        "driver registry: default_driver={default_provider}, default_model={default_model}",
    );
    Ok(Arc::new(builder.build()))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use async_trait::async_trait;
    use rara_domain_shared::settings::testing::MapSettingsProvider;

    use super::build_driver_registry;

    #[derive(Debug)]
    struct NoopKeyringStore;

    #[async_trait]
    impl rara_keyring_store::KeyringStore for NoopKeyringStore {
        async fn load(
            &self,
            _service: &str,
            _account: &str,
        ) -> rara_keyring_store::Result<Option<String>> {
            Ok(None)
        }

        async fn save(
            &self,
            _service: &str,
            _account: &str,
            _value: &str,
        ) -> rara_keyring_store::Result<()> {
            Ok(())
        }

        async fn delete(&self, _service: &str, _account: &str) -> rara_keyring_store::Result<bool> {
            Ok(false)
        }
    }

    #[tokio::test]
    async fn build_driver_registry_auto_discovers_providers() {
        let settings = MapSettingsProvider::new(HashMap::from([
            ("llm.default_provider".to_owned(), "ollama".to_owned()),
            (
                "llm.providers.ollama.base_url".to_owned(),
                "https://ollama.rara.local".to_owned(),
            ),
            (
                "llm.providers.ollama.api_key".to_owned(),
                "ollama".to_owned(),
            ),
            (
                "llm.providers.ollama.default_model".to_owned(),
                "qwen3.5:cloud".to_owned(),
            ),
            (
                "llm.providers.ollama.fallback_models".to_owned(),
                "qwen3:14b,llama3:8b".to_owned(),
            ),
        ]));

        let registry = build_driver_registry(Arc::new(settings), &NoopKeyringStore)
            .await
            .expect("driver registry should build");

        assert_eq!(registry.default_driver(), "ollama");
        assert_eq!(
            registry.default_model(),
            Some("qwen3.5:cloud".to_string())
        );
        assert_eq!(
            registry.fallback_models_for("ollama"),
            vec!["qwen3:14b".to_string(), "llama3:8b".to_string()]
        );
        assert!(registry.driver_names().contains(&"ollama".to_owned()));
    }

    #[tokio::test]
    async fn build_driver_registry_requires_default_provider_setting() {
        let settings = MapSettingsProvider::new(HashMap::from([(
            "llm.providers.ollama.default_model".to_owned(),
            "qwen3.5:cloud".to_owned(),
        )]));

        let err = build_driver_registry(Arc::new(settings), &NoopKeyringStore)
            .await
            .err()
            .expect("missing default provider should fail");

        assert!(err.to_string().contains("default provider"));
    }

    #[tokio::test]
    async fn build_driver_registry_requires_default_model_for_provider() {
        let settings = MapSettingsProvider::new(HashMap::from([
            ("llm.default_provider".to_owned(), "ollama".to_owned()),
            (
                "llm.providers.ollama.base_url".to_owned(),
                "https://ollama.rara.local".to_owned(),
            ),
            (
                "llm.providers.ollama.api_key".to_owned(),
                "ollama".to_owned(),
            ),
        ]));

        let err = build_driver_registry(Arc::new(settings), &NoopKeyringStore)
            .await
            .err()
            .expect("missing default model should fail");

        assert!(err.to_string().contains("default model"));
    }
}
