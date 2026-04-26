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
//! no native async driver.
//!
//! ## Reader / writer split (#1843)
//!
//! SQLite serialises writers at the file level — even with WAL, a second
//! writer racing the first gets `SQLITE_BUSY`. To make that contention
//! explicit and bounded, we run two pools:
//!
//! - **reader**: `max_connections` connections, used for `SELECT`-only paths.
//! - **writer**: a single connection (`max_size = 1`), used for everything that
//!   can mutate the database (insert/update/delete, `transaction`, FTS updates,
//!   migrations).
//!
//! Holding writers to a single bb8 slot pushes the queueing into the
//! application layer where it shows up as latency rather than as opaque
//! `database is locked` errors deep in driver code.

use std::time::Duration;

use diesel::SqliteConnection;
use diesel_async::{
    AsyncConnection, RunQueryDsl,
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

/// Reader + writer pool bundle (see module docs for the rationale).
#[derive(Clone)]
pub struct DieselSqlitePools {
    /// Concurrent SELECT-only pool, sized by
    /// `DieselPoolConfig::max_connections`.
    pub reader: DieselSqlitePool,
    /// Single-writer pool — only one mutation runs at a time.
    pub writer: DieselSqlitePool,
}

impl std::fmt::Debug for DieselSqlitePools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DieselSqlitePools").finish_non_exhaustive()
    }
}

/// Pool sizing and connection parameters for the diesel-async pools.
///
/// Defaults are deliberately absent — this struct is populated from the YAML
/// config (per the no-hardcoded-defaults invariant) by the caller and handed
/// to [`build_sqlite_pools`].
#[derive(Debug, Clone, bon::Builder, serde::Serialize, serde::Deserialize)]
pub struct DieselPoolConfig {
    /// Database URL (`sqlite://…`).
    pub database_url:    String,
    /// Maximum number of pooled reader connections.
    pub max_connections: u32,
    /// Minimum idle connections to keep warm in the reader pool (`None` means
    /// unset).
    #[serde(default)]
    pub min_idle:        Option<u32>,
}

/// Build a paired reader + writer pool of diesel-async SQLite connections.
///
/// Each newly-established connection has `PRAGMA journal_mode=WAL`,
/// `PRAGMA busy_timeout=5000`, and `PRAGMA foreign_keys=ON` applied via the
/// manager's `custom_setup` hook so pragmas are set exactly once per
/// physical connection rather than on every checkout. A
/// [`bb8::CustomizeConnection`] hook then runs a best-effort `ROLLBACK` on
/// every checkout so a connection leaked with an open transaction (from an
/// upstream bug) is scrubbed before the next user gets it.
///
/// The writer pool is hard-pinned to `max_size = 1` regardless of config —
/// SQLite serialises writers at the file level, so additional writer
/// connections only translate into `SQLITE_BUSY` retries.
#[tracing::instrument(level = "trace", skip(config), fields(url = %config.database_url), err)]
pub async fn build_sqlite_pools(config: &DieselPoolConfig) -> Result<DieselSqlitePools> {
    let reader = build_pool(config, config.max_connections, config.min_idle).await?;
    let writer = build_pool(config, 1, Some(1)).await?;
    Ok(DieselSqlitePools { reader, writer })
}

async fn build_pool(
    config: &DieselPoolConfig,
    max_size: u32,
    min_idle: Option<u32>,
) -> Result<DieselSqlitePool> {
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
    let mut builder = ::bb8::Pool::builder()
        .max_size(max_size)
        .connection_customizer(Box::new(SqliteConnectionCustomizer));
    if let Some(min_idle) = min_idle {
        builder = builder.min_idle(Some(min_idle));
    }
    // Cap how long a writer caller will block waiting for the single slot.
    // The default is 30s, which is long enough that callers (e.g. trace
    // save) that ought to retry fail closed instead.
    builder = builder.connection_timeout(Duration::from_secs(30));
    builder.build(manager).await.context(BuildDieselPoolSnafu)
}

/// Per-checkout connection scrubber.
///
/// `on_acquire` runs every time a connection is checked out. We issue a
/// best-effort `ROLLBACK` (idempotent — if no transaction is open SQLite
/// returns an error which we ignore) so that a connection leaked mid-tx
/// does not poison the next user with `cannot start a transaction within a
/// transaction`.
#[derive(Debug)]
struct SqliteConnectionCustomizer;

impl bb8::CustomizeConnection<DieselSqliteConnection, diesel_async::pooled_connection::PoolError>
    for SqliteConnectionCustomizer
{
    fn on_acquire<'a>(
        &'a self,
        conn: &'a mut DieselSqliteConnection,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<(), diesel_async::pooled_connection::PoolError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            // Best-effort: if no tx is open this errors and we discard it.
            let _ = diesel::sql_query("ROLLBACK").execute(conn).await;
            Ok(())
        })
    }
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
