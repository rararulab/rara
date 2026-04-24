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

use crate::{diesel_pool::DieselSqlitePool, kv::KVStore};

/// Database store that owns the shared diesel-async SQLite pool.
#[derive(Clone)]
pub struct DBStore {
    pool: DieselSqlitePool,
}

impl DBStore {
    /// Wrap an existing diesel-async SQLite pool.
    pub fn new(pool: DieselSqlitePool) -> Self { Self { pool } }

    /// Get a KV store instance.
    pub fn kv_store(&self) -> KVStore { KVStore::new(self.pool.clone()) }

    /// Get the underlying diesel-async SQLite pool.
    pub fn pool(&self) -> &DieselSqlitePool { &self.pool }
}

impl From<DBStore> for DieselSqlitePool {
    fn from(store: DBStore) -> Self { store.pool }
}
