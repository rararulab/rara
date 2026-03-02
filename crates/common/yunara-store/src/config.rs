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

use std::{path::Path, time::Duration};

use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;

use crate::{db::DBStore, err::Result};

/// Database configuration.
///
/// `database_url` and `migration_dir` are **required** — they must be supplied
/// via Consul KV or `RARA__DATABASE__*` environment variables.
/// Connection-pool parameters have sensible defaults.
#[derive(Debug, Clone, bon::Builder, Deserialize)]
#[builder(on(String, into))]
pub struct DatabaseConfig {
    /// PostgreSQL database URL, e.g. `postgres://user:pass@host:5432/dbname`.
    /// **Required** — must come from Consul or env.
    #[builder(getter)]
    pub database_url: String,

    /// SQLx migration directory path.
    /// **Required** — must come from Consul or env.
    #[builder(getter)]
    pub migration_dir: String,

    /// Maximum number of connections in the pool.
    #[serde(default = "default_max_connections")]
    #[builder(default = 20, getter)]
    pub max_connections: u32,

    /// Minimum number of idle connections.
    #[serde(default = "default_min_connections")]
    #[builder(default = 2, getter)]
    pub min_connections: u32,

    /// Connection timeout (default: 30 seconds).
    #[serde(default = "default_connect_timeout", with = "humantime_serde")]
    #[builder(default = Duration::from_secs(30), getter)]
    pub connect_timeout: Duration,

    /// Maximum lifetime of a connection (default: 30 minutes).
    #[serde(default = "default_max_lifetime", with = "humantime_serde::option")]
    #[builder(getter)]
    pub max_lifetime: Option<Duration>,

    /// Idle timeout for connections (default: 10 minutes).
    #[serde(default = "default_idle_timeout", with = "humantime_serde::option")]
    #[builder(getter)]
    pub idle_timeout: Option<Duration>,
}

fn default_max_connections() -> u32 { 20 }
fn default_min_connections() -> u32 { 2 }
fn default_connect_timeout() -> Duration { Duration::from_secs(30) }
fn default_max_lifetime() -> Option<Duration> { Some(Duration::from_secs(1800)) }
fn default_idle_timeout() -> Option<Duration> { Some(Duration::from_secs(600)) }

impl DatabaseConfig {
    #[tracing::instrument(
        level = "trace",
        skip(self),
        fields(database_url = %self.database_url, migration_dir = %self.migration_dir),
        err
    )]
    pub async fn open(&self) -> Result<DBStore> {
        let mut pool_options = PgPoolOptions::new()
            .max_connections(self.max_connections)
            .min_connections(self.min_connections)
            .acquire_timeout(self.connect_timeout);

        if let Some(max_lifetime) = self.max_lifetime {
            pool_options = pool_options.max_lifetime(max_lifetime);
        }

        if let Some(idle_timeout) = self.idle_timeout {
            pool_options = pool_options.idle_timeout(idle_timeout);
        }
        let pool = pool_options.connect(&self.database_url).await?;

        tracing::info!(
            "Initialized DBStore with database_url: {}",
            self.database_url
        );

        let migrator = sqlx::migrate::Migrator::new(Path::new(&self.migration_dir)).await?;
        migrator.run(&pool).await?;

        Ok(DBStore::new(pool))
    }
}
