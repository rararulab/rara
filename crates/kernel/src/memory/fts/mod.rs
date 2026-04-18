//! SQLite FTS5 full-text index for tape search.
//!
//! This is a **derived index** — the JSONL tape files remain the source of
//! truth.  If the FTS database is missing or corrupt, it can be rebuilt from
//! JSONL on next access.
//!
//! SQL operations are isolated in [`repo`]; this module contains only
//! business logic (entry filtering, text extraction, query sanitization).

mod repo;
mod tokenizer;

use serde_json::Value;
use sqlx::SqlitePool;
pub(crate) use tokenizer::warmup as warmup_tokenizer;
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
    /// Only entries with `id > high-water-mark` are indexed.  Updates the
    /// `tape_fts_meta` high-water mark on success.
    pub(crate) async fn index_entries(
        &self,
        tape_name: &str,
        session_key: &str,
        entries: &[TapEntry],
    ) -> Result<usize, sqlx::Error> {
        let hwm = repo::get_hwm(&self.pool, tape_name).await.unwrap_or(0) as u64;

        let new_entries: Vec<_> = entries.iter().filter(|e| e.id > hwm).collect();

        if new_entries.is_empty() {
            return Ok(0);
        }

        // Track the highest entry ID we've seen (not just indexed) so
        // non-Message entries are not re-scanned on subsequent calls.
        let max_id = new_entries.iter().map(|e| e.id).max().unwrap_or(hwm);

        let indexable: Vec<TapEntry> = new_entries
            .iter()
            .filter(|e| e.kind == TapEntryKind::Message)
            .map(|e| (*e).clone())
            .collect();

        // Run jieba segmentation on the blocking pool — it's CPU-bound
        // and can be slow on long messages (code blocks, transcripts).
        // Keeping it off the runtime thread avoids stalling other async
        // work while we index.
        let segmented: Vec<(u64, String, String)> = tokio::task::spawn_blocking(move || {
            indexable
                .into_iter()
                .filter_map(|entry| {
                    let content = extract_fts_content(&entry);
                    (!content.is_empty()).then(|| (entry.id, entry.kind.to_string(), content))
                })
                .collect()
        })
        .await
        .map_err(|e| sqlx::Error::Protocol(format!("fts segment task: {e}")))?;

        let mut tx = self.pool.begin().await?;
        let mut count = 0usize;

        for (entry_id, kind_str, content) in &segmented {
            repo::insert(
                &mut tx,
                content,
                tape_name,
                kind_str,
                *entry_id as i64,
                session_key,
            )
            .await?;

            count += 1;
        }

        repo::upsert_hwm(&mut tx, tape_name, max_id as i64).await?;
        tx.commit().await?;

        debug!(tape_name, count, max_id, "FTS indexed entries");
        Ok(count)
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

        let rows = repo::search(&self.pool, &fts_query, tape_filter, limit as i64).await?;

        Ok(rows
            .into_iter()
            .map(|r| FtsHit {
                entry_id:   r.entry_id as u64,
                tape_name:  r.tape_name,
                bm25_score: r.bm25_rank,
            })
            .collect())
    }

    /// Return the high-water mark (last indexed entry ID) for a tape.
    pub(crate) async fn last_indexed_id(&self, tape_name: &str) -> Result<u64, sqlx::Error> {
        Ok(repo::get_hwm(&self.pool, tape_name).await? as u64)
    }

    /// Remove all FTS entries for a tape (used on reset/archive/delete).
    pub(crate) async fn remove_tape(&self, tape_name: &str) -> Result<(), sqlx::Error> {
        repo::delete_by_tape(&self.pool, tape_name).await?;
        debug!(tape_name, "FTS entries removed");
        Ok(())
    }

    /// Delete all FTS data (full reset).
    pub(crate) async fn clear_all(&self) -> Result<(), sqlx::Error> {
        repo::delete_all(&self.pool).await?;
        warn!("FTS index cleared — will rebuild on next access");
        Ok(())
    }
}

