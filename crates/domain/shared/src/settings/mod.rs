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

//! Flat key-value settings contract shared across all crates.
//!
//! The [`SettingsProvider`] trait defines a simple get/set/delete/list
//! interface backed by a KV store. Well-known keys are declared in the
//! [`keys`] module.

use std::collections::HashMap;

/// Well-known setting key constants.
pub mod keys {
    pub const LLM_DEFAULT_PROVIDER: &str = "llm.default_provider";
    pub const LLM_PROVIDER: &str = "llm.provider";
    pub const LLM_PROVIDERS_OPENROUTER_BASE_URL: &str = "llm.providers.openrouter.base_url";
    pub const LLM_PROVIDERS_OPENROUTER_API_KEY: &str = "llm.providers.openrouter.api_key";
    pub const LLM_PROVIDERS_OLLAMA_API_KEY: &str = "llm.providers.ollama.api_key";
    pub const LLM_PROVIDERS_OLLAMA_BASE_URL: &str = "llm.providers.ollama.base_url";
    pub const LLM_MODELS_DEFAULT: &str = "llm.models.default";
    pub const LLM_MODELS_CHAT: &str = "llm.models.chat";
    pub const LLM_MODELS_JOB: &str = "llm.models.job";
    pub const LLM_FALLBACK_MODELS: &str = "llm.fallback_models";
    pub const LLM_FAVORITE_MODELS: &str = "llm.favorite_models";
    pub const TELEGRAM_BOT_TOKEN: &str = "telegram.bot_token";
    pub const TELEGRAM_CHAT_ID: &str = "telegram.chat_id";
    pub const TELEGRAM_ALLOWED_GROUP_CHAT_ID: &str = "telegram.allowed_group_chat_id";
    pub const TELEGRAM_NOTIFICATION_CHANNEL_ID: &str = "telegram.notification_channel_id";
    pub const GMAIL_ADDRESS: &str = "gmail.address";
    pub const GMAIL_APP_PASSWORD: &str = "gmail.app_password";
    pub const GMAIL_AUTO_SEND_ENABLED: &str = "gmail.auto_send_enabled";
    pub const COMPOSIO_API_KEY: &str = "composio.api_key";
    pub const COMPOSIO_ENTITY_ID: &str = "composio.entity_id";
    pub const MEMORY_MEM0_BASE_URL: &str = "memory.mem0.base_url";
    pub const MEMORY_MEMOS_BASE_URL: &str = "memory.memos.base_url";
    pub const MEMORY_MEMOS_TOKEN: &str = "memory.memos.token";
    pub const MEMORY_HINDSIGHT_BASE_URL: &str = "memory.hindsight.base_url";
    pub const MEMORY_HINDSIGHT_BANK_ID: &str = "memory.hindsight.bank_id";
    pub const FS_ALLOWED_DIRECTORIES: &str = "fs.allowed_directories";
    pub const FS_READ_ONLY_DIRECTORIES: &str = "fs.read_only_directories";
    pub const FS_DENIED_DIRECTORIES: &str = "fs.denied_directories";
}

/// Unified trait for reading and writing flat KV settings.
///
/// Implementations are expected to be backed by a durable store (e.g.
/// PostgreSQL `kv_table`) and to notify subscribers on mutation.
#[async_trait::async_trait]
pub trait SettingsProvider: Send + Sync {
    /// Get a single setting by key. Returns `None` if not set.
    async fn get(&self, key: &str) -> Option<String>;

    /// Get the first non-empty value from the provided key list.
    async fn get_first(&self, keys: &[&str]) -> Option<String> {
        for key in keys {
            if let Some(value) = self.get(key).await {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_owned());
                }
            }
        }
        None
    }

    /// Set a single setting. Creates or overwrites.
    async fn set(&self, key: &str, value: &str) -> anyhow::Result<()>;

    /// Delete a single setting.
    async fn delete(&self, key: &str) -> anyhow::Result<()>;

    /// List all settings as a flat key-value map.
    async fn list(&self) -> HashMap<String, String>;

    /// Batch update: for each entry, `Some(value)` sets the key,
    /// `None` deletes it. Notifies subscribers once after all mutations.
    async fn batch_update(&self, patches: HashMap<String, Option<String>>) -> anyhow::Result<()>;

    /// Subscribe to change notifications. The receiver is signaled
    /// (with `()`) whenever any setting is mutated.
    fn subscribe(&self) -> tokio::sync::watch::Receiver<()>;
}

/// Resolve the model for the given key, falling back to `llm.models.default`.
pub async fn get_model(settings: &dyn SettingsProvider, key: &str) -> Option<String> {
    let specific = format!("llm.models.{key}");
    if let Some(v) = settings.get(&specific).await {
        return Some(v);
    }
    settings.get(keys::LLM_MODELS_DEFAULT).await
}

/// Test utilities for settings.
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use tokio::sync::RwLock;

    use super::SettingsProvider;

    /// In-memory [`SettingsProvider`] backed by a `HashMap`.
    ///
    /// Useful for unit tests that need a settings provider without a database.
    #[derive(Default)]
    pub struct MapSettingsProvider {
        values: RwLock<HashMap<String, String>>,
    }

    impl MapSettingsProvider {
        pub fn new(values: HashMap<String, String>) -> Self {
            Self {
                values: RwLock::new(values),
            }
        }
    }

    #[async_trait]
    impl SettingsProvider for MapSettingsProvider {
        async fn get(&self, key: &str) -> Option<String> {
            self.values.read().await.get(key).cloned()
        }

        async fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
            self.values
                .write()
                .await
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }

        async fn delete(&self, key: &str) -> anyhow::Result<()> {
            self.values.write().await.remove(key);
            Ok(())
        }

        async fn list(&self) -> HashMap<String, String> { self.values.read().await.clone() }

        async fn batch_update(
            &self,
            patches: HashMap<String, Option<String>>,
        ) -> anyhow::Result<()> {
            let mut values = self.values.write().await;
            for (key, value) in patches {
                match value {
                    Some(value) => {
                        values.insert(key, value);
                    }
                    None => {
                        values.remove(&key);
                    }
                }
            }
            Ok(())
        }

        fn subscribe(&self) -> tokio::sync::watch::Receiver<()> {
            let (_tx, rx) = tokio::sync::watch::channel(());
            rx
        }
    }
}

#[cfg(test)]
mod tests {
    use super::keys;

    #[test]
    fn llm_provider_keys_include_shared_provider_constants() {
        assert_eq!(keys::LLM_DEFAULT_PROVIDER, "llm.default_provider");
        assert_eq!(
            keys::LLM_PROVIDERS_OPENROUTER_BASE_URL,
            "llm.providers.openrouter.base_url"
        );
        assert_eq!(
            keys::LLM_PROVIDERS_OPENROUTER_API_KEY,
            "llm.providers.openrouter.api_key"
        );
        assert_eq!(
            keys::LLM_PROVIDERS_OLLAMA_BASE_URL,
            "llm.providers.ollama.base_url"
        );
        assert_eq!(
            keys::LLM_PROVIDERS_OLLAMA_API_KEY,
            "llm.providers.ollama.api_key"
        );
    }
}
