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

//! SQLite implementation for memory storage.
//!
//! This backend is optimized for local development and fallback operation.
//! It combines:
//! - Relational metadata/chunk storage.
//! - FTS5 (`chunks_fts`) for keyword search.
//! - A local embedding cache table.

use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    manager::{ChunkDetail, MemoryResult},
    store::{ChunkInput, EmbeddedChunkRow, IndexedFileMeta, MemorySearchRow, MemoryStore},
};

const INIT_SQL: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  path TEXT NOT NULL UNIQUE,
  hash TEXT NOT NULL,
  mtime INTEGER NOT NULL,
  size INTEGER NOT NULL,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS chunks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  chunk_index INTEGER NOT NULL,
  content TEXT NOT NULL,
  embedding BLOB,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(file_id, chunk_index)
);

CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
  path,
  content,
  chunk_id UNINDEXED,
  tokenize = 'porter unicode61'
);

CREATE TABLE IF NOT EXISTS embedding_cache (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  text_hash TEXT NOT NULL,
  dim INTEGER NOT NULL,
  embedding BLOB NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(provider, model, text_hash)
);

CREATE INDEX IF NOT EXISTS idx_chunks_file_idx ON chunks(file_id, chunk_index);
"#;

/// SQLite-backed memory store.
#[derive(Debug, Clone)]
pub struct SqliteMemoryStore {
    db_path: PathBuf,
}

impl SqliteMemoryStore {
    /// Create a SQLite store bound to the given database file path.
    pub fn new(db_path: PathBuf) -> Self { Self { db_path } }

    /// Open a connection and enforce FK constraints.
    fn open(&self) -> MemoryResult<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(conn)
    }

    /// Delete a file and all of its chunk/FTS rows inside a transaction.
    fn delete_file_by_path(tx: &rusqlite::Transaction<'_>, path: &str) -> MemoryResult<()> {
        let file_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .optional()?;

        let Some(file_id) = file_id else {
            return Ok(());
        };

        let mut stmt = tx.prepare("SELECT id FROM chunks WHERE file_id = ?1")?;
        let chunk_ids = stmt
            .query_map(params![file_id], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        for chunk_id in chunk_ids {
            tx.execute(
                "DELETE FROM chunks_fts WHERE chunk_id = ?1",
                params![chunk_id.to_string()],
            )?;
        }

        tx.execute("DELETE FROM chunks WHERE file_id = ?1", params![file_id])?;
        tx.execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
        Ok(())
    }
}

impl MemoryStore for SqliteMemoryStore {
    fn ensure_schema(&self) -> MemoryResult<()> {
        let conn = self.open()?;
        conn.execute_batch(INIT_SQL)?;
        Ok(())
    }

