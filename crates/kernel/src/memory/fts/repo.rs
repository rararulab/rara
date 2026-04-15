//! Pure data-access layer for the FTS5 index.
//!
//! All SQL lives here. Business logic (filtering, text extraction,
//! high-water-mark decisions) stays in the parent module.

use sqlx::SqlitePool;

/// Row returned by an FTS5 search query.
pub(crate) struct FtsRow {
    pub entry_id:  i64,
    pub tape_name: String,
    pub bm25_rank: f64,
}

/// Insert a single entry into the FTS5 index.
pub(super) async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    content: &str,
    tape_name: &str,
    entry_kind: &str,
    entry_id: i64,
    session_key: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO tape_fts (content, tape_name, entry_kind, entry_id, session_key) VALUES (?, \
         ?, ?, ?, ?)",
    )
    .bind(content)
    .bind(tape_name)
    .bind(entry_kind)
    .bind(entry_id)
    .bind(session_key)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Update (or insert) the high-water mark for a tape.
pub(super) async fn upsert_hwm(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    tape_name: &str,
    last_indexed_id: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO tape_fts_meta (tape_name, last_indexed_id) VALUES (?, ?) ON \
         CONFLICT(tape_name) DO UPDATE SET last_indexed_id = excluded.last_indexed_id",
    )
    .bind(tape_name)
    .bind(last_indexed_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Read the high-water mark for a tape. Returns 0 if not found.
pub(super) async fn get_hwm(pool: &SqlitePool, tape_name: &str) -> sqlx::Result<i64> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT last_indexed_id FROM tape_fts_meta WHERE tape_name = ?")
            .bind(tape_name)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(id,)| id).unwrap_or(0))
}

/// Search the FTS5 index. Returns rows sorted by BM25 relevance.
pub(super) async fn search(
    pool: &SqlitePool,
    fts_query: &str,
    tape_filter: Option<&str>,
    limit: i64,
) -> sqlx::Result<Vec<FtsRow>> {
    let rows: Vec<(i64, String, f64)> = if let Some(tape) = tape_filter {
        sqlx::query_as(
            "SELECT entry_id, tape_name, bm25(tape_fts) AS rank FROM tape_fts WHERE tape_fts \
             MATCH ? AND tape_name = ? ORDER BY rank LIMIT ?",
        )
        .bind(fts_query)
        .bind(tape)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT entry_id, tape_name, bm25(tape_fts) AS rank FROM tape_fts WHERE tape_fts \
             MATCH ? ORDER BY rank LIMIT ?",
        )
        .bind(fts_query)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    Ok(rows
        .into_iter()
        .map(|(entry_id, tape_name, bm25_rank)| FtsRow {
            entry_id,
            tape_name,
            bm25_rank,
        })
        .collect())
}

/// Delete all FTS entries for a specific tape.
pub(super) async fn delete_by_tape(pool: &SqlitePool, tape_name: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM tape_fts WHERE tape_name = ?")
        .bind(tape_name)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM tape_fts_meta WHERE tape_name = ?")
        .bind(tape_name)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete all FTS data (full reset).
pub(super) async fn delete_all(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM tape_fts").execute(pool).await?;
    sqlx::query("DELETE FROM tape_fts_meta")
        .execute(pool)
        .await?;
    Ok(())
}
