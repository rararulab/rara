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

//! Runtime-changeable settings backed by the MVCC `settings_version` table.
//!
//! Each mutation appends a row with a monotonically increasing global version.
//! `value = NULL` represents a tombstone (deleted key). Current state is the
//! snapshot at the maximum version.

use std::{collections::HashMap, sync::Arc};

use rara_domain_shared::settings::SettingsProvider;
use snafu::Whatever;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use tokio::sync::watch;

/// Internal prefix applied to all settings keys in the version table.
const PREFIX: &str = "settings.";

/// Settings service backed entirely by the MVCC `settings_version` table.
///
/// Implements
/// [`SettingsProvider`].
#[derive(Clone)]
pub struct SettingsSvc {
    pool: SqlitePool,
    tx:   Arc<watch::Sender<()>>,
}

impl SettingsSvc {
    /// Load settings service with the given database pool.
    pub async fn load(pool: SqlitePool) -> Result<Self, Whatever> {
        let (tx, _rx) = watch::channel(());
        Ok(Self {
            pool,
            tx: Arc::new(tx),
        })
    }

    /// Notify subscribers after a mutation.
    fn notify(&self) { let _ = self.tx.send(()); }

    /// Bump global version counter atomically, return the new version.
    async fn next_version(&self) -> anyhow::Result<i64> {
        let (ver,): (i64,) = sqlx::query_as(
            "UPDATE settings_version_counter SET current = current + 1 WHERE id = 1 RETURNING \
             current",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(ver)
    }

    /// Append version log entries. `value = None` means tombstone (delete).
    async fn append_version_log(
        &self,
        version: i64,
        entries: &[(String, Option<String>)],
    ) -> anyhow::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut builder =
            QueryBuilder::<Sqlite>::new("INSERT INTO settings_version (version, key, value) ");
        builder.push_values(entries, |mut row, (key, value)| {
            row.push_bind(version).push_bind(key).push_bind(value);
        });
        builder.build().execute(&self.pool).await?;
        Ok(())
    }

    /// Read the current global version number.
    pub async fn current_version(&self) -> anyhow::Result<i64> {
        let (ver,): (i64,) =
            sqlx::query_as("SELECT current FROM settings_version_counter WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        Ok(ver)
    }

    /// Snapshot all settings at a given version (MVCC point-in-time read).
    pub async fn snapshot(&self, version: i64) -> anyhow::Result<HashMap<String, String>> {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT sv.key, sv.value
             FROM settings_version sv
             INNER JOIN (
                 SELECT key, MAX(version) AS max_ver
                 FROM settings_version
                 WHERE version <= ?1
                 GROUP BY key
             ) latest ON sv.key = latest.key AND sv.version = latest.max_ver
             WHERE sv.value IS NOT NULL",
        )
        .bind(version)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|(k, v)| {
                let stripped = k.strip_prefix(PREFIX)?;
                let plain: String = serde_json::from_str(&v?).unwrap_or_default();
                Some((stripped.to_owned(), plain))
            })
            .collect())
    }

    /// List version log entries.
    pub async fn list_versions(&self, limit: i64) -> anyhow::Result<Vec<VersionEntry>> {
        let rows: Vec<VersionEntry> = sqlx::query_as(
            "SELECT version, key, value, changed_at
             FROM settings_version
             ORDER BY version DESC, key ASC
             LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Rollback to a specific version (forward operation — creates a new
    /// version).
    pub async fn rollback_to(&self, target_version: i64) -> anyhow::Result<i64> {
        let target_snap = self.snapshot(target_version).await?;
        let current = self.list().await;

        let mut patches = HashMap::new();
        for (k, v) in &target_snap {
            if current.get(k) != Some(v) {
                patches.insert(k.clone(), Some(v.clone()));
            }
        }
        for k in current.keys() {
            if !target_snap.contains_key(k.as_str()) {
                patches.insert(k.clone(), None);
            }
        }

        if patches.is_empty() {
            return self.current_version().await;
        }

        self.batch_update(patches).await?;
        self.current_version().await
    }
}

/// A single entry in the version log.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct VersionEntry {
    /// The version number this entry belongs to.
    pub version:    i64,
    /// The settings key (with `settings.` prefix).
    pub key:        String,
    /// The value, or `None` for tombstones.
    pub value:      Option<String>,
    /// When this entry was created.
    pub changed_at: String,
}

#[async_trait::async_trait]
impl rara_domain_shared::settings::SettingsProvider for SettingsSvc {
    async fn get(&self, key: &str) -> Option<String> {
        let prefixed = format!("{PREFIX}{key}");
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT value FROM settings_version
             WHERE key = ?1
             ORDER BY version DESC LIMIT 1",
        )
        .bind(&prefixed)
        .fetch_optional(&self.pool)
        .await
        .ok()?;

        row.and_then(|(v,)| {
            let raw = v?; // None = tombstone — treat as absent
            serde_json::from_str(&raw).ok()
        })
    }

    async fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let prefixed = format!("{PREFIX}{key}");
        let ver = self.next_version().await?;
        let json_val = serde_json::to_string(&value)?;
        self.append_version_log(ver, &[(prefixed, Some(json_val))])
            .await?;
        self.notify();
        Ok(())
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        let prefixed = format!("{PREFIX}{key}");
        let ver = self.next_version().await?;
        self.append_version_log(ver, &[(prefixed, None)]).await?;
        self.notify();
        Ok(())
    }

    async fn list(&self) -> HashMap<String, String> {
        let ver = self.current_version().await.unwrap_or(i64::MAX);
        self.snapshot(ver).await.unwrap_or_default()
    }

    async fn batch_update(&self, patches: HashMap<String, Option<String>>) -> anyhow::Result<()> {
        if patches.is_empty() {
            return Ok(());
        }
        let ver = self.next_version().await?;
        let entries: Vec<(String, Option<String>)> = patches
            .into_iter()
            .map(|(key, value)| {
                let prefixed = format!("{PREFIX}{key}");
                let json_val = value.map(|v| serde_json::to_string(&v).unwrap());
                (prefixed, json_val)
            })
            .collect();
        self.append_version_log(ver, &entries).await?;
        self.notify();
        Ok(())
    }

    fn subscribe(&self) -> watch::Receiver<()> { self.tx.subscribe() }
}

