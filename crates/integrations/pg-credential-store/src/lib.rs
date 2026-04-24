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

//! Diesel-backed [`KeyringStore`] implementation.
//!
//! Despite the historical `pg-` crate prefix, the underlying storage is the
//! workspace's shared SQLite database — the `credential_store` table is
//! defined alongside the rest of the schema in `rara-model/src/schema.rs`.
//! The crate is part of the sqlx → diesel migration tracked in #1702.

use std::fmt::Debug;

use async_trait::async_trait;
use diesel::{
    ExpressionMethods, OptionalExtension, QueryDsl, Queryable, Selectable, SelectableHelper,
    sql_types::Text, upsert::excluded,
};
use diesel_async::RunQueryDsl;
use rara_keyring_store::{KeyringStore, PgSnafu, PoolSnafu, Result};
use rara_model::schema::credential_store;
use snafu::ResultExt;
use yunara_store::diesel_pool::DieselSqlitePool;

/// Row projection for the `credential_store` table.
#[derive(Queryable, Selectable)]
#[diesel(table_name = credential_store)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
struct CredentialRow {
    value: String,
}

#[derive(Clone)]
pub struct PgKeyringStore {
    pool: DieselSqlitePool,
}

impl Debug for PgKeyringStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgKeyringStore").finish()
    }
}

impl PgKeyringStore {
    pub fn new(pool: DieselSqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl KeyringStore for PgKeyringStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>> {
        let mut conn = self.pool.get().await.context(PoolSnafu)?;
        let row: Option<CredentialRow> = credential_store::table
            .filter(credential_store::service.eq(service))
            .filter(credential_store::account.eq(account))
            .select(CredentialRow::as_select())
            .first(&mut *conn)
            .await
            .optional()
            .context(PgSnafu)?;
        Ok(row.map(|r| r.value))
    }

    #[tracing::instrument(skip(self, value), fields(value_len = value.len()), level = "debug")]
    async fn save(&self, service: &str, account: &str, value: &str) -> Result<()> {
        let mut conn = self.pool.get().await.context(PoolSnafu)?;
        // SQLite's `datetime('now')` is emitted via `sql::<Text>` — diesel has
        // no cross-backend DSL helper for the sqlite-specific `datetime()`
        // form. Per docs/guides/db-diesel-migration.md, narrow literal-SQL
        // fragments like this are the only sanctioned `sql!` escape-hatch use.
        let now = diesel::dsl::sql::<Text>("datetime('now')");
        diesel::insert_into(credential_store::table)
            .values((
                credential_store::service.eq(service),
                credential_store::account.eq(account),
                credential_store::value.eq(value),
                credential_store::updated_at.eq(now.clone()),
            ))
            .on_conflict((credential_store::service, credential_store::account))
            .do_update()
            .set((
                credential_store::value.eq(excluded(credential_store::value)),
                credential_store::updated_at.eq(now),
            ))
            .execute(&mut *conn)
            .await
            .context(PgSnafu)?;
        Ok(())
    }

    #[tracing::instrument(skip(self), level = "debug")]
    async fn delete(&self, service: &str, account: &str) -> Result<bool> {
        let mut conn = self.pool.get().await.context(PoolSnafu)?;
        let affected = diesel::delete(
            credential_store::table
                .filter(credential_store::service.eq(service))
                .filter(credential_store::account.eq(account)),
        )
        .execute(&mut *conn)
        .await
        .context(PgSnafu)?;
        Ok(affected > 0)
    }
}
