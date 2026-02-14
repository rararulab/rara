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

//! PostgreSQL implementation for memory storage.
//!
//! This backend is the production-default in the current deployment model.
//! It uses:
//! - `memory_files` for file metadata.
//! - `memory_chunks` for chunk content and optional embeddings.
//! - `memory_embedding_cache` for embedding reuse.
//! - PostgreSQL full-text search (`tsvector/tsquery`) for keyword retrieval.

use std::future::Future;

use sqlx::{PgPool, Row};

use crate::{
    manager::{ChunkDetail, MemoryError, MemoryResult},
    store::{ChunkInput, EmbeddedChunkRow, IndexedFileMeta, MemorySearchRow, MemoryStore},
};

const INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS memory_files (
  id BIGSERIAL PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  hash TEXT NOT NULL,
  mtime BIGINT NOT NULL,
  size BIGINT NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS memory_chunks (
  id BIGSERIAL PRIMARY KEY,
  file_id BIGINT NOT NULL REFERENCES memory_files(id) ON DELETE CASCADE,
  chunk_index BIGINT NOT NULL,
  content TEXT NOT NULL,
  embedding BYTEA,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(file_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_file_idx
  ON memory_chunks(file_id, chunk_index);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_content_tsv
  ON memory_chunks USING GIN (to_tsvector('simple', content));

CREATE TABLE IF NOT EXISTS memory_embedding_cache (
  id BIGSERIAL PRIMARY KEY,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  text_hash TEXT NOT NULL,
  dim INTEGER NOT NULL,
  embedding BYTEA NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(provider, model, text_hash)
);
"#;

#[derive(Debug, Clone)]
pub struct PgMemoryStore {
    pool: PgPool,
}

impl PgMemoryStore {
    /// Create a store backed by a shared PostgreSQL pool.
    pub fn new(pool: PgPool) -> Self { Self { pool } }

    /// Bridge async sqlx operations into the sync store trait.
    ///
    /// `MemoryStore` is synchronous by design so `MemoryManager` can run heavy
    /// I/O inside `spawn_blocking`. This helper executes async sqlx calls on
    /// the current Tokio runtime handle.
    fn block_on<F, T>(&self, future: F) -> MemoryResult<T>
    where
        F: Future<Output = Result<T, sqlx::Error>>,
    {
        let handle = tokio::runtime::Handle::try_current().map_err(|e| MemoryError::Other {
            message: format!("tokio runtime not available for pg memory store: {e}"),
        })?;
        let result = handle.block_on(future)?;
        Ok(result)
    }
}

impl MemoryStore for PgMemoryStore {
    fn ensure_schema(&self) -> MemoryResult<()> {
        self.block_on(sqlx::query(INIT_SQL).execute(&self.pool))?;
        Ok(())
    }

    fn list_files(&self) -> MemoryResult<Vec<IndexedFileMeta>> {
        let rows = self.block_on(
            sqlx::query("SELECT path, hash, mtime, size FROM memory_files")
                .fetch_all(&self.pool),
        )?;

        Ok(rows
            .into_iter()
            .map(|row| IndexedFileMeta {
                path: row.get::<String, _>("path"),
                hash: row.get::<String, _>("hash"),
                mtime: row.get::<i64, _>("mtime"),
                size: row.get::<i64, _>("size"),
            })
            .collect())
    }

    fn upsert_file_chunks(
        &self,
        path: &str,
        hash: &str,
        mtime: i64,
        size: i64,
        chunks: &[ChunkInput],
    ) -> MemoryResult<()> {
        self.block_on(async {
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
                    "INSERT INTO memory_chunks(file_id, chunk_index, content, embedding) VALUES($1, $2, $3, $4)",
                )
                .bind(file_id)
                .bind(chunk.chunk_index)
                .bind(&chunk.content)
                .bind(chunk.embedding.as_ref().map(|v| f32_vec_to_blob(v)))
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        })?;

        Ok(())
    }

    fn delete_files(&self, paths: &[String]) -> MemoryResult<()> {
        if paths.is_empty() {
            return Ok(());
        }

        self.block_on(
            sqlx::query("DELETE FROM memory_files WHERE path = ANY($1)")
                .bind(paths)
                .execute(&self.pool),
        )?;

        Ok(())
    }

    fn keyword_search(&self, query: &str, limit: usize) -> MemoryResult<Vec<MemorySearchRow>> {
        let rows = self.block_on(
            sqlx::query(
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
            .fetch_all(&self.pool),
        )?;

        Ok(rows
            .into_iter()
            .map(|row| MemorySearchRow {
                chunk_id: row.get::<i64, _>("id"),
                path: row.get::<String, _>("path"),
                chunk_index: row.get::<i64, _>("chunk_index"),
                content: row.get::<String, _>("content"),
                score: row.get::<f64, _>("score"),
            })
            .collect())
    }

    fn list_embedded_chunks(&self, limit: usize) -> MemoryResult<Vec<EmbeddedChunkRow>> {
        let rows = self.block_on(
            sqlx::query(
                r#"
                SELECT c.id, f.path, c.chunk_index, c.content, c.embedding
                FROM memory_chunks c
                JOIN memory_files f ON f.id = c.file_id
                WHERE c.embedding IS NOT NULL
                ORDER BY c.id DESC
                LIMIT $1
                "#,
            )
            .bind(i64::try_from(limit).unwrap_or(5000))
            .fetch_all(&self.pool),
        )?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let blob: Vec<u8> = row.get("embedding");
                EmbeddedChunkRow {
                    chunk_id: row.get::<i64, _>("id"),
                    path: row.get::<String, _>("path"),
                    chunk_index: row.get::<i64, _>("chunk_index"),
                    content: row.get::<String, _>("content"),
                    embedding: blob_to_f32_vec(&blob),
                }
            })
            .collect())
    }

    fn list_embedded_chunks_by_path(&self, path: &str) -> MemoryResult<Vec<EmbeddedChunkRow>> {
        let rows = self.block_on(
            sqlx::query(
                r#"
                SELECT c.id, f.path, c.chunk_index, c.content, c.embedding
                FROM memory_chunks c
                JOIN memory_files f ON f.id = c.file_id
                WHERE f.path = $1 AND c.embedding IS NOT NULL
                ORDER BY c.chunk_index ASC
                "#,
            )
            .bind(path)
            .fetch_all(&self.pool),
        )?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let blob: Vec<u8> = row.get("embedding");
                EmbeddedChunkRow {
                    chunk_id: row.get::<i64, _>("id"),
                    path: row.get::<String, _>("path"),
                    chunk_index: row.get::<i64, _>("chunk_index"),
                    content: row.get::<String, _>("content"),
                    embedding: blob_to_f32_vec(&blob),
                }
            })
            .collect())
    }

    fn get_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        text_hash: &str,
    ) -> MemoryResult<Option<Vec<f32>>> {
        let row = self.block_on(
            sqlx::query(
                r#"
                SELECT embedding
                FROM memory_embedding_cache
                WHERE provider = $1 AND model = $2 AND text_hash = $3
                "#,
            )
            .bind(provider)
            .bind(model)
            .bind(text_hash)
            .fetch_optional(&self.pool),
        )?;

        Ok(row.map(|row| {
            let blob: Vec<u8> = row.get("embedding");
            blob_to_f32_vec(&blob)
        }))
    }

    fn put_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        text_hash: &str,
        embedding: &[f32],
    ) -> MemoryResult<()> {
        self.block_on(
            sqlx::query(
                r#"
                INSERT INTO memory_embedding_cache(provider, model, text_hash, dim, embedding)
                VALUES($1, $2, $3, $4, $5)
                ON CONFLICT(provider, model, text_hash)
                DO UPDATE SET dim = EXCLUDED.dim, embedding = EXCLUDED.embedding
                "#,
            )
            .bind(provider)
            .bind(model)
            .bind(text_hash)
            .bind(i32::try_from(embedding.len()).unwrap_or(0))
            .bind(f32_vec_to_blob(embedding))
            .execute(&self.pool),
        )?;

        Ok(())
    }

    fn get_chunk(&self, chunk_id: i64) -> MemoryResult<Option<ChunkDetail>> {
        let row = self.block_on(
            sqlx::query(
                r#"
                SELECT c.id, f.path, c.chunk_index, c.content
                FROM memory_chunks c
                JOIN memory_files f ON f.id = c.file_id
                WHERE c.id = $1
                "#,
            )
            .bind(chunk_id)
            .fetch_optional(&self.pool),
        )?;

        Ok(row.map(|row| ChunkDetail {
            chunk_id: row.get::<i64, _>("id"),
            path: row.get::<String, _>("path"),
            chunk_index: row.get::<i64, _>("chunk_index"),
            content: row.get::<String, _>("content"),
        }))
    }
}

fn f32_vec_to_blob(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<u8>>()
}

/// Decode little-endian f32 bytes into a vector.
fn blob_to_f32_vec(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect()
}
