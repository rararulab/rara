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

use diesel::{QueryDsl, TextExpressionMethods};
use diesel_async::RunQueryDsl;
use rara_model::schema::kv_table;
use snafu::Whatever;
use tokio::sync::watch;
use yunara_store::{KVStore, diesel_pool::DieselSqlitePools};

/// Internal prefix applied to all settings keys in the KV store.
const PREFIX: &str = "settings.";

/// Service that manages flat KV settings with SQLite persistence.
///
/// Implements
/// [`SettingsProvider`](rara_domain_shared::settings::SettingsProvider).
#[derive(Clone)]
pub struct SettingsSvc {
    kv:    KVStore,
    pools: DieselSqlitePools,
    tx:    Arc<watch::Sender<()>>,
}

impl SettingsSvc {
    /// Load settings from the flat KV store.
    pub async fn load(kv: KVStore, pools: DieselSqlitePools) -> Result<Self, Whatever> {
        let (tx, _rx) = watch::channel(());
        Ok(Self {
            kv,
            pools,
            tx: Arc::new(tx),
        })
    }

    /// Notify subscribers after a mutation.
    fn notify(&self) { let _ = self.tx.send(()); }
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
        let pattern = format!("{PREFIX}%");
        let mut conn = match self.pools.reader.get().await {
            Ok(c) => c,
            Err(_) => return HashMap::new(),
        };
        let rows: Vec<(String, Option<String>)> = match kv_table::table
            .filter(kv_table::key.like(pattern))
            .select((kv_table::key, kv_table::value))
            .load(&mut *conn)
            .await
        {
            Ok(rows) => rows,
            Err(_) => return HashMap::new(),
        };

        rows.into_iter()
            .filter_map(|(k, v)| {
                let v = v?;
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
