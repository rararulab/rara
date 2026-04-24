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
    db::DBStore,
    diesel_pool::{DieselPoolConfig, build_sqlite_pool},
    error::Result,
};

/// Database configuration for SQLite.
///
/// The database URL is determined by the caller (typically from
/// `rara_paths::database_dir()`). Migrations are embedded at compile time by
/// the consuming binary via `diesel_migrations::embed_migrations!`.
#[derive(Debug, Clone, bon::Builder, serde::Serialize, serde::Deserialize)]
pub struct DatabaseConfig {
    /// Maximum number of connections in the pool.
    #[serde(default = "default_max_connections")]
    #[builder(default = 5, getter)]
    pub max_connections: u32,
}

fn default_max_connections() -> u32 { 5 }

impl DatabaseConfig {
    /// Open a SQLite database at the given URL.
    ///
    /// Builds a diesel-async bb8 pool and applies the `WAL` / `busy_timeout`
    /// / `foreign_keys` pragmas once per physical connection via the pool's
    /// `custom_setup` hook. The caller is responsible for running
    /// migrations afterwards.
    #[tracing::instrument(
        level = "trace",
        skip(self),
        fields(%database_url),
        err
    )]
    pub async fn open(&self, database_url: &str) -> Result<DBStore> {
        let pool = build_sqlite_pool(
            &DieselPoolConfig::builder()
                .database_url(database_url.to_owned())
                .max_connections(self.max_connections)
                .build(),
        )
        .await?;

        tracing::info!("SQLite database initialized: {database_url}");

        Ok(DBStore::new(pool))
    }
}
