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

//! Execution trace construction and storage.
//!
//! Each agent turn produces an [`ExecutionTrace`] summarizing duration, token
//! usage, tool calls, and plan steps. Traces are persisted to SQLite so that
//! any channel adapter can retrieve them later (e.g. Telegram inline buttons
//! that open the full turn detail, or the web chat UI execution-trace pane).
//!
//! Submodules:
//! - [`builder`] — incrementally assembles an [`ExecutionTrace`] while the
//!   agent turn runs, by observing [`crate::io::StreamEvent`]s. The kernel turn
//!   driver owns one builder per turn and attaches it to the
//!   [`crate::io::StreamHandle`] so every `emit` both broadcasts to channel
//!   adapters and feeds the trace accumulator.
//! - [`tool_display`] — pure tool-name/argument formatting helpers shared by
//!   the builder (to render persisted tool entries) and channel adapters (for
//!   live progress displays).

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use diesel::{
    ExpressionMethods, QueryDsl, Queryable, Selectable, SelectableHelper, sql_types::Text,
};
use diesel_async::RunQueryDsl;
use rara_model::schema::execution_traces;
use snafu::ResultExt;
use yunara_store::diesel_pool::DieselSqlitePools;

use crate::error::{DieselPoolSnafu, DieselSnafu, JsonSnafu, Result};

pub mod builder;
pub mod tool_display;

pub use builder::TraceBuilder;

/// Summary of a single agent turn execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionTrace {
    pub duration_secs:    u64,
    pub iterations:       usize,
    pub model:            String,
    pub input_tokens:     u32,
    pub output_tokens:    u32,
    pub thinking_ms:      u64,
    /// Truncated reasoning text (first ~500 chars).
    pub thinking_preview: String,
    /// Plan steps with status.
    pub plan_steps:       Vec<String>,
    /// High-level rationale the LLM stated for this turn (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_rationale:   Option<String>,
    /// Tool execution records.
    pub tools:            Vec<ToolTraceEntry>,
    /// Per-turn correlation handle. Stable across every entry produced by
    /// the same inbound message. Accepts the legacy `rara_message_id` key
    /// on read for `execution_traces.trace_data` rows persisted before
    /// issue #1978.
    #[serde(alias = "rara_message_id")]
    pub rara_turn_id:     String,
}

/// Record of a single tool invocation within a turn.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolTraceEntry {
    pub name:        String,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    pub success:     bool,
    pub summary:     String,
    pub error:       Option<String>,
}

const TRACE_RETENTION_DAYS: u32 = 30;
const CLEANUP_INTERVAL: u32 = 100;

/// Row projection for a stored trace's serialized payload.
#[derive(Queryable, Selectable)]
#[diesel(table_name = execution_traces)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
struct TraceDataRow {
    trace_data: String,
}

/// Row projection for a trace's session_id.
#[derive(Queryable, Selectable)]
#[diesel(table_name = execution_traces)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
struct TraceSessionRow {
    session_id: String,
}

/// Row projection for `(session_id, trace_data)` lookups by message id.
#[derive(Queryable, Selectable)]
#[diesel(table_name = execution_traces)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
struct TraceSessionAndDataRow {
    session_id: String,
    trace_data: String,
}

/// Persistent store for execution traces backed by SQLite.
///
/// Traces older than 30 days are automatically cleaned up every 100 saves.
#[derive(Debug, Clone)]
pub struct TraceService {
    pools:      DieselSqlitePools,
    save_count: Arc<AtomicU32>,
}

impl TraceService {
    pub fn new(pools: DieselSqlitePools) -> Self {
        Self {
            pools,
            save_count: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Save an execution trace. Returns the generated ULID.
    #[tracing::instrument(skip_all)]
    pub async fn save(&self, session_id: &str, trace: &ExecutionTrace) -> Result<String> {
        let id = ulid::Ulid::new().to_string();
        let trace_data = serde_json::to_string(trace).context(JsonSnafu)?;

        let mut conn = self.pools.writer.get().await.context(DieselPoolSnafu)?;
        diesel::insert_into(execution_traces::table)
            .values((
                execution_traces::id.eq(&id),
                execution_traces::session_id.eq(session_id),
                execution_traces::trace_data.eq(&trace_data),
            ))
            .execute(&mut *conn)
            .await
            .context(DieselSnafu)?;

        // Periodically clean up old traces.
        if self.save_count.fetch_add(1, Ordering::Relaxed) % CLEANUP_INTERVAL == 0 {
            let writer = self.pools.writer.clone();
            tokio::spawn(async move {
                let cutoff = format!("-{TRACE_RETENTION_DAYS} days");
                let mut conn = match writer.get().await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to acquire diesel conn for trace cleanup");
                        return;
                    }
                };
                // `created_at < datetime('now', ?)` — diesel has no cross-backend
                // DSL for sqlite's `datetime()`; emit it via the sanctioned
                // `sql::<Text>` helper per docs/guides/db-diesel-migration.md.
                let cutoff_expr = diesel::dsl::sql::<Text>("datetime('now', ")
                    .bind::<Text, _>(cutoff)
                    .sql(")");
                if let Err(e) = diesel::delete(
                    execution_traces::table.filter(execution_traces::created_at.lt(cutoff_expr)),
                )
                .execute(&mut *conn)
                .await
                {
                    tracing::warn!(error = %e, "failed to clean up old execution traces");
                }
            });
        }

        Ok(id)
    }

