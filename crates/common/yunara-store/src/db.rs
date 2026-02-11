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

use sqlx::{PgPool, Postgres, postgres::PgPoolOptions};

use crate::{err::*, kv::KVStore};

/// Database store that manages the PostgreSQL connection pool
#[derive(Clone)]
pub struct DBStore {
    pool: PgPool,
}

impl DBStore {
    pub(crate) fn new(pool: PgPool) -> Self { Self { pool } }

    /// Get a KV store instance
    pub fn kv_store(&self) -> KVStore { KVStore::new(self.pool.clone()) }

    /// Get the underlying PostgreSQL pool
    pub fn pool(&self) -> &PgPool { &self.pool }

    /// Acquire a connection from the pool
    pub async fn acquire(&self) -> Result<sqlx::pool::PoolConnection<Postgres>> {
        Ok(self.pool.acquire().await?)
    }

    /// Creates a DBStore backed by a lazily-connected pool.
    ///
    /// Intended for tests where the DB might not be queried.
    #[doc(hidden)]
    pub fn new_lazy(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy(database_url)?;
        Ok(Self { pool })
    }
}

impl Into<PgPool> for DBStore {
    fn into(self) -> PgPool { self.pool }
}
