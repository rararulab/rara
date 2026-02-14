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

//! SQLite-backed memory store with FTS5 full-text search.

use std::path::Path;

use rusqlite::params;
use snafu::ResultExt;

use crate::{
    error::{self},
    types::{MemoryChunk, MemoryDocument, SearchResult},
};

/// SQLite memory store with FTS5 indexing for full-text search.
pub struct SqliteMemoryStore {
    conn: tokio::sync::Mutex<rusqlite::Connection>,
}

impl SqliteMemoryStore {
    /// Open (or create) a SQLite database at `db_path` and initialize the
    /// schema.
    pub fn open(db_path: &Path) -> error::Result<Self> {
        let conn = rusqlite::Connection::open(db_path).context(error::SqliteSnafu)?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                hash TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
                doc_id,
                chunk_id,
                heading,
                content,
                tokenize='unicode61'
            );
            ",
        )
        .context(error::SqliteSnafu)?;

        Ok(Self {
            conn: tokio::sync::Mutex::new(conn),
        })
    }

    /// Create an in-memory store (useful for testing).
    #[cfg(test)]
    pub fn open_in_memory() -> error::Result<Self> {
        let conn = rusqlite::Connection::open_in_memory().context(error::SqliteSnafu)?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                hash TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
                doc_id,
                chunk_id,
                heading,
                content,
                tokenize='unicode61'
            );
            ",
        )
        .context(error::SqliteSnafu)?;

        Ok(Self {
            conn: tokio::sync::Mutex::new(conn),
        })
    }

    /// Insert or update a document and its chunks.
    pub async fn upsert_document(&self, doc: &MemoryDocument) -> error::Result<()> {
        let conn = self.conn.lock().await;

        // Delete existing chunks for this document.
        conn.execute(
            "DELETE FROM chunks_fts WHERE doc_id = ?1",
            params![doc.id],
        )
        .context(error::SqliteSnafu)?;

        // Upsert the document row.
        conn.execute(
            "INSERT OR REPLACE INTO documents (id, title, content, hash, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![doc.id, doc.title, doc.content, doc.hash, doc.updated_at],
        )
        .context(error::SqliteSnafu)?;

        // Insert chunks into FTS.
        for chunk in &doc.chunks {
            conn.execute(
                "INSERT INTO chunks_fts (doc_id, chunk_id, heading, content) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    chunk.doc_id,
                    chunk.chunk_id,
                    chunk.heading,
                    chunk.content
                ],
            )
            .context(error::SqliteSnafu)?;
        }

        Ok(())
    }

    /// Delete a document and its chunks.
    pub async fn delete_document(&self, doc_id: &str) -> error::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM chunks_fts WHERE doc_id = ?1",
            params![doc_id],
        )
        .context(error::SqliteSnafu)?;
        conn.execute("DELETE FROM documents WHERE id = ?1", params![doc_id])
            .context(error::SqliteSnafu)?;
        Ok(())
    }

    /// Retrieve a document by ID, including its chunks.
    pub async fn get_document(
        &self,
        doc_id: &str,
    ) -> error::Result<Option<MemoryDocument>> {
        let conn = self.conn.lock().await;

        let mut stmt = conn
            .prepare(
                "SELECT id, title, content, hash, updated_at FROM documents WHERE id = ?1",
            )
            .context(error::SqliteSnafu)?;

        let doc = stmt
            .query_row(params![doc_id], |row| {
                Ok(MemoryDocument {
                    id:         row.get(0)?,
                    title:      row.get(1)?,
                    content:    row.get(2)?,
                    hash:       row.get(3)?,
                    updated_at: row.get(4)?,
                    chunks:     Vec::new(),
                })
            })
            .optional()
            .context(error::SqliteSnafu)?;

        let Some(mut doc) = doc else {
            return Ok(None);
        };

        // Fetch associated chunks.
        let mut chunk_stmt = conn
            .prepare(
                "SELECT doc_id, chunk_id, heading, content, rowid FROM chunks_fts \
                 WHERE doc_id = ?1 ORDER BY rowid",
            )
            .context(error::SqliteSnafu)?;

        let chunks = chunk_stmt
            .query_map(params![doc_id], |row| {
                let doc_id: String = row.get(0)?;
                let chunk_id: String = row.get(1)?;
                let heading: Option<String> = row.get(2)?;
                let content: String = row.get(3)?;
                let rowid: i64 = row.get(4)?;
                Ok(MemoryChunk {
                    chunk_id,
                    doc_id,
                    content,
                    heading,
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    chunk_index: rowid as u32,
                })
            })
            .context(error::SqliteSnafu)?
            .collect::<Result<Vec<_>, _>>()
            .context(error::SqliteSnafu)?;

        doc.chunks = chunks;
        Ok(Some(doc))
    }

    /// Retrieve only the stored hash for a document (fast path for sync).
    pub async fn get_document_hash(
        &self,
        doc_id: &str,
    ) -> error::Result<Option<String>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare("SELECT hash FROM documents WHERE id = ?1")
            .context(error::SqliteSnafu)?;

        stmt.query_row(params![doc_id], |row| row.get(0))
            .optional()
            .context(error::SqliteSnafu)
    }

    /// Search the FTS index.
    ///
    /// Returns results ranked by BM25 relevance. The `snippet()` function
    /// highlights matches with `<b>…</b>` markers.
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> error::Result<Vec<SearchResult>> {
        let conn = self.conn.lock().await;

        let mut stmt = conn
            .prepare(
                "SELECT doc_id, chunk_id, heading, \
                    snippet(chunks_fts, 3, '<b>', '</b>', '...', 64), \
                    bm25(chunks_fts) \
                 FROM chunks_fts \
                 WHERE chunks_fts MATCH ?1 \
                 ORDER BY bm25(chunks_fts) \
                 LIMIT ?2",
            )
            .context(error::SqliteSnafu)?;

        #[allow(clippy::cast_possible_wrap)]
        let results = stmt
            .query_map(params![query, limit as i64], |row| {
                Ok(SearchResult {
                    doc_id:   row.get(0)?,
                    chunk_id: row.get(1)?,
                    heading:  row.get(2)?,
                    snippet:  row.get(3)?,
                    rank:     row.get(4)?,
                })
            })
            .context(error::SqliteSnafu)?
            .collect::<Result<Vec<_>, _>>()
            .context(error::SqliteSnafu)?;

        Ok(results)
    }

    /// List all indexed documents as `(id, hash)` pairs.
    pub async fn list_documents(&self) -> error::Result<Vec<(String, String)>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare("SELECT id, hash FROM documents")
            .context(error::SqliteSnafu)?;

        let docs = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .context(error::SqliteSnafu)?
            .collect::<Result<Vec<_>, _>>()
            .context(error::SqliteSnafu)?;

        Ok(docs)
    }
}

