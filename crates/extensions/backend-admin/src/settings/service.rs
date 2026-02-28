// Copyright 2025 Crrow
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

//! Runtime-changeable settings backed by KV store.

use std::sync::{Arc, RwLock};

use chrono::Utc;
use rara_domain_shared::settings::model::{Settings, UpdateRequest};
use snafu::{ResultExt, Whatever, whatever};
use tokio::sync::watch;
use yunara_store::KVStore;

/// KV key for persisted runtime settings JSON.
pub const RUNTIME_SETTINGS_KV_KEY: &str = "runtime_settings.v1";

/// Service that manages runtime settings with KV persistence + in-memory cache.
///
/// Subscribers can call [`subscribe`](SettingsSvc::subscribe) to receive a
/// [`watch::Receiver<Settings>`] that is notified immediately whenever
/// [`update`](SettingsSvc::update) persists new settings.
#[derive(Clone)]
pub struct SettingsSvc {
    kv:    KVStore,
    cache: Arc<RwLock<Settings>>,
    tx:    Arc<watch::Sender<Settings>>,
}

impl SettingsSvc {
    /// Load from KV store, merging with `fallback` for any missing fields.
    pub async fn load(kv: KVStore) -> Result<Self, Whatever> {
        let mut stored = kv
            .get::<Settings>(RUNTIME_SETTINGS_KV_KEY)
            .await
            .whatever_context("failed to load runtime settings from kv")?
            .unwrap_or_default();
        stored.normalize();
        let (tx, _rx) = watch::channel(stored.clone());
        Ok(Self {
            kv,
            cache: Arc::new(RwLock::new(stored)),
            tx: Arc::new(tx),
        })
    }

    /// Snapshot of the current settings.
    pub fn current(&self) -> Settings {
        self.cache
            .read()
            .map_or_else(|_| Settings::default(), |g| g.clone())
    }

    /// Apply a partial update, persist to KV, and return the new snapshot.
    pub async fn update(&self, patch: UpdateRequest) -> Result<Settings, Whatever> {
        let mut next = self.current();
        next.apply_patch(patch);
        next.normalize();
        next.updated_at = Some(Utc::now());

        self.kv
            .set(RUNTIME_SETTINGS_KV_KEY, &next)
            .await
            .whatever_context("failed to persist runtime settings to kv")?;

        let mut guard = match self.cache.write() {
            Ok(guard) => guard,
            Err(_) => {
                whatever!("failed to lock runtime settings cache")
            }
        };
        *guard = next.clone();
        drop(guard);

        // Push to watch channel so subscribers see the update immediately.
        let _ = self.tx.send(next.clone());

        Ok(next)
    }

    /// Obtain a [`watch::Receiver`] that is notified on every settings update.
    ///
    /// The receiver's current value is always the latest committed snapshot.
    pub fn subscribe(&self) -> watch::Receiver<Settings> { self.tx.subscribe() }
}

#[async_trait::async_trait]
impl rara_domain_shared::settings::SettingsUpdater for SettingsSvc {
    async fn update_settings(&self, patch: UpdateRequest) -> anyhow::Result<Settings> {
        self.update(patch).await.map_err(|e| anyhow::anyhow!("{e}"))
    }
}