    fn list_files(&self) -> MemoryResult<Vec<IndexedFileMeta>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT path, hash, mtime, size FROM files")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(IndexedFileMeta {
                    path: row.get(0)?,
                    hash: row.get(1)?,
                    mtime: row.get(2)?,
                    size: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn upsert_file_chunks(
        &self,
        path: &str,
        hash: &str,
        mtime: i64,
        size: i64,
        chunks: &[ChunkInput],
    ) -> MemoryResult<()> {
        let mut conn = self.open()?;
        let tx = conn.transaction()?;

        Self::delete_file_by_path(&tx, path)?;

        tx.execute(
            "INSERT INTO files(path, hash, mtime, size) VALUES(?1, ?2, ?3, ?4)",
            params![path, hash, mtime, size],
        )?;
        let file_id = tx.last_insert_rowid();

        for chunk in chunks {
            tx.execute(
                "INSERT INTO chunks(file_id, chunk_index, content, embedding) VALUES(?1, ?2, ?3, ?4)",
                params![
                    file_id,
                    chunk.chunk_index,
                    chunk.content,
                    chunk.embedding.as_ref().map(|it| f32_vec_to_blob(it)),
                ],
            )?;
            let chunk_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO chunks_fts(path, content, chunk_id) VALUES(?1, ?2, ?3)",
                params![path, chunk.content, chunk_id.to_string()],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    fn delete_files(&self, paths: &[String]) -> MemoryResult<()> {
        if paths.is_empty() {
            return Ok(());
        }

        let mut conn = self.open()?;
        let tx = conn.transaction()?;
        for path in paths {
            Self::delete_file_by_path(&tx, path)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn keyword_search(&self, query: &str, limit: usize) -> MemoryResult<Vec<MemorySearchRow>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT c.id, f.path, c.chunk_index, c.content, bm25(chunks_fts) AS score
            FROM chunks_fts
            JOIN chunks c ON c.id = CAST(chunks_fts.chunk_id AS INTEGER)
            JOIN files f ON f.id = c.file_id
            WHERE chunks_fts MATCH ?1
            ORDER BY score
            LIMIT ?2
            "#,
        )?;

        let rows = stmt
            .query_map(params![query, i64::try_from(limit).unwrap_or(20)], |row| {
                Ok(MemorySearchRow {
                    chunk_id: row.get(0)?,
                    path: row.get(1)?,
                    chunk_index: row.get(2)?,
                    content: row.get(3)?,
                    score: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    fn list_embedded_chunks(&self, limit: usize) -> MemoryResult<Vec<EmbeddedChunkRow>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT c.id, f.path, c.chunk_index, c.content, c.embedding
            FROM chunks c
            JOIN files f ON f.id = c.file_id
            WHERE c.embedding IS NOT NULL
            ORDER BY c.id DESC
            LIMIT ?1
            "#,
        )?;

        let rows = stmt
            .query_map(params![i64::try_from(limit).unwrap_or(5000)], |row| {
                let blob: Vec<u8> = row.get(4)?;
                Ok(EmbeddedChunkRow {
                    chunk_id: row.get(0)?,
                    path: row.get(1)?,
                    chunk_index: row.get(2)?,
                    content: row.get(3)?,
                    embedding: blob_to_f32_vec(&blob),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    fn list_embedded_chunks_by_path(&self, path: &str) -> MemoryResult<Vec<EmbeddedChunkRow>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT c.id, f.path, c.chunk_index, c.content, c.embedding
            FROM chunks c
            JOIN files f ON f.id = c.file_id
            WHERE f.path = ?1 AND c.embedding IS NOT NULL
            ORDER BY c.chunk_index ASC
            "#,
        )?;

        let rows = stmt
            .query_map(params![path], |row| {
                let blob: Vec<u8> = row.get(4)?;
                Ok(EmbeddedChunkRow {
                    chunk_id: row.get(0)?,
                    path: row.get(1)?,
                    chunk_index: row.get(2)?,
                    content: row.get(3)?,
                    embedding: blob_to_f32_vec(&blob),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    fn get_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        text_hash: &str,
    ) -> MemoryResult<Option<Vec<f32>>> {
        let conn = self.open()?;
        let result = conn
            .query_row(
                r#"
                SELECT embedding
                FROM embedding_cache
                WHERE provider = ?1 AND model = ?2 AND text_hash = ?3
                "#,
                params![provider, model, text_hash],
                |row| {
                    let blob: Vec<u8> = row.get(0)?;
                    Ok(blob_to_f32_vec(&blob))
                },
            )
            .optional()?;

        Ok(result)
    }

    fn put_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        text_hash: &str,
        embedding: &[f32],
    ) -> MemoryResult<()> {
        let conn = self.open()?;
        conn.execute(
            r#"
            INSERT INTO embedding_cache(provider, model, text_hash, dim, embedding)
            VALUES(?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(provider, model, text_hash)
            DO UPDATE SET dim = excluded.dim, embedding = excluded.embedding
            "#,
            params![
                provider,
                model,
                text_hash,
                i64::try_from(embedding.len()).unwrap_or(0),
                f32_vec_to_blob(embedding),
            ],
        )?;
        Ok(())
    }

    fn get_chunk(&self, chunk_id: i64) -> MemoryResult<Option<ChunkDetail>> {
        let conn = self.open()?;
        let detail = conn
            .query_row(
                r#"
                SELECT c.id, f.path, c.chunk_index, c.content
                FROM chunks c
                JOIN files f ON f.id = c.file_id
                WHERE c.id = ?1
                "#,
                params![chunk_id],
                |row| {
                    Ok(ChunkDetail {
                        chunk_id: row.get(0)?,
                        path: row.get(1)?,
                        chunk_index: row.get(2)?,
                        content: row.get(3)?,
                    })
                },
            )
            .optional()?;

        Ok(detail)
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