    /// Retrieve an execution trace by ID.
    #[tracing::instrument(skip_all)]
    pub async fn get(&self, id: &str) -> Result<Option<ExecutionTrace>> {
        use diesel::OptionalExtension;

        let mut conn = self.pools.reader.get().await.context(DieselPoolSnafu)?;
        let row: Option<TraceDataRow> = execution_traces::table
            .filter(execution_traces::id.eq(id))
            .select(TraceDataRow::as_select())
            .first(&mut *conn)
            .await
            .optional()
            .context(DieselSnafu)?;

        match row {
            Some(r) => {
                let trace = serde_json::from_str(&r.trace_data).context(JsonSnafu)?;
                Ok(Some(trace))
            }
            None => Ok(None),
        }
    }

    /// Retrieve the session_id associated with a trace.
    #[tracing::instrument(skip_all)]
    pub async fn get_session_id(&self, id: &str) -> Result<Option<String>> {
        use diesel::OptionalExtension;

        let mut conn = self.pools.reader.get().await.context(DieselPoolSnafu)?;
        let row: Option<TraceSessionRow> = execution_traces::table
            .filter(execution_traces::id.eq(id))
            .select(TraceSessionRow::as_select())
            .first(&mut *conn)
            .await
            .optional()
            .context(DieselSnafu)?;
        Ok(row.map(|r| r.session_id))
    }

    /// Find the session and full execution trace for a `rara_turn_id`.
    ///
    /// Returns the indexed `session_id` column plus the parsed
    /// [`ExecutionTrace`] from `trace_data` — the trace already aggregates
    /// model, tokens, iterations, thinking, tools, plan steps, and
    /// rationale, so callers do not need to re-derive these from tape
    /// entries.
    ///
    /// Reads both `$.rara_turn_id` (current key) and `$.rara_message_id`
    /// (legacy key) via `COALESCE`. Existing rows written before issue
    /// #1978 carry the legacy key; new writes only emit the new key.
    #[tracing::instrument(skip_all)]
    pub async fn find_trace_by_turn_id(
        &self,
        turn_id: &str,
    ) -> Result<Option<(String, ExecutionTrace)>> {
        use diesel::OptionalExtension;

        let mut conn = self.pools.reader.get().await.context(DieselPoolSnafu)?;

        // No diesel DSL exists for SQLite JSON1; emit the predicate as raw
        // SQL per docs/guides/db-diesel-migration.md. `COALESCE` lets one
        // query match both the new and legacy keys without a UNION.
        let predicate = diesel::dsl::sql::<diesel::sql_types::Bool>(
            "COALESCE(json_extract(trace_data, '$.rara_turn_id'), json_extract(trace_data, \
             '$.rara_message_id')) = ",
        )
        .bind::<Text, _>(turn_id);

        let row: Option<TraceSessionAndDataRow> = execution_traces::table
            .filter(predicate)
            .select(TraceSessionAndDataRow::as_select())
            .limit(1)
            .first(&mut *conn)
            .await
            .optional()
            .context(DieselSnafu)?;

        match row {
            Some(r) => {
                let trace = serde_json::from_str(&r.trace_data).context(JsonSnafu)?;
                Ok(Some((r.session_id, trace)))
            }
            None => Ok(None),
        }
    }

    /// Delete traces older than `retention_days`. Returns the number of rows
    /// removed.
    #[tracing::instrument(skip_all)]
    pub async fn cleanup(&self, retention_days: u32) -> Result<u64> {
        let mut conn = self.pools.writer.get().await.context(DieselPoolSnafu)?;
        let cutoff = format!("-{retention_days} days");
        let cutoff_expr = diesel::dsl::sql::<Text>("datetime('now', ")
            .bind::<Text, _>(cutoff)
            .sql(")");
        let affected = diesel::delete(
            execution_traces::table.filter(execution_traces::created_at.lt(cutoff_expr)),
        )
        .execute(&mut *conn)
        .await
        .context(DieselSnafu)?;
        Ok(affected as u64)
    }
}
