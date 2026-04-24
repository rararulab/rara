//! Pure data-access layer for the FTS5 index.
//!
//! All SQL lives here. Business logic (filtering, text extraction,
//! high-water-mark decisions) stays in the parent module.
//!
//! `tape_fts` is an FTS5 virtual table — diesel's DSL does not express
//! `MATCH` or FTS5-specific functions like `bm25()`, and `tape_fts` is not
//! part of the generated `schema.rs`. Per
//! docs/guides/db-diesel-migration.md, these narrow cases are the sanctioned
//! `sql_query` / `sql::<T>` escape-hatch sites. `tape_fts_meta` is a regular
//! table and uses the DSL as usual.

use diesel::{
    ExpressionMethods, OptionalExtension, QueryDsl, QueryableByName,
    sql_types::{BigInt, Double, Text},
    upsert::excluded,
};
use diesel_async::RunQueryDsl;
use rara_model::schema::tape_fts_meta;
use snafu::ResultExt;
use yunara_store::diesel_pool::DieselSqliteConnection;

use crate::error::{DieselSnafu, Result};

/// Row returned by an FTS5 search query.
pub(crate) struct FtsRow {
    pub entry_id:  i64,
    pub tape_name: String,
    pub bm25_rank: f64,
}

/// Queryable projection for the raw FTS5 search output.
#[derive(QueryableByName)]
struct FtsSearchRow {
    #[diesel(sql_type = BigInt)]
    entry_id:  i64,
    #[diesel(sql_type = Text)]
    tape_name: String,
    #[diesel(sql_type = Double)]
    rank:      f64,
}

/// Insert a single entry into the FTS5 index.
///
/// FTS5: diesel DSL does not express INSERTs into virtual tables — see
/// docs/guides/db-diesel-migration.md.
pub(super) async fn insert(
    conn: &mut DieselSqliteConnection,
    content: &str,
    tape_name: &str,
    entry_kind: &str,
    entry_id: i64,
    session_key: &str,
) -> Result<()> {
    diesel::sql_query(
        "INSERT INTO tape_fts (content, tape_name, entry_kind, entry_id, session_key) VALUES (?, \
         ?, ?, ?, ?)",
    )
    .bind::<Text, _>(content)
    .bind::<Text, _>(tape_name)
    .bind::<Text, _>(entry_kind)
    .bind::<BigInt, _>(entry_id)
    .bind::<Text, _>(session_key)
    .execute(conn)
    .await
    .context(DieselSnafu)?;
    Ok(())
}

/// Update (or insert) the high-water mark for a tape.
pub(super) async fn upsert_hwm(
    conn: &mut DieselSqliteConnection,
    tape_name: &str,
    last_indexed_id: i64,
) -> Result<()> {
    diesel::insert_into(tape_fts_meta::table)
        .values((
            tape_fts_meta::tape_name.eq(tape_name),
            tape_fts_meta::last_indexed_id.eq(last_indexed_id as i32),
        ))
        .on_conflict(tape_fts_meta::tape_name)
        .do_update()
        .set(tape_fts_meta::last_indexed_id.eq(excluded(tape_fts_meta::last_indexed_id)))
        .execute(conn)
        .await
        .context(DieselSnafu)?;
    Ok(())
}

/// Read the high-water mark for a tape. Returns 0 if not found.
pub(super) async fn get_hwm(conn: &mut DieselSqliteConnection, tape_name: &str) -> Result<i64> {
    let row: Option<i32> = tape_fts_meta::table
        .filter(tape_fts_meta::tape_name.eq(tape_name))
        .select(tape_fts_meta::last_indexed_id)
        .first(conn)
        .await
        .optional()
        .context(DieselSnafu)?;
    Ok(row.map(i64::from).unwrap_or(0))
}

/// Search the FTS5 index. Returns rows sorted by BM25 relevance.
///
/// FTS5: diesel DSL does not express `MATCH` or `bm25()` — see
/// docs/guides/db-diesel-migration.md.
pub(super) async fn search(
    conn: &mut DieselSqliteConnection,
    fts_query: &str,
    tape_filter: Option<&str>,
    limit: i64,
) -> Result<Vec<FtsRow>> {
    let rows: Vec<FtsSearchRow> = if let Some(tape) = tape_filter {
        diesel::sql_query(
            "SELECT entry_id, tape_name, bm25(tape_fts) AS rank FROM tape_fts WHERE tape_fts \
             MATCH ? AND tape_name = ? ORDER BY rank LIMIT ?",
        )
        .bind::<Text, _>(fts_query)
        .bind::<Text, _>(tape)
        .bind::<BigInt, _>(limit)
        .load(conn)
        .await
        .context(DieselSnafu)?
    } else {
        diesel::sql_query(
            "SELECT entry_id, tape_name, bm25(tape_fts) AS rank FROM tape_fts WHERE tape_fts \
             MATCH ? ORDER BY rank LIMIT ?",
        )
        .bind::<Text, _>(fts_query)
        .bind::<BigInt, _>(limit)
        .load(conn)
        .await
        .context(DieselSnafu)?
    };

    Ok(rows
        .into_iter()
        .map(|r| FtsRow {
            entry_id:  r.entry_id,
            tape_name: r.tape_name,
            bm25_rank: r.rank,
        })
        .collect())
}

/// Delete all FTS entries for a specific tape.
///
/// FTS5: `DELETE` on the virtual table is also outside DSL coverage.
pub(super) async fn delete_by_tape(
    conn: &mut DieselSqliteConnection,
    tape_name: &str,
) -> Result<()> {
    diesel::sql_query("DELETE FROM tape_fts WHERE tape_name = ?")
        .bind::<Text, _>(tape_name)
        .execute(&mut *conn)
        .await
        .context(DieselSnafu)?;
    diesel::delete(tape_fts_meta::table.filter(tape_fts_meta::tape_name.eq(tape_name)))
        .execute(conn)
        .await
        .context(DieselSnafu)?;
    Ok(())
}

/// Delete all FTS data (full reset).
pub(super) async fn delete_all(conn: &mut DieselSqliteConnection) -> Result<()> {
    diesel::sql_query("DELETE FROM tape_fts")
        .execute(&mut *conn)
        .await
        .context(DieselSnafu)?;
    diesel::delete(tape_fts_meta::table)
        .execute(conn)
        .await
        .context(DieselSnafu)?;
    Ok(())
}
