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

//! Runtime-changeable settings backed by flat KV store.
//!
//! Each setting is stored as a separate row in `kv_table` with a
//! `settings.` prefix (e.g. `settings.llm.provider`).

use std::{collections::HashMap, sync::Arc};

use snafu::{ResultExt, Whatever};
use sqlx::SqlitePool;
use tokio::sync::watch;
use tracing::info;
use yunara_store::KVStore;

/// Internal prefix applied to all settings keys in the KV store.
const PREFIX: &str = "settings.";

/// KV key used by the old nested-struct settings format.
const LEGACY_KV_KEY: &str = "runtime_settings.v1";

/// Service that manages flat KV settings with PostgreSQL persistence.
///
/// Implements
/// [`SettingsProvider`](rara_domain_shared::settings::SettingsProvider).
#[derive(Clone)]
pub struct SettingsSvc {
    kv:   KVStore,
    pool: SqlitePool,
    tx:   Arc<watch::Sender<()>>,
}

impl SettingsSvc {
    /// Load settings and perform one-time migration from the legacy
    /// nested format (if the old key still exists).
    pub async fn load(kv: KVStore, pool: SqlitePool) -> Result<Self, Whatever> {
        let (tx, _rx) = watch::channel(());
        let svc = Self {
            kv,
            pool,
            tx: Arc::new(tx),
        };
        svc.migrate_legacy().await?;
        Ok(svc)
    }

    /// If the old `runtime_settings.v1` blob exists, decompose it into
    /// flat KV pairs and delete the blob.
    async fn migrate_legacy(&self) -> Result<(), Whatever> {
        let blob: Option<legacy::Settings> = self
            .kv
            .get(LEGACY_KV_KEY)
            .await
            .whatever_context("failed to read legacy settings key")?;

        let Some(old) = blob else {
            return Ok(());
        };

        info!("migrating legacy runtime_settings.v1 to flat KV pairs");

        let pairs = legacy::flatten(&old);
        for (key, value) in &pairs {
            let prefixed = format!("{PREFIX}{key}");
            self.kv
                .set(&prefixed, &value.to_owned())
                .await
                .whatever_context("failed to write migrated setting")?;
        }

        self.kv
            .remove(LEGACY_KV_KEY)
            .await
            .whatever_context("failed to remove legacy settings key")?;

        info!(count = pairs.len(), "legacy settings migration complete");
        Ok(())
    }

    /// Notify subscribers after a mutation.
    fn notify(&self) { let _ = self.tx.send(()); }

    /// Return a `watch::Receiver<()>` for change notifications.
    pub fn watch_receiver(&self) -> watch::Receiver<()> { self.tx.subscribe() }
}

#[async_trait::async_trait]
impl rara_domain_shared::settings::SettingsProvider for SettingsSvc {
    async fn get(&self, key: &str) -> Option<String> {
        let prefixed = format!("{PREFIX}{key}");
        self.kv.get::<String>(&prefixed).await.ok().flatten()
    }

