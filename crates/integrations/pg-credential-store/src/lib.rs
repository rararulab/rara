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

use std::fmt::Debug;

use async_trait::async_trait;
use rara_keyring_store::{KeyringStore, Result};
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct PgKeyringStore {
    pool: SqlitePool,
}

impl Debug for PgKeyringStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgKeyringStore").finish()
    }
}

impl PgKeyringStore {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl KeyringStore for PgKeyringStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT value FROM credential_store WHERE service = ?1 AND account = ?2",
        )
        .bind(service)
        .bind(account)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| rara_keyring_store::Error::Pg {
            source:   e,
            location: snafu::Location::default(),
        })?;
        Ok(row.map(|(v,)| v))
    }

    #[tracing::instrument(skip(self, value), fields(value_len = value.len()), level = "debug")]
    async fn save(&self, service: &str, account: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO credential_store (service, account, value, updated_at) VALUES (?1, ?2, \
             ?3, datetime('now')) ON CONFLICT (service, account) DO UPDATE SET value = ?3, updated_at = \
             datetime('now')",
        )
        .bind(service)
        .bind(account)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(|e| rara_keyring_store::Error::Pg {
            source:   e,
            location: snafu::Location::default(),
        })?;
        Ok(())
    }

    #[tracing::instrument(skip(self), level = "debug")]
    async fn delete(&self, service: &str, account: &str) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM credential_store WHERE service = ?1 AND account = ?2")
                .bind(service)
                .bind(account)
                .execute(&self.pool)
                .await
                .map_err(|e| rara_keyring_store::Error::Pg {
                    source:   e,
                    location: snafu::Location::default(),
                })?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("../../rara-model/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn load_missing_returns_none() {
        let pool = setup().await;
        let store = PgKeyringStore::new(pool);
        assert_eq!(store.load("svc", "acc").await.unwrap(), None);
    }

    #[tokio::test]
    async fn save_then_load() {
        let pool = setup().await;
        let store = PgKeyringStore::new(pool);
        store.save("svc", "acc", "secret").await.unwrap();
        assert_eq!(
            store.load("svc", "acc").await.unwrap(),
            Some("secret".to_owned())
        );
    }

    #[tokio::test]
    async fn save_overwrites() {
        let pool = setup().await;
        let store = PgKeyringStore::new(pool);
        store.save("svc", "acc", "v1").await.unwrap();
        store.save("svc", "acc", "v2").await.unwrap();
        assert_eq!(
            store.load("svc", "acc").await.unwrap(),
            Some("v2".to_owned())
        );
    }

    #[tokio::test]
    async fn delete_existing_returns_true() {
        let pool = setup().await;
        let store = PgKeyringStore::new(pool);
        store.save("svc", "acc", "val").await.unwrap();
        assert!(store.delete("svc", "acc").await.unwrap());
        assert_eq!(store.load("svc", "acc").await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_missing_returns_false() {
        let pool = setup().await;
        let store = PgKeyringStore::new(pool);
        assert!(!store.delete("svc", "acc").await.unwrap());
    }
}
