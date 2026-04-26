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

use crate::{
    diesel_pool::{DieselSqlitePool, DieselSqlitePools},
    kv::KVStore,
};

/// Database store that owns the shared diesel-async SQLite pools.
#[derive(Clone)]
pub struct DBStore {
    pools: DieselSqlitePools,
}

impl DBStore {
    /// Wrap an existing reader/writer pool bundle.
    pub fn new(pools: DieselSqlitePools) -> Self { Self { pools } }

    /// Get a KV store instance.
    pub fn kv_store(&self) -> KVStore { KVStore::new(self.pools.clone()) }

    /// Get the underlying reader/writer pool bundle.
    pub fn pools(&self) -> &DieselSqlitePools { &self.pools }

    /// Reader pool — concurrent SELECTs.
    pub fn reader(&self) -> &DieselSqlitePool { &self.pools.reader }

    /// Writer pool — single-writer mutations.
    pub fn writer(&self) -> &DieselSqlitePool { &self.pools.writer }
}

impl From<DBStore> for DieselSqlitePools {
    fn from(store: DBStore) -> Self { store.pools }
}
