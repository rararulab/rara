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

//! Memory items CRUD — SQLite persistence for the knowledge layer.
//!
//! Each memory item stores a single fact/preference/habit extracted from
//! conversation, along with an optional embedding blob for vector search.

use diesel::{ExpressionMethods, QueryDsl, Queryable, Selectable, SelectableHelper};
use diesel_async::RunQueryDsl;
use rara_model::schema::memory_items;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use yunara_store::diesel_pool::DieselSqlitePool;

use crate::error::{DieselPoolSnafu, DieselSnafu, Result};

/// A single memory item stored in SQLite.
///
/// The `id` column is `INTEGER PRIMARY KEY AUTOINCREMENT` so it's effectively
/// NOT NULL for persisted rows. Diesel's schema introspection exposes it as
/// `Nullable<Integer>` (matching SQLite's own rules for autoincrement), so we
/// load through a private `MemoryItemRow` and coerce to `i64` at the boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id:              i64,
    pub username:        String,
    pub content:         String,
    pub memory_type:     String,
    pub category:        String,
    pub source_tape:     Option<String>,
    pub source_entry_id: Option<i64>,
    pub created_at:      String,
    pub updated_at:      String,
    /// Per-item trust score in `[0.0, 1.0]`. Newly inserted rows default
    /// to `1.0` via the SQL column default. The update logic that moves
    /// this value lands in issue #2112.
    pub confidence:      f32,
}

/// Diesel row projection for `memory_items`. Kept private so the public
/// [`MemoryItem`] API stays `id: i64` after the coercion. The underlying
/// SQLite column is `INTEGER PRIMARY KEY AUTOINCREMENT`, which diesel
/// introspects as `Nullable<Integer>` (i32); we widen to `i64` at the
/// boundary because memory item ids fit in i32 in practice and the public
/// type predates the migration.
#[derive(Queryable, Selectable)]
#[diesel(table_name = memory_items)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
struct MemoryItemRow {
    id:              Option<i32>,
    username:        String,
    content:         String,
    memory_type:     String,
    category:        String,
    source_tape:     Option<String>,
    source_entry_id: Option<i32>,
    created_at:      String,
    updated_at:      String,
    confidence:      f32,
}

impl From<MemoryItemRow> for MemoryItem {
    fn from(r: MemoryItemRow) -> Self {
        Self {
            id:              r.id.map(i64::from).unwrap_or(0),
            username:        r.username,
            content:         r.content,
            memory_type:     r.memory_type,
            category:        r.category,
            source_tape:     r.source_tape,
            source_entry_id: r.source_entry_id.map(i64::from),
            created_at:      r.created_at,
            updated_at:      r.updated_at,
            confidence:      r.confidence,
        }
    }
}

/// Data needed to insert a new memory item (no id, no timestamps).
#[derive(Debug, Clone)]
pub struct NewMemoryItem {
    pub username:        String,
    pub content:         String,
    pub memory_type:     String,
    pub category:        String,
    pub source_tape:     Option<String>,
    pub source_entry_id: Option<i64>,
    pub embedding:       Option<Vec<u8>>,
}

/// Insert a new memory item. Returns the assigned row id.
pub async fn insert_item(pool: &DieselSqlitePool, item: &NewMemoryItem) -> Result<i64> {
    let mut conn = pool.get().await.context(DieselPoolSnafu)?;
    let source_entry_id: Option<i32> = item.source_entry_id.map(|v| v as i32);
    let id: Option<i32> = diesel::insert_into(memory_items::table)
        .values((
            memory_items::username.eq(&item.username),
            memory_items::content.eq(&item.content),
            memory_items::memory_type.eq(&item.memory_type),
            memory_items::category.eq(&item.category),
            memory_items::source_tape.eq(&item.source_tape),
            memory_items::source_entry_id.eq(source_entry_id),
            memory_items::embedding.eq(&item.embedding),
        ))
        .returning(memory_items::id)
        .get_result(&mut *conn)
        .await
        .context(DieselSnafu)?;
    Ok(id.map(i64::from).unwrap_or(0))
}

