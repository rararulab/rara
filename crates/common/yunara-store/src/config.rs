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

use sqlx::sqlite::SqlitePoolOptions;

use crate::{db::DBStore, err::Result};

/// Database configuration for SQLite.
///
/// The database URL is determined by the caller (typically from
/// `rara_paths::database_dir()`).  Migrations are embedded at compile time.
#[derive(Debug, Clone, bon::Builder, serde::Deserialize)]
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
    /// Sets WAL mode, busy timeout and foreign key pragmas.
    /// The caller is responsible for running migrations afterwards.
    #[tracing::instrument(
        level = "trace",
        skip(self),
        fields(%database_url),
        err
    )]
    pub async fn open(&self, database_url: &str) -> Result<DBStore> {
        let pool = SqlitePoolOptions::new()
            .max_connections(self.max_connections)
            .connect(database_url)
            .await?;

        // Set recommended SQLite pragmas for WAL mode and concurrency.
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA foreign_keys=ON").execute(&pool).await?;

        tracing::info!("SQLite database initialized: {database_url}");

        Ok(DBStore::new(pool))
    }
}
