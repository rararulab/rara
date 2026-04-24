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

//! Diesel-async + bb8 connection pools.
//!
//! Introduced as part of the sqlx → diesel migration (#1702). The sqlx pool
//! in [`crate::db`] and the diesel pools defined here live side-by-side
//! during the transition. Consumer crates are migrated one at a time; the
//! sqlx pool is removed in the final cutover PR.
//!
//! SQLite uses [`SyncConnectionWrapper`] because SQLite itself is
//! single-threaded and has no native async driver. Postgres uses the native
//! [`AsyncPgConnection`].

use diesel::SqliteConnection;
use diesel_async::{
    AsyncPgConnection, pooled_connection::AsyncDieselConnectionManager,
    sync_connection_wrapper::SyncConnectionWrapper,
};
use snafu::ResultExt;

use crate::error::{BuildDieselPoolSnafu, Result};

/// SQLite diesel-async connection, wrapping a blocking [`SqliteConnection`]
/// in [`SyncConnectionWrapper`].
pub type DieselSqliteConnection = SyncConnectionWrapper<SqliteConnection>;

/// bb8-managed diesel-async pool for SQLite.
pub type DieselSqlitePool = ::bb8::Pool<AsyncDieselConnectionManager<DieselSqliteConnection>>;

/// Postgres diesel-async connection.
pub type DieselPgConnection = AsyncPgConnection;

/// bb8-managed diesel-async pool for Postgres.
pub type DieselPgPool = ::bb8::Pool<AsyncDieselConnectionManager<DieselPgConnection>>;

/// Pool sizing and connection parameters for the diesel-async pools.
///
/// Defaults are deliberately absent — this struct is populated from the YAML
/// config (per the no-hardcoded-defaults invariant) by the caller and handed
/// to [`build_sqlite_pool`] / [`build_pg_pool`].
#[derive(Debug, Clone, bon::Builder, serde::Serialize, serde::Deserialize)]
pub struct DieselPoolConfig {
    /// Database URL (`sqlite://…` or `postgres://…`).
    pub database_url:    String,
    /// Maximum number of pooled connections.
    pub max_connections: u32,
    /// Minimum idle connections to keep warm (`None` means unset).
    #[serde(default)]
    pub min_idle:        Option<u32>,
}

/// Build a bb8 pool of diesel-async SQLite connections.
///
/// Migrations are **not** applied here — migration is still driven by
/// `rara-app::init_infra` via `sqlx::migrate!` until the cutover PR
/// replaces it with `diesel_migrations::embed_migrations!`.
#[tracing::instrument(level = "trace", skip(config), fields(url = %config.database_url), err)]
pub async fn build_sqlite_pool(config: &DieselPoolConfig) -> Result<DieselSqlitePool> {
    let manager =
        AsyncDieselConnectionManager::<DieselSqliteConnection>::new(config.database_url.clone());
    let mut builder = ::bb8::Pool::builder().max_size(config.max_connections);
    if let Some(min_idle) = config.min_idle {
        builder = builder.min_idle(Some(min_idle));
    }
    builder.build(manager).await.context(BuildDieselPoolSnafu)
}

/// Build a bb8 pool of diesel-async Postgres connections.
#[tracing::instrument(level = "trace", skip(config), fields(url = %config.database_url), err)]
pub async fn build_pg_pool(config: &DieselPoolConfig) -> Result<DieselPgPool> {
    let manager =
        AsyncDieselConnectionManager::<DieselPgConnection>::new(config.database_url.clone());
    let mut builder = ::bb8::Pool::builder().max_size(config.max_connections);
    if let Some(min_idle) = config.min_idle {
        builder = builder.min_idle(Some(min_idle));
    }
    builder.build(manager).await.context(BuildDieselPoolSnafu)
}

/// Re-export of the bb8 runtime pool error so consumers don't have to
/// depend on `bb8` directly to name it in signatures.
pub use ::bb8::RunError as DieselPoolRunError;
/// Re-export of the underlying diesel-async pool init error.
pub use diesel_async::pooled_connection::PoolError as DieselPoolInitError;
