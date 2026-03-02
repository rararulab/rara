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

//! LLM provider registry construction and Composio auth provider.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::info;

/// Build a [`ProviderRegistry`](rara_kernel::provider::ProviderRegistry) from
/// runtime settings.
///
/// Reads `llm.provider` (default: `"openrouter"`) and `llm.models.default`
/// to determine the default provider and model. Then registers all
/// available providers based on configured API keys / base URLs.
pub async fn build_provider_registry(
    settings: &dyn rara_domain_shared::settings::SettingsProvider,
    credential_store: &dyn rara_keyring_store::KeyringStore,
) -> Arc<rara_kernel::provider::ProviderRegistry> {
    use rara_domain_shared::settings::keys;
    use rara_kernel::provider::{OpenAiProvider, ProviderRegistryBuilder};

    let default_provider = settings
        .get(keys::LLM_PROVIDER)
        .await
        .unwrap_or_else(|| "openrouter".to_owned());
    let default_model = settings
        .get(keys::LLM_MODELS_DEFAULT)
        .await
        .unwrap_or_else(|| "openai/gpt-4o-mini".to_owned());

    let mut builder = ProviderRegistryBuilder::new(&default_provider, &default_model);

    // -- openrouter ---------------------------------------------------------
    if let Some(api_key) = settings.get(keys::LLM_OPENROUTER_API_KEY).await {
        builder = builder.provider("openrouter", Arc::new(OpenAiProvider::new(api_key)));
    }

    // -- ollama -------------------------------------------------------------
    {
        let base_url = settings
            .get(keys::LLM_OLLAMA_BASE_URL)
            .await
            .unwrap_or_else(|| "http://localhost:11434".to_owned());
        let config = async_openai::config::OpenAIConfig::new()
            .with_api_base(format!("{}/v1", base_url))
            .with_api_key("ollama");
        builder = builder.provider("ollama", Arc::new(OpenAiProvider::with_config(config)));
    }

    // -- codex (OpenAI via OAuth) -------------------------------------------
    if let Ok(Some(tokens)) = rara_codex_oauth::load_tokens(credential_store).await {
        let config = async_openai::config::OpenAIConfig::new().with_api_key(&tokens.access_token);
        builder = builder.provider("codex", Arc::new(OpenAiProvider::with_config(config)));
    }

    info!("provider registry: default_provider={default_provider}, default_model={default_model}");
    Arc::new(builder.build())
}

/// Composio auth provider that reads credentials from runtime settings.
#[derive(Clone)]
pub struct SettingsComposioAuthProvider {
    settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
}

impl SettingsComposioAuthProvider {
    pub fn new(settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>) -> Self {
        Self { settings }
    }
}

#[async_trait]
impl rara_composio::ComposioAuthProvider for SettingsComposioAuthProvider {
    async fn acquire_auth(&self) -> anyhow::Result<rara_composio::ComposioAuth> {
        use rara_domain_shared::settings::keys;
        let api_key = self
            .settings
            .get(keys::COMPOSIO_API_KEY)
            .await
            .ok_or_else(|| anyhow::anyhow!("composio.api_key is not configured in settings"))?;
        let entity_id = self.settings.get(keys::COMPOSIO_ENTITY_ID).await;
        Ok(rara_composio::ComposioAuth::new(
            api_key,
            entity_id.as_deref(),
        ))
    }
}

/// Convenience: build a Composio auth provider from settings.
pub fn composio_auth_provider(
    settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
) -> Arc<dyn rara_composio::ComposioAuthProvider> {
    Arc::new(SettingsComposioAuthProvider::new(settings))
}
