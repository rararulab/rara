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
