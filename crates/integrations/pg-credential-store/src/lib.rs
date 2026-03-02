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

use std::fmt::Debug;

use async_trait::async_trait;
use rara_keyring_store::{KeyringStore, Result};
use sqlx::PgPool;

#[derive(Clone)]
pub struct PgKeyringStore {
    pool: PgPool,
}

impl Debug for PgKeyringStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgKeyringStore").finish()
    }
}

impl PgKeyringStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl KeyringStore for PgKeyringStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT value FROM credential_store WHERE service = $1 AND account = $2",
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
            "INSERT INTO credential_store (service, account, value, updated_at) \
             VALUES ($1, $2, $3, now()) \
             ON CONFLICT (service, account) DO UPDATE SET value = $3, updated_at = now()",
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
        let result = sqlx::query(
            "DELETE FROM credential_store WHERE service = $1 AND account = $2",
        )
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
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    async fn setup() -> (PgPool, impl std::any::Any) {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let pool = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("../../rara-model/migrations")
            .run(&pool)
            .await
            .unwrap();
        (pool, container)
    }

    #[tokio::test]
    async fn load_missing_returns_none() {
        let (pool, _c) = setup().await;
        let store = PgKeyringStore::new(pool);
        assert_eq!(store.load("svc", "acc").await.unwrap(), None);
    }

    #[tokio::test]
    async fn save_then_load() {
        let (pool, _c) = setup().await;
        let store = PgKeyringStore::new(pool);
        store.save("svc", "acc", "secret").await.unwrap();
        assert_eq!(
            store.load("svc", "acc").await.unwrap(),
            Some("secret".to_owned())
        );
    }

    #[tokio::test]
    async fn save_overwrites() {
        let (pool, _c) = setup().await;
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
        let (pool, _c) = setup().await;
        let store = PgKeyringStore::new(pool);
        store.save("svc", "acc", "val").await.unwrap();
        assert!(store.delete("svc", "acc").await.unwrap());
        assert_eq!(store.load("svc", "acc").await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_missing_returns_false() {
        let (pool, _c) = setup().await;
        let store = PgKeyringStore::new(pool);
        assert!(!store.delete("svc", "acc").await.unwrap());
    }
}
