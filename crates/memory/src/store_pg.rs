// Copyright 2025 Crrow
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

//! PostgreSQL storage for memory index.
//!
//! Schema is managed via migrations in `crates/rara-model/migrations/`.
//! This module only handles CRUD operations against the existing tables:
//! - `memory_files` — file-level metadata.
//! - `memory_chunks` — chunk content with full-text index.

use sqlx::{PgPool, Row};

use crate::{
    manager::{ChunkDetail, MemoryResult},
    store::{ChunkInput, IndexedFileMeta, MemorySearchRow},
};

#[derive(Debug, Clone)]
pub struct PgMemoryStore {
    pool: PgPool,
}

impl PgMemoryStore {
    pub fn new(pool: PgPool) -> Self { Self { pool } }

    pub async fn list_files(&self) -> MemoryResult<Vec<IndexedFileMeta>> {
        let rows = sqlx::query("SELECT path, hash, mtime, size FROM memory_files")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .into_iter()
            .map(|row| IndexedFileMeta {
                path:  row.get::<String, _>("path"),
                hash:  row.get::<String, _>("hash"),
                mtime: row.get::<i64, _>("mtime"),
                size:  row.get::<i64, _>("size"),
            })
            .collect())
    }

    pub async fn upsert_file_chunks(
        &self,
        path: &str,
        hash: &str,
        mtime: i64,
        size: i64,
        chunks: &[ChunkInput],
    ) -> MemoryResult<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM memory_files WHERE path = $1")
            .bind(path)
            .execute(&mut *tx)
            .await?;

        let row = sqlx::query(
            "INSERT INTO memory_files(path, hash, mtime, size) VALUES($1, $2, $3, $4) RETURNING id",
        )
        .bind(path)
        .bind(hash)
        .bind(mtime)
        .bind(size)
        .fetch_one(&mut *tx)
        .await?;
        let file_id: i64 = row.get("id");

        for chunk in chunks {
            sqlx::query(
                "INSERT INTO memory_chunks(file_id, chunk_index, content) VALUES($1, $2, $3)",
            )
            .bind(file_id)
            .bind(chunk.chunk_index)
            .bind(&chunk.content)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_files(&self, paths: &[String]) -> MemoryResult<()> {
        if paths.is_empty() {
            return Ok(());
        }

        sqlx::query("DELETE FROM memory_files WHERE path = ANY($1)")
            .bind(paths)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn keyword_search(
        &self,
        query: &str,
        limit: usize,
    ) -> MemoryResult<Vec<MemorySearchRow>> {
        let rows = sqlx::query(
            r#"
            SELECT c.id, f.path, c.chunk_index, c.content,
                   ts_rank_cd(to_tsvector('simple', c.content), plainto_tsquery('simple', $1)) AS score
            FROM memory_chunks c
            JOIN memory_files f ON f.id = c.file_id
            WHERE to_tsvector('simple', c.content) @@ plainto_tsquery('simple', $1)
            ORDER BY score DESC
            LIMIT $2
            "#,
        )
        .bind(query)
        .bind(i64::try_from(limit).unwrap_or(20))
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| MemorySearchRow {
                chunk_id:    row.get::<i64, _>("id"),
                path:        row.get::<String, _>("path"),
                chunk_index: row.get::<i64, _>("chunk_index"),
                content:     row.get::<String, _>("content"),
                score:       row.get::<f64, _>("score"),
            })
            .collect())
    }

    pub async fn get_chunk(&self, chunk_id: i64) -> MemoryResult<Option<ChunkDetail>> {
        let row = sqlx::query(
            r#"
            SELECT c.id, f.path, c.chunk_index, c.content
            FROM memory_chunks c
            JOIN memory_files f ON f.id = c.file_id
            WHERE c.id = $1
            "#,
        )
        .bind(chunk_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| ChunkDetail {
            chunk_id:    row.get::<i64, _>("id"),
            path:        row.get::<String, _>("path"),
            chunk_index: row.get::<i64, _>("chunk_index"),
            content:     row.get::<String, _>("content"),
        }))
    }
}
