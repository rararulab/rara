//! SQLite FTS5 full-text index for tape search.
//!
//! This is a **derived index** — the JSONL tape files remain the source of
//! truth.  If the FTS database is missing or corrupt, it can be rebuilt from
//! JSONL on next access.
//!
//! The index lives in the shared `sqlx::SqlitePool` managed by `rara-model`,
//! alongside the `chat_session` and other tables.

use sqlx::SqlitePool;
use tracing::{debug, warn};

use super::{TapEntry, TapEntryKind};

/// A hit returned by [`TapeFts::search`].
#[derive(Debug, Clone)]
pub(crate) struct FtsHit {
    /// Entry ID from the original tape (for joining back to the in-memory
    /// cache).
    pub entry_id:   u64,
    /// Tape name the entry belongs to.
    pub tape_name:  String,
    /// BM25 relevance score (lower = more relevant in FTS5).
    pub bm25_score: f64,
}

/// Async FTS5 index backed by the shared SQLite pool.
///
/// All operations are best-effort — callers should fall back to brute-force
/// search when FTS returns an error.
#[derive(Debug, Clone)]
pub(crate) struct TapeFts {
    pool: SqlitePool,
}

impl TapeFts {
    /// Create a new FTS handle using the shared pool.
    ///
    /// The pool must already have the `tape_fts` virtual table (created by
    /// the `tape_fts_init` migration).
    pub(crate) fn new(pool: SqlitePool) -> Self { Self { pool } }

    /// Index a batch of tape entries into FTS5.
    ///
    /// Only entries with `id > after_id` are indexed.  Updates the
    /// `tape_fts_meta` high-water mark on success.
    pub(crate) async fn index_entries(
        &self,
        tape_name: &str,
        session_key: &str,
        entries: &[TapEntry],
    ) -> Result<usize, sqlx::Error> {
        let hwm = self.last_indexed_id(tape_name).await.unwrap_or(0);

        let new_entries: Vec<_> = entries.iter().filter(|e| e.id > hwm).collect();

        if new_entries.is_empty() {
            return Ok(0);
        }

        // Track the highest entry ID we've seen (not just indexed) so
        // non-Message entries are not re-scanned on subsequent calls.
        let max_id = new_entries.iter().map(|e| e.id).max().unwrap_or(hwm);

        let indexable: Vec<_> = new_entries
            .iter()
            .filter(|e| e.kind == TapEntryKind::Message)
            .collect();

        let mut tx = self.pool.begin().await?;
        let mut count = 0u64;

        for entry in &indexable {
            let content = extract_fts_content(entry);
            if content.is_empty() {
                continue;
            }
            let kind_str = entry.kind.to_string();
            let entry_id = entry.id as i64;

            sqlx::query(
                "INSERT INTO tape_fts (content, tape_name, entry_kind, entry_id, session_key) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&content)
            .bind(tape_name)
            .bind(&kind_str)
            .bind(entry_id)
            .bind(session_key)
            .execute(&mut *tx)
            .await?;

            count += 1;
        }

        // Update high-water mark.
        let max_id_i64 = max_id as i64;
        sqlx::query(
            "INSERT INTO tape_fts_meta (tape_name, last_indexed_id) VALUES (?, ?) ON \
             CONFLICT(tape_name) DO UPDATE SET last_indexed_id = excluded.last_indexed_id",
        )
        .bind(tape_name)
        .bind(max_id_i64)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        debug!(tape_name, count, max_id, "FTS indexed entries");
        Ok(count as usize)
    }

    /// Query FTS5 for matching entries.
    ///
    /// Returns hits sorted by BM25 relevance.  `tape_filter` restricts
    /// results to a single tape when `Some`.
    pub(crate) async fn search(
        &self,
        query: &str,
        tape_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<FtsHit>, sqlx::Error> {
        let fts_query = sanitize_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let limit_i64 = limit as i64;

        let rows: Vec<(i64, String, f64)> = if let Some(tape) = tape_filter {
            sqlx::query_as(
                "SELECT entry_id, tape_name, bm25(tape_fts) AS rank FROM tape_fts WHERE tape_fts \
                 MATCH ? AND tape_name = ? ORDER BY rank LIMIT ?",
            )
            .bind(&fts_query)
            .bind(tape)
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT entry_id, tape_name, bm25(tape_fts) AS rank FROM tape_fts WHERE tape_fts \
                 MATCH ? ORDER BY rank LIMIT ?",
            )
            .bind(&fts_query)
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await?
        };

        let hits = rows
            .into_iter()
            .map(|(entry_id, tape_name, bm25_score)| FtsHit {
                entry_id: entry_id as u64,
                tape_name,
                bm25_score,
            })
            .collect();

        Ok(hits)
    }

    /// Return the high-water mark (last indexed entry ID) for a tape.
    pub(crate) async fn last_indexed_id(&self, tape_name: &str) -> Result<u64, sqlx::Error> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT last_indexed_id FROM tape_fts_meta WHERE tape_name = ?")
                .bind(tape_name)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|(id,)| id as u64).unwrap_or(0))
    }

    /// Remove all FTS entries for a tape (used on reset/archive).
    pub(crate) async fn remove_tape(&self, tape_name: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM tape_fts WHERE tape_name = ?")
            .bind(tape_name)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM tape_fts_meta WHERE tape_name = ?")
            .bind(tape_name)
            .execute(&self.pool)
            .await?;
        debug!(tape_name, "FTS entries removed");
        Ok(())
    }

    /// Delete all FTS data (full reset).
    pub(crate) async fn clear_all(&self) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM tape_fts")
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM tape_fts_meta")
            .execute(&self.pool)
            .await?;
        warn!("FTS index cleared — will rebuild on next access");
        Ok(())
    }
}

