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

use async_trait::async_trait;

use crate::normalize_entity_id;

/// Runtime auth snapshot used for a single Composio request.
#[derive(Debug, Clone)]
pub struct ComposioAuth {
    pub api_key:           String,
    pub default_entity_id: String,
}

impl ComposioAuth {
    pub fn new(api_key: impl Into<String>, default_entity_id: Option<&str>) -> Self {
        Self {
            api_key:           api_key.into(),
            default_entity_id: normalize_entity_id(default_entity_id.unwrap_or("default")),
        }
    }
}

/// Auth provider abstraction for Composio credentials.
#[async_trait]
pub trait ComposioAuthProvider: Send + Sync {
    async fn acquire_auth(&self) -> anyhow::Result<ComposioAuth>;
}

/// Fixed auth provider (useful for tests and static configuration).
#[derive(Debug, Clone)]
pub struct StaticComposioAuthProvider {
    auth: ComposioAuth,
}

impl StaticComposioAuthProvider {
    pub fn new(api_key: impl Into<String>, default_entity_id: Option<&str>) -> Self {
        Self {
            auth: ComposioAuth::new(api_key, default_entity_id),
        }
    }
}

#[async_trait]
impl ComposioAuthProvider for StaticComposioAuthProvider {
    async fn acquire_auth(&self) -> anyhow::Result<ComposioAuth> { Ok(self.auth.clone()) }
}

/// Environment-backed auth provider.
#[derive(Debug, Clone, Default)]
pub struct EnvComposioAuthProvider;

#[async_trait]
impl ComposioAuthProvider for EnvComposioAuthProvider {
    async fn acquire_auth(&self) -> anyhow::Result<ComposioAuth> {
        let api_key = std::env::var("COMPOSIO_API_KEY")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("COMPOSIO_API_KEY is not configured"))?;
        let entity_id = std::env::var("COMPOSIO_ENTITY_ID")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        Ok(ComposioAuth::new(api_key, entity_id.as_deref()))
    }
}
