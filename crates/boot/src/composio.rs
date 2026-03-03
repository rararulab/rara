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

//! Composio auth provider backed by runtime settings.

use std::sync::Arc;

use async_trait::async_trait;

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