/// Extract searchable text from a tape entry for FTS indexing.
///
/// Returns only JSON string leaves (payload + metadata) joined by
/// newlines — object keys, numbers, and JSON punctuation are dropped so
/// the jieba segmenter doesn't chew on structural noise (e.g. `"role"`,
/// `{`, `}`). The brute-force search path keeps its fuller text surface
/// via [`super::service::extract_searchable_text`].
fn extract_fts_content(entry: &TapEntry) -> String {
    let mut parts = Vec::new();
    collect_json_strings(&entry.payload, &mut parts);
    if let Some(meta) = entry.metadata.as_ref() {
        collect_json_strings(meta, &mut parts);
    }
    tokenizer::segment(&parts.join("\n"))
}

/// Recursively push every string leaf in `value` into `out`.
fn collect_json_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) if !s.is_empty() => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| collect_json_strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_json_strings(v, out)),
        _ => {}
    }
}

/// Sanitize a user query for FTS5 MATCH syntax.
///
/// FTS5 interprets special characters (`*`, `"`, `OR`, `AND`, `NOT`, etc.).
/// We quote each term to treat them as literals, then join with spaces
/// (implicit AND).
fn sanitize_fts_query(query: &str) -> String {
    // Pre-segment so CJK terms match the segmented index surface.
    let segmented = tokenizer::segment(query);
    segmented
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|term| {
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
        let content = extract_fts_content(&entry);
        assert!(content.contains("hello world"));
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
    fn extract_fts_content_empty_payload() {
        let entry = TapEntry {
            id:        3,
            kind:      TapEntryKind::Event,
            payload:   serde_json::json!({}),
            timestamp: jiff::Timestamp::now(),
            metadata:  None,
        };
        let content = extract_fts_content(&entry);
        assert!(!content.contains("hello"));
    }

    /// Helper: create an in-memory pool with the FTS schema.
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
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
        pool
    }

    #[tokio::test]
    async fn roundtrip_index_and_search() {
        let pool = test_pool().await;
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

    /// Verifies jieba pre-segmentation makes CJK BM25 work: a query for a
    /// segmented word ("机器学习") must retrieve the right entry even when
    /// the word appears inside a longer sentence.
    #[tokio::test]
    async fn chinese_segmentation_enables_bm25() {
        let pool = test_pool().await;
        let fts = TapeFts::new(pool);

        let entries = vec![
            TapEntry {
                id:        1,
                kind:      TapEntryKind::Message,
                payload:   serde_json::json!({"content": "今天学习了机器学习的基础知识"}),
                timestamp: jiff::Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        2,
                kind:      TapEntryKind::Message,
                payload:   serde_json::json!({"content": "买了一台新的机器"}),
                timestamp: jiff::Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        3,
                kind:      TapEntryKind::Message,
                payload:   serde_json::json!({"content": "unrelated English content"}),
                timestamp: jiff::Timestamp::now(),
                metadata:  None,
            },
        ];

        fts.index_entries("cn-tape", "session-cn", &entries)
            .await
            .expect("index");

        // "机器学习" should hit entry 1 (where jieba keeps it as one word).
        let hits = fts
            .search("机器学习", Some("cn-tape"), 10)
            .await
            .expect("search ml");
        assert!(
            hits.iter().any(|h| h.entry_id == 1),
            "expected entry 1 to match '机器学习', got {hits:?}"
        );

        // "机器" alone should retrieve both CJK entries.
        let hits = fts
            .search("机器", Some("cn-tape"), 10)
            .await
            .expect("search machine");
        let ids: Vec<u64> = hits.iter().map(|h| h.entry_id).collect();
        assert!(ids.contains(&1), "entry 1 should match '机器', got {ids:?}");
        assert!(ids.contains(&2), "entry 2 should match '机器', got {ids:?}");
    }
}
