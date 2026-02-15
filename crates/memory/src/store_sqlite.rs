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
//!
//! Note: The `chunks.embedding` column and `embedding_cache` table are
//! retained in the schema for backward compatibility but are no longer
//! written to. Chroma handles embeddings server-side.

use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    manager::{ChunkDetail, MemoryResult},
    store::{ChunkInput, IndexedFileMeta, MemorySearchRow, MemoryStore},
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
                "INSERT INTO chunks(file_id, chunk_index, content) VALUES(?1, ?2, ?3)",
                params![
                    file_id,
                    chunk.chunk_index,
                    chunk.content,
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
