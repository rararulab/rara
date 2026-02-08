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

use sqlx::{
    Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::{config::DatabaseConfig, err::*, kv::KVStore};

/// Database store that manages the SQLite connection pool
#[derive(Clone)]
pub struct DBStore {
    pool: SqlitePool,
}

impl DBStore {
    /// Create a new database store with the given configuration
    ///
    /// # Arguments
    /// * `config` - Database configuration
    #[tracing::instrument(level = "trace", skip(config), fields(db_path = ?config.db_path), err)]
    pub async fn new(config: DatabaseConfig) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(&config.db_path)
            .create_if_missing(true);

        let mut pool_options = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.connect_timeout);

        if let Some(max_lifetime) = config.max_lifetime {
            pool_options = pool_options.max_lifetime(max_lifetime);
        }

        if let Some(idle_timeout) = config.idle_timeout {
            pool_options = pool_options.idle_timeout(idle_timeout);
        }
        let pool = pool_options.connect_with(options).await?;

        tracing::info!(
            "Initialized DBStore with path: {}",
            config.db_path.display()
        );

        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    /// Get a KV store instance
    pub fn kv_store(&self) -> KVStore { KVStore::new(self.pool.clone()) }

    /// Get the underlying SQLite pool
    pub fn pool(&self) -> &SqlitePool { &self.pool }

    /// Acquire a connection from the pool
    pub async fn acquire(&self) -> Result<sqlx::pool::PoolConnection<Sqlite>> {
        Ok(self.pool.acquire().await?)
    }

    /// Creates a DBStore backed by a lazily-connected in-memory pool.
    ///
    /// Intended for testing scenarios where the database will not actually
    /// be queried. No migrations are run and no connection is established
    /// until a query is attempted.
    #[doc(hidden)]
    pub fn new_lazy_in_memory() -> Self {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_lazy_with(options);

        Self { pool }
    }
}