#[cfg(test)]
mod tests {
    use rara_domain_shared::settings::SettingsProvider;

    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(include_str!(
            "../../../../rara-model/migrations/20260304000000_init.up.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../../rara-model/migrations/20260412000000_settings_version.up.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn set_bumps_version() {
        let pool = test_pool().await;
        let svc = SettingsSvc::load(pool).await.unwrap();

        svc.set("llm.provider", "ollama").await.unwrap();
        assert_eq!(svc.current_version().await.unwrap(), 1);

        svc.set("llm.provider", "openrouter").await.unwrap();
        assert_eq!(svc.current_version().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn get_returns_latest_value() {
        let pool = test_pool().await;
        let svc = SettingsSvc::load(pool).await.unwrap();

        svc.set("key", "v1").await.unwrap();
        svc.set("key", "v2").await.unwrap();
        assert_eq!(svc.get("key").await.unwrap(), "v2");
    }

    #[tokio::test]
    async fn snapshot_at_version() {
        let pool = test_pool().await;
        let svc = SettingsSvc::load(pool).await.unwrap();

        svc.set("a", "1").await.unwrap(); // v1
        svc.set("b", "2").await.unwrap(); // v2
        svc.set("a", "3").await.unwrap(); // v3

        let snap = svc.snapshot(2).await.unwrap();
        assert_eq!(snap.get("a").unwrap(), "1");
        assert_eq!(snap.get("b").unwrap(), "2");

        let snap = svc.snapshot(3).await.unwrap();
        assert_eq!(snap.get("a").unwrap(), "3");
    }

    #[tokio::test]
    async fn delete_creates_tombstone() {
        let pool = test_pool().await;
        let svc = SettingsSvc::load(pool).await.unwrap();

        svc.set("x", "val").await.unwrap(); // v1
        svc.delete("x").await.unwrap(); // v2 tombstone

        assert!(svc.get("x").await.is_none());

        let snap = svc.snapshot(1).await.unwrap();
        assert_eq!(snap.get("x").unwrap(), "val");

        let snap = svc.snapshot(2).await.unwrap();
        assert!(!snap.contains_key("x"));
    }

    #[tokio::test]
    async fn batch_update_single_version() {
        let pool = test_pool().await;
        let svc = SettingsSvc::load(pool).await.unwrap();

        let mut patches = HashMap::new();
        patches.insert("a".to_owned(), Some("1".to_owned()));
        patches.insert("b".to_owned(), Some("2".to_owned()));
        svc.batch_update(patches).await.unwrap();

        assert_eq!(svc.current_version().await.unwrap(), 1);
        assert_eq!(svc.get("a").await.unwrap(), "1");
        assert_eq!(svc.get("b").await.unwrap(), "2");
    }

    #[tokio::test]
    async fn list_returns_current_snapshot() {
        let pool = test_pool().await;
        let svc = SettingsSvc::load(pool).await.unwrap();

        svc.set("a", "1").await.unwrap();
        svc.set("b", "2").await.unwrap();

        let all = svc.list().await;
        assert_eq!(all.get("a").unwrap(), "1");
        assert_eq!(all.get("b").unwrap(), "2");
    }

    #[tokio::test]
    async fn rollback_creates_new_version() {
        let pool = test_pool().await;
        let svc = SettingsSvc::load(pool).await.unwrap();

        svc.set("a", "original").await.unwrap(); // v1
        svc.set("a", "changed").await.unwrap(); // v2

        let new_ver = svc.rollback_to(1).await.unwrap();
        assert_eq!(new_ver, 3); // rollback creates v3
        assert_eq!(svc.get("a").await.unwrap(), "original");
    }
}
