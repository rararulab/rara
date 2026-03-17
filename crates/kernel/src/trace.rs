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

//! Execution trace storage.
//!
//! Each agent turn produces an [`ExecutionTrace`] summarizing duration, token
//! usage, tool calls, and plan steps. Traces are persisted to SQLite so that
//! any channel adapter can retrieve them later (e.g. Telegram inline buttons).

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use sqlx::SqlitePool;

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
    /// Tool execution records.
    pub tools:            Vec<ToolTraceEntry>,
    /// Rara internal message ID for end-to-end correlation.
    pub rara_message_id:  String,
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

/// Persistent store for execution traces backed by SQLite.
///
/// Traces older than 30 days are automatically cleaned up every 100 saves.
#[derive(Debug, Clone)]
pub struct TraceService {
    pool:       SqlitePool,
    save_count: Arc<AtomicU32>,
}

impl TraceService {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            save_count: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Save an execution trace. Returns the generated ULID.
    pub async fn save(
        &self,
        session_id: &str,
        trace: &ExecutionTrace,
    ) -> Result<String, sqlx::Error> {
        let id = ulid::Ulid::new().to_string();
        let trace_data =
            serde_json::to_string(trace).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

        sqlx::query("INSERT INTO execution_traces (id, session_id, trace_data) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(session_id)
            .bind(&trace_data)
            .execute(&self.pool)
            .await?;

        // Periodically clean up old traces.
        if self.save_count.fetch_add(1, Ordering::Relaxed) % CLEANUP_INTERVAL == 0 {
            let pool = self.pool.clone();
            tokio::spawn(async move {
                if let Err(e) = sqlx::query(
                    "DELETE FROM execution_traces WHERE created_at < datetime('now', ?)",
                )
                .bind(format!("-{TRACE_RETENTION_DAYS} days"))
                .execute(&pool)
                .await
                {
                    tracing::warn!(error = %e, "failed to clean up old execution traces");
                }
            });
        }

        Ok(id)
    }

    /// Retrieve an execution trace by ID.
    pub async fn get(&self, id: &str) -> Result<Option<ExecutionTrace>, sqlx::Error> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT trace_data FROM execution_traces WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        match row {
            Some((data,)) => {
                let trace = serde_json::from_str(&data)
                    .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
                Ok(Some(trace))
            }
            None => Ok(None),
        }
    }

    /// Delete traces older than `retention_days`. Returns the number of rows
    /// removed.
    pub async fn cleanup(&self, retention_days: u32) -> Result<u64, sqlx::Error> {
        let result =
            sqlx::query("DELETE FROM execution_traces WHERE created_at < datetime('now', ?)")
                .bind(format!("-{retention_days} days"))
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected())
    }
}