/// List all memory items for a given user.
pub async fn list_items_by_username(
    pool: &DieselSqlitePool,
    username: &str,
) -> Result<Vec<MemoryItem>> {
    let mut conn = pool.get().await.context(DieselPoolSnafu)?;
    let rows: Vec<MemoryItemRow> = memory_items::table
        .filter(memory_items::username.eq(username))
        .order(memory_items::created_at.desc())
        .select(MemoryItemRow::as_select())
        .load(&mut *conn)
        .await
        .context(DieselSnafu)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Get memory items by a list of ids.
pub async fn get_items_by_ids(pool: &DieselSqlitePool, ids: &[i64]) -> Result<Vec<MemoryItem>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let narrowed: Vec<i32> = ids.iter().map(|&v| v as i32).collect();
    let mut conn = pool.get().await.context(DieselPoolSnafu)?;
    let rows: Vec<MemoryItemRow> = memory_items::table
        .filter(memory_items::id.eq_any(narrowed))
        .order(memory_items::created_at.desc())
        .select(MemoryItemRow::as_select())
        .load(&mut *conn)
        .await
        .context(DieselSnafu)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Load all embeddings for a user. Returns (id, embedding_blob) pairs.
///
/// Only returns rows that have a non-null embedding.
pub async fn load_embeddings(
    pool: &DieselSqlitePool,
    username: &str,
) -> Result<Vec<(i64, Vec<u8>)>> {
    let mut conn = pool.get().await.context(DieselPoolSnafu)?;
    let rows: Vec<(Option<i32>, Option<Vec<u8>>)> = memory_items::table
        .filter(memory_items::username.eq(username))
        .filter(memory_items::embedding.is_not_null())
        .select((memory_items::id, memory_items::embedding))
        .load(&mut *conn)
        .await
        .context(DieselSnafu)?;

    Ok(rows
        .into_iter()
        .filter_map(|(id, emb)| emb.map(|e| (id.map(i64::from).unwrap_or(0), e)))
        .collect())
}

/// List distinct categories for a user.
pub async fn list_categories(pool: &DieselSqlitePool, username: &str) -> Result<Vec<String>> {
    let mut conn = pool.get().await.context(DieselPoolSnafu)?;
    let cats: Vec<String> = memory_items::table
        .filter(memory_items::username.eq(username))
        .select(memory_items::category)
        .distinct()
        .order(memory_items::category.asc())
        .load(&mut *conn)
        .await
        .context(DieselSnafu)?;
    Ok(cats)
}

/// List items in a specific category for a user.
pub async fn list_items_by_category(
    pool: &DieselSqlitePool,
    username: &str,
    category: &str,
) -> Result<Vec<MemoryItem>> {
    let mut conn = pool.get().await.context(DieselPoolSnafu)?;
    let rows: Vec<MemoryItemRow> = memory_items::table
        .filter(memory_items::username.eq(username))
        .filter(memory_items::category.eq(category))
        .order(memory_items::created_at.desc())
        .select(MemoryItemRow::as_select())
        .load(&mut *conn)
        .await
        .context(DieselSnafu)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[cfg(test)]
mod tests {
    use diesel_async::RunQueryDsl as _;
    use yunara_store::diesel_pool::DieselSqlitePool;

    use super::*;

    /// SQL bundled into the `2026-05-21-000000_memory_confidence_outcomes`
    /// migration. Tests apply this on top of the kernel's shared test
    /// schema (which still mirrors the pre-migration `memory_items`
    /// shape) to exercise the migration's effect — including the
    /// backfill behavior for rows that existed before the column was
    /// added.
    const ADD_CONFIDENCE_SQL: &str =
        "ALTER TABLE memory_items ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0";
    const CREATE_OUTCOMES_SQL: &str =
        "CREATE TABLE memory_outcomes (id INTEGER PRIMARY KEY AUTOINCREMENT, item_id INTEGER NOT \
         NULL REFERENCES memory_items(id) ON DELETE CASCADE, outcome_kind TEXT NOT NULL, \
         tape_entry_id INTEGER, created_at TEXT NOT NULL DEFAULT (datetime('now')))";
    const CREATE_OUTCOMES_INDEX_SQL: &str =
        "CREATE INDEX idx_memory_outcomes_item_id ON memory_outcomes(item_id)";

    /// Build a pool, apply the migration DDL, return the writer pool.
    async fn pool_with_migration() -> DieselSqlitePool {
        let pools = crate::testing::build_memory_diesel_pools().await;
        run_migration(&pools.writer).await;
        pools.writer
    }

    /// Apply the new migration's DDL against an existing pool.
    async fn run_migration(pool: &DieselSqlitePool) {
        let mut conn = pool.get().await.expect("pool conn");
        for ddl in [
            ADD_CONFIDENCE_SQL,
            CREATE_OUTCOMES_SQL,
            CREATE_OUTCOMES_INDEX_SQL,
        ] {
            diesel::sql_query(ddl)
                .execute(&mut *conn)
                .await
                .expect("apply migration ddl");
        }
    }

    fn sample_new_item(username: &str) -> NewMemoryItem {
        NewMemoryItem {
            username:        username.into(),
            content:         "I prefer dark mode".into(),
            memory_type:     "preference".into(),
            category:        "ui".into(),
            source_tape:     None,
            source_entry_id: None,
            embedding:       None,
        }
    }

    #[tokio::test]
    async fn insert_item_defaults_confidence_to_one() {
        let pool = pool_with_migration().await;
        let id = insert_item(&pool, &sample_new_item("alice"))
            .await
            .expect("insert ok");
        let items = get_items_by_ids(&pool, &[id]).await.expect("load ok");
        assert_eq!(items.len(), 1, "exactly one row");
        assert_eq!(
            items[0].confidence, 1.0,
            "freshly inserted item must default to confidence 1.0"
        );
    }

    #[tokio::test]
    async fn backfill_existing_rows_to_one() {
        // Build a pool with the *pre-migration* schema, insert a row
        // through that schema, then run the migration. The pre-existing
        // row must end up with confidence = 1.0 via the SQL DEFAULT.
        let pools = crate::testing::build_memory_diesel_pools().await;
        let pool = pools.writer.clone();

        let id: Option<i32> = {
            let mut conn = pool.get().await.expect("pool conn");
            diesel::insert_into(memory_items::table)
                .values((
                    memory_items::username.eq("alice"),
                    memory_items::content.eq("old fact"),
                    memory_items::memory_type.eq("preference"),
                    memory_items::category.eq("ui"),
                ))
                .returning(memory_items::id)
                .get_result(&mut *conn)
                .await
                .expect("insert pre-migration row")
        };
        let id = id.map(i64::from).unwrap_or(0);

        run_migration(&pool).await;

        let items = get_items_by_ids(&pool, &[id]).await.expect("load ok");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].confidence, 1.0,
            "pre-existing row must backfill to confidence 1.0"
        );
    }

    #[tokio::test]
    async fn memory_outcomes_table_accepts_rows() {
        let pool = pool_with_migration().await;
        let id = insert_item(&pool, &sample_new_item("alice"))
            .await
            .expect("insert ok");

        let mut conn = pool.get().await.expect("pool conn");
        diesel::sql_query(
            "INSERT INTO memory_outcomes (item_id, outcome_kind) VALUES (?, 'contradicted')",
        )
        .bind::<diesel::sql_types::Integer, _>(id as i32)
        .execute(&mut *conn)
        .await
        .expect("insert outcome");

        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type = diesel::sql_types::Integer)]
            n: i32,
        }
        let rows: Vec<Count> = diesel::sql_query("SELECT COUNT(*) AS n FROM memory_outcomes")
            .load(&mut *conn)
            .await
            .expect("count outcomes");
        assert_eq!(rows[0].n, 1, "outcome row must be persisted");
    }

    #[tokio::test]
    async fn memory_outcomes_cascades_on_item_delete() {
        let pool = pool_with_migration().await;
        let id = insert_item(&pool, &sample_new_item("alice"))
            .await
            .expect("insert ok");

        let mut conn = pool.get().await.expect("pool conn");
        // SQLite enforces foreign keys per-connection only when this
        // pragma is on; the test pool does not set it globally, so we
        // opt in explicitly here.
        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(&mut *conn)
            .await
            .expect("enable fk");
        diesel::sql_query(
            "INSERT INTO memory_outcomes (item_id, outcome_kind) VALUES (?, 'contradicted')",
        )
        .bind::<diesel::sql_types::Integer, _>(id as i32)
        .execute(&mut *conn)
        .await
        .expect("insert outcome");

        diesel::sql_query("DELETE FROM memory_items WHERE id = ?")
            .bind::<diesel::sql_types::Integer, _>(id as i32)
            .execute(&mut *conn)
            .await
            .expect("delete parent");

        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type = diesel::sql_types::Integer)]
            n: i32,
        }
        let rows: Vec<Count> = diesel::sql_query("SELECT COUNT(*) AS n FROM memory_outcomes")
            .load(&mut *conn)
            .await
            .expect("count outcomes");
        assert_eq!(
            rows[0].n, 0,
            "outcome rows must cascade-delete with the parent memory_items row"
        );
    }
}
