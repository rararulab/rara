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

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Row, SqlitePool, sqlite::SqliteRow};

/// A single memory item stored in SQLite.
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

impl<'r> FromRow<'r, SqliteRow> for MemoryItem {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id:              row.try_get("id")?,
            username:        row.try_get("username")?,
            content:         row.try_get("content")?,
            memory_type:     row.try_get("memory_type")?,
            category:        row.try_get("category")?,
            source_tape:     row.try_get("source_tape")?,
            source_entry_id: row.try_get("source_entry_id")?,
            created_at:      row.try_get("created_at")?,
            updated_at:      row.try_get("updated_at")?,
        })
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
pub async fn insert_item(pool: &SqlitePool, item: &NewMemoryItem) -> sqlx::Result<i64> {
    let row: (i64,) = sqlx::query_as(
        r#"INSERT INTO memory_items (username, content, memory_type, category, source_tape, source_entry_id, embedding)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
           RETURNING id"#,
    )
    .bind(&item.username)
    .bind(&item.content)
    .bind(&item.memory_type)
    .bind(&item.category)
    .bind(&item.source_tape)
    .bind(&item.source_entry_id)
    .bind(&item.embedding)
    .fetch_one(pool)
    .await?;

    Ok(row.0)
}

/// List all memory items for a given user.
pub async fn list_items_by_username(
    pool: &SqlitePool,
    username: &str,
) -> sqlx::Result<Vec<MemoryItem>> {
    sqlx::query_as::<_, MemoryItem>(
        "SELECT id, username, content, memory_type, category, source_tape, source_entry_id, \
         created_at, updated_at FROM memory_items WHERE username = ?1 ORDER BY created_at DESC",
    )
    .bind(username)
    .fetch_all(pool)
    .await
}

/// Get memory items by a list of ids.
pub async fn get_items_by_ids(pool: &SqlitePool, ids: &[i64]) -> sqlx::Result<Vec<MemoryItem>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // Build a comma-separated placeholder list for the IN clause.
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT id, username, content, memory_type, category, source_tape, source_entry_id, \
         created_at, updated_at FROM memory_items WHERE id IN ({}) ORDER BY created_at DESC",
        placeholders.join(", ")
    );

    let mut query = sqlx::query_as::<_, MemoryItem>(&sql);
    for id in ids {
        query = query.bind(id);
    }
    query.fetch_all(pool).await
}

/// Load all embeddings for a user. Returns (id, embedding_blob) pairs.
///
/// Only returns rows that have a non-null embedding.
pub async fn load_embeddings(
    pool: &SqlitePool,
    username: &str,
) -> sqlx::Result<Vec<(i64, Vec<u8>)>> {
    sqlx::query_as::<_, (i64, Vec<u8>)>(
        "SELECT id, embedding FROM memory_items WHERE username = ?1 AND embedding IS NOT NULL",
    )
    .bind(username)
    .fetch_all(pool)
    .await
}

/// List distinct categories for a user.
pub async fn list_categories(pool: &SqlitePool, username: &str) -> sqlx::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT category FROM memory_items WHERE username = ?1 ORDER BY category",
    )
    .bind(username)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|(c,)| c).collect())
}

/// List items in a specific category for a user.
pub async fn list_items_by_category(
    pool: &SqlitePool,
    username: &str,
    category: &str,
) -> sqlx::Result<Vec<MemoryItem>> {
    sqlx::query_as::<_, MemoryItem>(
        "SELECT id, username, content, memory_type, category, source_tape, source_entry_id, \
         created_at, updated_at FROM memory_items WHERE username = ?1 AND category = ?2 ORDER BY \
         created_at DESC",
    )
    .bind(username)
    .bind(category)
    .fetch_all(pool)
    .await
}
