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

//! Outcome feedback for the knowledge layer.
//!
//! Defines [`OutcomeKind`] — the polarity-carrying enum the kernel hands to
//! [`KnowledgeService::commit_outcome`](super::service::KnowledgeService::commit_outcome) —
//! and [`apply_ema`], the pure EMA update used to move
//! `memory_items.confidence` in `[0, 1]` based on accumulated outcomes.
//!
//! Keeping the enum + math in this file lets the EMA be unit-tested
//! without diesel, and gives the persistence path (which lives in
//! [`KnowledgeService::commit_outcome`](super::service::KnowledgeService::commit_outcome))
//! one small file to import from rather than reaching into `items.rs`.

use diesel::{ExpressionMethods, QueryDsl, result::OptionalExtension};
use diesel_async::{AsyncConnection, RunQueryDsl};
use rara_model::schema::{memory_items, memory_outcomes};
use snafu::ResultExt;
use yunara_store::diesel_pool::DieselSqlitePool;

use crate::error::{DieselPoolSnafu, DieselSnafu, Result};

/// EMA learning rate used by [`apply_ema`].
///
/// At `α = 0.2`, five outcomes of the same polarity move confidence by
/// roughly 67 % toward the bound — which matches the "you have to be
/// wrong a handful of times before rara stops trusting you" intuition
/// the ranking layer assumes.
///
/// This is a mechanism-tuning constant (no operator-relevant right
/// value), so per the project spec it lives as a `const` next to the
/// mechanism rather than as a YAML knob.
pub const CONFIDENCE_ALPHA: f32 = 0.2;

/// Kind of outcome reported against a set of memory items.
///
/// The polarity table is:
///
/// - [`OutcomeKind::Success`] / [`OutcomeKind::Confirmed`] → positive update
///   (`c' = c + α (1 − c)`).
/// - [`OutcomeKind::Failure`] / [`OutcomeKind::Contradicted`] → negative update
///   (`c' = c − α c`).
///
/// Both pairs collapse to two cases in the EMA, but the audit row
/// preserves the source distinction: `Success` typically comes from
/// automated tool-result correlation, while `Confirmed` typically comes
/// from explicit user feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    /// Tool-result-style positive signal.
    Success,
    /// Tool-result-style negative signal.
    Failure,
    /// User-feedback-style positive signal.
    Confirmed,
    /// User-feedback-style negative signal.
    Contradicted,
}

impl OutcomeKind {
    /// Stable string used both as the `memory_outcomes.outcome_kind`
    /// column value and as the canonical wire / log representation.
    pub fn as_str(self) -> &'static str {
        match self {
            OutcomeKind::Success => "success",
            OutcomeKind::Failure => "failure",
            OutcomeKind::Confirmed => "confirmed",
            OutcomeKind::Contradicted => "contradicted",
        }
    }

    /// Whether this outcome moves confidence up (`true`) or down (`false`).
    pub fn is_positive(self) -> bool {
        matches!(self, OutcomeKind::Success | OutcomeKind::Confirmed)
    }
}

/// Pure EMA update for a single outcome.
///
/// Bounded by construction in `[0, 1]` for any starting `current` in
/// `[0, 1]`: positive update `c' = c + α(1 − c)` lands in `[c, 1]`;
/// negative update `c' = c − α c` lands in `[0, c]`. No clamping needed.
pub fn apply_ema(current: f32, kind: OutcomeKind) -> f32 {
    if kind.is_positive() {
        CONFIDENCE_ALPHA.mul_add(1.0 - current, current)
    } else {
        CONFIDENCE_ALPHA.mul_add(-current, current)
    }
}

