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
//! Introduced as part of the sqlx → diesel migration (#1702). SQLite uses
//! [`SyncConnectionWrapper`] because SQLite itself is single-threaded and has
//! no native async driver. Postgres uses the native [`AsyncPgConnection`].

use diesel::SqliteConnection;
use diesel_async::{
    AsyncConnection, AsyncPgConnection, RunQueryDsl,
    pooled_connection::{AsyncDieselConnectionManager, ManagerConfig},
    sync_connection_wrapper::SyncConnectionWrapper,
};
use futures::FutureExt;
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
/// Each newly-established connection has `PRAGMA journal_mode=WAL`,
/// `PRAGMA busy_timeout=5000`, and `PRAGMA foreign_keys=ON` applied via the
/// manager's `custom_setup` hook so pragmas are set exactly once per
/// physical connection rather than on every checkout.
#[tracing::instrument(level = "trace", skip(config), fields(url = %config.database_url), err)]
pub async fn build_sqlite_pool(config: &DieselPoolConfig) -> Result<DieselSqlitePool> {
    let mut manager_config = ManagerConfig::<DieselSqliteConnection>::default();
    manager_config.custom_setup = Box::new(|url| {
        let url = url.to_owned();
        async move {
            let mut conn = DieselSqliteConnection::establish(&url).await?;
            for pragma in SQLITE_PRAGMAS {
                diesel::sql_query(*pragma)
                    .execute(&mut conn)
                    .await
                    .map_err(diesel::ConnectionError::CouldntSetupConfiguration)?;
            }
            Ok(conn)
        }
        .boxed()
    });
    let manager = AsyncDieselConnectionManager::<DieselSqliteConnection>::new_with_config(
        config.database_url.clone(),
        manager_config,
    );
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

/// SQLite pragmas applied to every newly-established connection. Order
/// matches the sqlx-era setup: WAL journaling, a 5s busy wait, and enforced
/// foreign keys.
const SQLITE_PRAGMAS: &[&str] = &[
    "PRAGMA journal_mode=WAL",
    "PRAGMA busy_timeout=5000",
    "PRAGMA foreign_keys=ON",
];

/// Re-export of the bb8 runtime pool error so consumers don't have to
/// depend on `bb8` directly to name it in signatures.
pub use ::bb8::RunError as DieselPoolRunError;
/// Re-export of the underlying diesel-async pool init error.
pub use diesel_async::pooled_connection::PoolError as DieselPoolInitError;