    async fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let prefixed = format!("{PREFIX}{key}");
        self.kv
            .set(&prefixed, &value.to_owned())
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        self.notify();
        Ok(())
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        let prefixed = format!("{PREFIX}{key}");
        self.kv
            .remove(&prefixed)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        self.notify();
        Ok(())
    }

    async fn list(&self) -> HashMap<String, String> {
        // Query all rows with the settings prefix.
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM kv_table WHERE key LIKE ?1")
                .bind(format!("{PREFIX}%"))
                .fetch_all(&self.pool)
                .await
                .unwrap_or_default();

        rows.into_iter()
            .filter_map(|(k, v)| {
                let stripped = k.strip_prefix(PREFIX)?;
                // Values are JSON-encoded strings in the KV store, so we
                // need to deserialize the outer JSON quotes.
                let plain: String = serde_json::from_str(&v).unwrap_or(v);
                Some((stripped.to_owned(), plain))
            })
            .collect()
    }

    async fn batch_update(&self, patches: HashMap<String, Option<String>>) -> anyhow::Result<()> {
        for (key, value) in &patches {
            let prefixed = format!("{PREFIX}{key}");
            match value {
                Some(v) => {
                    self.kv
                        .set(&prefixed, &v.to_owned())
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                None => {
                    self.kv
                        .remove(&prefixed)
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
            }
        }
        self.notify();
        Ok(())
    }

    fn subscribe(&self) -> watch::Receiver<()> { self.tx.subscribe() }
}

// ---------------------------------------------------------------------------
// Legacy settings types — kept only for migration
// ---------------------------------------------------------------------------

mod legacy {
    use std::collections::HashMap;

    use serde::Deserialize;

    #[derive(Debug, Clone, Default, Deserialize)]
    pub struct Settings {
        #[serde(default)]
        pub ai:       AISettings,
        #[serde(default)]
        pub telegram: TelegramSettings,
        #[serde(default)]
        pub agent:    AgentSettings,
    }

    #[derive(Debug, Clone, Default, Deserialize)]
    pub struct AISettings {
        pub openrouter_api_key: Option<String>,
        pub provider:           Option<String>,
        pub ollama_base_url:    Option<String>,
        #[serde(default)]
        pub models:             HashMap<String, String>,
        #[serde(default)]
        pub fallback_models:    Vec<String>,
        #[serde(default)]
        pub favorite_models:    Vec<String>,
    }

    #[derive(Debug, Clone, Default, Deserialize)]
    pub struct TelegramSettings {
        pub bot_token:               Option<String>,
        pub chat_id:                 Option<i64>,
        pub allowed_group_chat_id:   Option<i64>,
        pub notification_channel_id: Option<i64>,
    }

    #[derive(Debug, Clone, Default, Deserialize)]
    #[serde(default)]
    pub struct AgentSettings {
        pub memory:   MemorySettings,
        pub composio: ComposioSettings,
        pub gmail:    GmailSettings,
    }

    #[derive(Debug, Clone, Default, Deserialize)]
    #[serde(default)]
    pub struct MemorySettings {
        pub mem0_base_url:      Option<String>,
        pub memos_base_url:     Option<String>,
        pub memos_token:        Option<String>,
        pub hindsight_base_url: Option<String>,
        pub hindsight_bank_id:  Option<String>,
    }

    #[derive(Debug, Clone, Default, Deserialize)]
    #[serde(default)]
    pub struct ComposioSettings {
        pub api_key:   Option<String>,
        pub entity_id: Option<String>,
    }

    #[derive(Debug, Clone, Default, Deserialize)]
    #[serde(default)]
    pub struct GmailSettings {
        pub address:           Option<String>,
        pub app_password:      Option<String>,
        pub auto_send_enabled: bool,
    }

    /// Flatten a legacy Settings struct into a list of (key, value) pairs.
    pub fn flatten(s: &Settings) -> Vec<(String, String)> {
        let mut out = Vec::new();

        // AI settings
        if let Some(ref v) = s.ai.provider {
            out.push(("llm.provider".to_owned(), v.clone()));
        }
        if let Some(ref v) = s.ai.openrouter_api_key {
            out.push(("llm.openrouter.api_key".to_owned(), v.clone()));
        }
        if let Some(ref v) = s.ai.ollama_base_url {
            out.push(("llm.ollama.base_url".to_owned(), v.clone()));
        }
        for (k, v) in &s.ai.models {
            out.push((format!("llm.models.{k}"), v.clone()));
        }
        if !s.ai.fallback_models.is_empty() {
            out.push((
                "llm.fallback_models".to_owned(),
                serde_json::to_string(&s.ai.fallback_models).unwrap_or_default(),
            ));
        }
        if !s.ai.favorite_models.is_empty() {
            out.push((
                "llm.favorite_models".to_owned(),
                serde_json::to_string(&s.ai.favorite_models).unwrap_or_default(),
            ));
        }

        // Telegram settings
        if let Some(ref v) = s.telegram.bot_token {
            out.push(("telegram.bot_token".to_owned(), v.clone()));
        }
        if let Some(v) = s.telegram.chat_id {
            out.push(("telegram.chat_id".to_owned(), v.to_string()));
        }
        if let Some(v) = s.telegram.allowed_group_chat_id {
            out.push(("telegram.allowed_group_chat_id".to_owned(), v.to_string()));
        }
        if let Some(v) = s.telegram.notification_channel_id {
            out.push(("telegram.notification_channel_id".to_owned(), v.to_string()));
        }

        // Composio settings
        if let Some(ref v) = s.agent.composio.api_key {
            out.push(("composio.api_key".to_owned(), v.clone()));
        }
        if let Some(ref v) = s.agent.composio.entity_id {
            out.push(("composio.entity_id".to_owned(), v.clone()));
        }

        // Gmail settings
        if let Some(ref v) = s.agent.gmail.address {
            out.push(("gmail.address".to_owned(), v.clone()));
        }
        if let Some(ref v) = s.agent.gmail.app_password {
            out.push(("gmail.app_password".to_owned(), v.clone()));
        }
        if s.agent.gmail.auto_send_enabled {
            out.push(("gmail.auto_send_enabled".to_owned(), "true".to_owned()));
        }

        // Memory settings
        if let Some(ref v) = s.agent.memory.mem0_base_url {
            out.push(("memory.mem0.base_url".to_owned(), v.clone()));
        }
        if let Some(ref v) = s.agent.memory.memos_base_url {
            out.push(("memory.memos.base_url".to_owned(), v.clone()));
        }
        if let Some(ref v) = s.agent.memory.memos_token {
            out.push(("memory.memos.token".to_owned(), v.clone()));
        }
        if let Some(ref v) = s.agent.memory.hindsight_base_url {
            out.push(("memory.hindsight.base_url".to_owned(), v.clone()));
        }
        if let Some(ref v) = s.agent.memory.hindsight_bank_id {
            out.push(("memory.hindsight.bank_id".to_owned(), v.clone()));
        }

        out
    }
}