/// Write one `memory_outcomes` row per id in `item_ids` and update the
/// parent `memory_items.confidence` via [`apply_ema`], all in a single
/// writer transaction.
///
/// - Empty `item_ids` is a no-op (returns Ok).
/// - Ids with no matching `memory_items` row are silently skipped. The
///   transaction still commits any successful pairs.
///
/// The `(write memory_outcomes row, update memory_items.confidence)`
/// pair is the unit of truth: if only one half lands, the future
/// `explain()` trace starts lying. Diesel's `transaction()` is the
/// right boundary; the writer pool already serializes writes.
pub(super) async fn commit_outcome_inner(
    writer: &DieselSqlitePool,
    item_ids: &[i64],
    kind: OutcomeKind,
    tape_entry_id: Option<i64>,
) -> Result<()> {
    if item_ids.is_empty() {
        return Ok(());
    }

    let ids_i32: Vec<i32> = item_ids.iter().map(|&v| v as i32).collect();
    let tape_entry_id_i32: Option<i32> = tape_entry_id.map(|v| v as i32);
    let kind_str = kind.as_str();

    let mut conn = writer.get().await.context(DieselPoolSnafu)?;
    conn.transaction::<_, diesel::result::Error, _>(async |conn| {
        for id in &ids_i32 {
            // Read the current confidence. If no row matches, skip — the
            // outer call still returns Ok per spec.
            let current: Option<f32> = memory_items::table
                .filter(memory_items::id.eq(*id))
                .select(memory_items::confidence)
                .first::<f32>(&mut *conn)
                .await
                .optional()?;
            let Some(current) = current else {
                continue;
            };

            // Insert the audit row.
            diesel::insert_into(memory_outcomes::table)
                .values((
                    memory_outcomes::item_id.eq(*id),
                    memory_outcomes::outcome_kind.eq(kind_str),
                    memory_outcomes::tape_entry_id.eq(tape_entry_id_i32),
                ))
                .execute(&mut *conn)
                .await?;

            // Update the parent's confidence.
            let updated = apply_ema(current, kind);
            diesel::update(memory_items::table.filter(memory_items::id.eq(*id)))
                .set(memory_items::confidence.eq(updated))
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    })
    .await
    .context(DieselSnafu)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use diesel_async::RunQueryDsl as _;
    use yunara_store::diesel_pool::DieselSqlitePool;

    use super::*;

    /// Mirrors the DDL bundled into the
    /// `2026-05-21-000000_memory_confidence_outcomes` migration. The
    /// shared test pool's bootstrap schema still tracks the
    /// pre-migration `memory_items` shape, so each test opts in.
    const ADD_CONFIDENCE_SQL: &str =
        "ALTER TABLE memory_items ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0";
    const CREATE_OUTCOMES_SQL: &str =
        "CREATE TABLE memory_outcomes (id INTEGER PRIMARY KEY AUTOINCREMENT, item_id INTEGER NOT \
         NULL REFERENCES memory_items(id) ON DELETE CASCADE, outcome_kind TEXT NOT NULL, \
         tape_entry_id INTEGER, created_at TEXT NOT NULL DEFAULT (datetime('now')))";
    const CREATE_OUTCOMES_INDEX_SQL: &str =
        "CREATE INDEX idx_memory_outcomes_item_id ON memory_outcomes(item_id)";

    async fn pool_with_migration() -> DieselSqlitePool {
        let pools = crate::testing::build_memory_diesel_pools().await;
        let mut conn = pools.writer.get().await.expect("pool conn");
        for ddl in [
            ADD_CONFIDENCE_SQL,
            CREATE_OUTCOMES_SQL,
            CREATE_OUTCOMES_INDEX_SQL,
        ] {
            diesel::sql_query(ddl)
                .execute(&mut *conn)
                .await
                .expect("apply migration ddl");
        }
        drop(conn);
        pools.writer
    }

    async fn insert_item_with_confidence(pool: &DieselSqlitePool, confidence: f32) -> i64 {
        use rara_model::schema::memory_items;
        let mut conn = pool.get().await.expect("pool conn");
        let id: Option<i32> = diesel::insert_into(memory_items::table)
            .values((
                memory_items::username.eq("alice"),
                memory_items::content.eq("fact"),
                memory_items::memory_type.eq("preference"),
                memory_items::category.eq("ui"),
                memory_items::confidence.eq(confidence),
            ))
            .returning(memory_items::id)
            .get_result(&mut *conn)
            .await
            .expect("insert row");
        id.map(i64::from).unwrap_or(0)
    }

    async fn fetch_confidence(pool: &DieselSqlitePool, id: i64) -> f32 {
        use rara_model::schema::memory_items;
        let mut conn = pool.get().await.expect("pool conn");
        memory_items::table
            .filter(memory_items::id.eq(id as i32))
            .select(memory_items::confidence)
            .first::<f32>(&mut *conn)
            .await
            .expect("fetch confidence")
    }

    #[test]
    fn ema_positive_monotone_toward_one() {
        let mut c: f32 = 0.5;
        let mut prev = c;
        for _ in 0..5 {
            c = apply_ema(c, OutcomeKind::Confirmed);
            assert!(c > prev, "must strictly increase: prev={prev} next={c}");
            assert!(c <= 1.0, "must stay <= 1");
            assert!(c >= prev, "must stay >= previous (>= starting c)");
            prev = c;
        }
        // Sanity: after five Confirmed updates from 0.5 the value should
        // be well past 0.8 but still below 1.
        assert!(c > 0.8 && c < 1.0, "expected (0.8, 1.0), got {c}");
    }

    #[test]
    fn ema_negative_monotone_toward_zero() {
        let mut c: f32 = 0.5;
        let mut prev = c;
        for _ in 0..5 {
            c = apply_ema(c, OutcomeKind::Contradicted);
            assert!(c < prev, "must strictly decrease: prev={prev} next={c}");
            assert!(c >= 0.0, "must stay >= 0");
            assert!(c <= prev, "must stay <= previous (<= starting c)");
            prev = c;
        }
        // Sanity: after five Contradicted updates from 0.5 the value
        // should be well below 0.2 but still positive.
        assert!(c > 0.0 && c < 0.2, "expected (0.0, 0.2), got {c}");
    }

    #[tokio::test]
    async fn commit_outcome_writes_row_and_updates_confidence() {
        let pool = pool_with_migration().await;
        let id = insert_item_with_confidence(&pool, 1.0).await;

        commit_outcome_inner(&pool, &[id], OutcomeKind::Failure, Some(42))
            .await
            .expect("commit_outcome ok");

        // Audit row landed with the expected polarity + tape entry id.
        let mut conn = pool.get().await.expect("pool conn");
        #[derive(diesel::QueryableByName)]
        struct Row {
            #[diesel(sql_type = diesel::sql_types::Text)]
            outcome_kind:  String,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
            tape_entry_id: Option<i32>,
        }
        let rows: Vec<Row> = diesel::sql_query(
            "SELECT outcome_kind, tape_entry_id FROM memory_outcomes WHERE item_id = ?",
        )
        .bind::<diesel::sql_types::Integer, _>(id as i32)
        .load(&mut *conn)
        .await
        .expect("query outcome row");
        assert_eq!(rows.len(), 1, "exactly one outcome row");
        assert_eq!(rows[0].outcome_kind, "failure");
        assert_eq!(rows[0].tape_entry_id, Some(42));
        drop(conn);

        // Parent confidence applied the negative EMA: c=1.0, α=0.2 →
        // 1.0 − 0.2·1.0 = 0.8.
        let after = fetch_confidence(&pool, id).await;
        assert!(
            (after - 0.8).abs() < 1e-6,
            "expected 0.8 after one Failure, got {after}"
        );
    }

    #[tokio::test]
    async fn commit_outcome_skips_unknown_ids() {
        let pool = pool_with_migration().await;
        let valid = insert_item_with_confidence(&pool, 1.0).await;
        let unknown: i64 = 999_999;

        commit_outcome_inner(&pool, &[valid, unknown], OutcomeKind::Failure, None)
            .await
            .expect("commit_outcome returns Ok even with unknown ids");

        // Valid row changed via the same EMA as the previous test.
        let after = fetch_confidence(&pool, valid).await;
        assert!(
            (after - 0.8).abs() < 1e-6,
            "valid row's confidence must update: got {after}"
        );

        // No outcome row written for the unknown id.
        let mut conn = pool.get().await.expect("pool conn");
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type = diesel::sql_types::Integer)]
            n: i32,
        }
        let rows: Vec<Count> = diesel::sql_query("SELECT COUNT(*) AS n FROM memory_outcomes")
            .load(&mut *conn)
            .await
            .expect("count outcomes");
        assert_eq!(rows[0].n, 1, "only the valid id produces an audit row");
    }
}
