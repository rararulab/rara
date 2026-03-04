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

use std::path::Path;

use serde::Deserialize;
use sqlx::sqlite::SqlitePoolOptions;

use crate::{db::DBStore, err::Result};

/// Database configuration for SQLite.
///
/// `database_url` and `migration_dir` are **required** — they must be supplied
/// via config file or environment variables.
#[derive(Debug, Clone, bon::Builder, Deserialize)]
#[builder(on(String, into))]
pub struct DatabaseConfig {
    /// SQLite database URL, e.g. `sqlite:path/to/rara.db?mode=rwc`.
    /// **Required** — must come from config or env.
    #[builder(getter)]
    pub database_url: String,

    /// SQLx migration directory path.
    /// **Required** — must come from config or env.
    #[builder(getter)]
    pub migration_dir: String,

    /// Maximum number of connections in the pool.
    #[serde(default = "default_max_connections")]
    #[builder(default = 5, getter)]
    pub max_connections: u32,
}

fn default_max_connections() -> u32 { 5 }

impl DatabaseConfig {
    #[tracing::instrument(
        level = "trace",
        skip(self),
        fields(database_url = %self.database_url, migration_dir = %self.migration_dir),
        err
    )]
    pub async fn open(&self) -> Result<DBStore> {
        let pool = SqlitePoolOptions::new()
            .max_connections(self.max_connections)
            .connect(&self.database_url)
            .await?;

        // Set recommended SQLite pragmas for WAL mode and concurrency.
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA foreign_keys=ON")
            .execute(&pool)
            .await?;

        tracing::info!(
            "Initialized DBStore with database_url: {}",
            self.database_url
        );

        let migrator = sqlx::migrate::Migrator::new(Path::new(&self.migration_dir)).await?;
        migrator.run(&pool).await?;

        Ok(DBStore::new(pool))
    }
}