/// Extension trait for `rusqlite::Result<T>` to provide `optional()`.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryChunk, MemoryDocument};

    fn make_doc(id: &str, content: &str) -> MemoryDocument {
        MemoryDocument {
            id:         id.to_owned(),
            title:      format!("Title for {id}"),
            content:    content.to_owned(),
            chunks:     vec![MemoryChunk {
                chunk_id:    format!("{id}#0"),
                doc_id:      id.to_owned(),
                content:     content.to_owned(),
                heading:     None,
                chunk_index: 0,
            }],
            hash:       "abc123".to_owned(),
            updated_at: 1000,
        }
    }

    #[tokio::test]
    async fn test_upsert_and_get() {
        let store = SqliteMemoryStore::open_in_memory().unwrap();
        let doc = make_doc("test.md", "Hello world");

        store.upsert_document(&doc).await.unwrap();
        let retrieved = store.get_document("test.md").await.unwrap().unwrap();

        assert_eq!(retrieved.id, "test.md");
        assert_eq!(retrieved.title, "Title for test.md");
        assert_eq!(retrieved.content, "Hello world");
        assert_eq!(retrieved.hash, "abc123");
        assert_eq!(retrieved.chunks.len(), 1);
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let store = SqliteMemoryStore::open_in_memory().unwrap();
        let result = store.get_document("nope.md").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete() {
        let store = SqliteMemoryStore::open_in_memory().unwrap();
        let doc = make_doc("del.md", "To be deleted");

        store.upsert_document(&doc).await.unwrap();
        assert!(store.get_document("del.md").await.unwrap().is_some());

        store.delete_document("del.md").await.unwrap();
        assert!(store.get_document("del.md").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_search() {
        let store = SqliteMemoryStore::open_in_memory().unwrap();
        let doc1 = make_doc("rust.md", "Rust is a systems programming language");
        let doc2 = make_doc("python.md", "Python is great for data science");

        store.upsert_document(&doc1).await.unwrap();
        store.upsert_document(&doc2).await.unwrap();

        let results = store.search("rust programming", 10).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].doc_id, "rust.md");
    }

    #[tokio::test]
    async fn test_list_documents() {
        let store = SqliteMemoryStore::open_in_memory().unwrap();
        let doc1 = make_doc("a.md", "AAA");
        let doc2 = make_doc("b.md", "BBB");

        store.upsert_document(&doc1).await.unwrap();
        store.upsert_document(&doc2).await.unwrap();

        let docs = store.list_documents().await.unwrap();
        assert_eq!(docs.len(), 2);
    }

    #[tokio::test]
    async fn test_get_document_hash() {
        let store = SqliteMemoryStore::open_in_memory().unwrap();
        let doc = make_doc("hash.md", "content");

        store.upsert_document(&doc).await.unwrap();

        let hash = store.get_document_hash("hash.md").await.unwrap().unwrap();
        assert_eq!(hash, "abc123");

        let none = store.get_document_hash("missing.md").await.unwrap();
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn test_upsert_replaces_chunks() {
        let store = SqliteMemoryStore::open_in_memory().unwrap();
        let mut doc = make_doc("update.md", "Original content");
        store.upsert_document(&doc).await.unwrap();

        // Update with new content and chunks.
        doc.content = "Updated content".to_owned();
        doc.hash = "new_hash".to_owned();
        doc.chunks = vec![
            MemoryChunk {
                chunk_id:    "update.md#0".to_owned(),
                doc_id:      "update.md".to_owned(),
                content:     "Updated chunk 1".to_owned(),
                heading:     Some("Section A".to_owned()),
                chunk_index: 0,
            },
            MemoryChunk {
                chunk_id:    "update.md#1".to_owned(),
                doc_id:      "update.md".to_owned(),
                content:     "Updated chunk 2".to_owned(),
                heading:     Some("Section B".to_owned()),
                chunk_index: 1,
            },
        ];
        store.upsert_document(&doc).await.unwrap();

        let retrieved = store.get_document("update.md").await.unwrap().unwrap();
        assert_eq!(retrieved.hash, "new_hash");
        assert_eq!(retrieved.chunks.len(), 2);
    }
}