/// Extract searchable text from a tape entry for FTS indexing.
///
/// Mirrors the logic in `service::extract_searchable_text` but kept
/// deliberately simple — FTS5 tokenization handles normalization.
fn extract_fts_content(entry: &TapEntry) -> String {
    let mut parts = Vec::new();
    if let Some(text) = entry.payload.get("content").and_then(|v| v.as_str()) {
        parts.push(text);
    }
    if let Some(meta) = &entry.metadata {
        if let Some(text) = meta.as_str() {
            parts.push(text);
        } else if let Some(obj) = meta.as_object() {
            for v in obj.values() {
                if let Some(s) = v.as_str() {
                    parts.push(s);
                }
            }
        }
    }
    parts.join(" ")
}

/// Sanitize a user query for FTS5 MATCH syntax.
///
/// FTS5 interprets special characters (`*`, `"`, `OR`, `AND`, `NOT`, etc.).
/// We quote each term to treat them as literals, then join with spaces
/// (implicit AND).
fn sanitize_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|term| {
            // Wrap each term in double quotes to escape FTS5 operators.
            // Escape any embedded double quotes.
            let escaped = term.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_fts_query_basic() {
        assert_eq!(sanitize_fts_query("hello world"), "\"hello\" \"world\"");
    }

    #[test]
    fn sanitize_fts_query_special_chars() {
        assert_eq!(sanitize_fts_query("foo*bar"), "\"foo*bar\"");
    }

    #[test]
    fn sanitize_fts_query_empty() {
        assert_eq!(sanitize_fts_query("   "), "");
    }

    #[test]
    fn sanitize_fts_query_embedded_quotes() {
        assert_eq!(sanitize_fts_query(r#"say "hi""#), "\"say\" \"\"\"hi\"\"\"");
    }

    #[test]
    fn extract_fts_content_message() {
        let entry = TapEntry {
            id:        1,
            kind:      TapEntryKind::Message,
            payload:   serde_json::json!({"content": "hello world"}),
            timestamp: jiff::Timestamp::now(),
            metadata:  None,
        };
        assert_eq!(extract_fts_content(&entry), "hello world");
    }

    #[test]
    fn extract_fts_content_with_metadata() {
        let entry = TapEntry {
            id:        2,
            kind:      TapEntryKind::Message,
            payload:   serde_json::json!({"content": "main text"}),
            timestamp: jiff::Timestamp::now(),
            metadata:  Some(serde_json::json!({"model": "gpt-4", "note": "test"})),
        };
        let content = extract_fts_content(&entry);
        assert!(content.contains("main text"));
        assert!(content.contains("gpt-4"));
        assert!(content.contains("test"));
    }

    #[test]
    fn extract_fts_content_empty() {
        let entry = TapEntry {
            id:        3,
            kind:      TapEntryKind::Event,
            payload:   serde_json::json!({}),
            timestamp: jiff::Timestamp::now(),
            metadata:  None,
        };
        assert_eq!(extract_fts_content(&entry), "");
    }

    #[tokio::test]
    async fn roundtrip_index_and_search() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory pool");

        // Create schema manually (migrations don't run on :memory:).
        sqlx::query(
            "CREATE VIRTUAL TABLE tape_fts USING fts5(content, tape_name UNINDEXED, entry_kind \
             UNINDEXED, entry_id UNINDEXED, session_key UNINDEXED, tokenize = 'unicode61 \
             remove_diacritics 2')",
        )
        .execute(&pool)
        .await
        .expect("create fts table");

        sqlx::query(
            "CREATE TABLE tape_fts_meta (tape_name TEXT PRIMARY KEY, last_indexed_id INTEGER NOT \
             NULL DEFAULT 0)",
        )
        .execute(&pool)
        .await
        .expect("create meta table");

        let fts = TapeFts::new(pool);

        let entries = vec![
            TapEntry {
                id:        1,
                kind:      TapEntryKind::Message,
                payload:   serde_json::json!({"content": "Rust ownership model"}),
                timestamp: jiff::Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        2,
                kind:      TapEntryKind::Message,
                payload:   serde_json::json!({"content": "Python garbage collector"}),
                timestamp: jiff::Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        3,
                kind:      TapEntryKind::ToolCall,
                payload:   serde_json::json!({"name": "bash"}),
                timestamp: jiff::Timestamp::now(),
                metadata:  None,
            },
        ];

        // Index
        let count = fts
            .index_entries("test-tape", "session-1", &entries)
            .await
            .expect("index");
        assert_eq!(count, 2, "should index 2 Message entries, skip ToolCall");

        // Search
        let hits = fts
            .search("rust ownership", Some("test-tape"), 10)
            .await
            .expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry_id, 1);

        // Search across all tapes
        let hits = fts.search("python", None, 10).await.expect("search all");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry_id, 2);

        // High-water mark
        let hwm = fts.last_indexed_id("test-tape").await.expect("hwm");
        assert_eq!(hwm, 3);

        // Re-index is idempotent (no new entries above hwm)
        let count = fts
            .index_entries("test-tape", "session-1", &entries)
            .await
            .expect("re-index");
        assert_eq!(count, 0);

        // Remove tape
        fts.remove_tape("test-tape").await.expect("remove");
        let hits = fts
            .search("rust", Some("test-tape"), 10)
            .await
            .expect("search after remove");
        assert!(hits.is_empty());
    }
}
