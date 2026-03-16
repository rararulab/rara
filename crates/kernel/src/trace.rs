//! Execution trace storage.
//!
//! Each agent turn produces an [`ExecutionTrace`] summarizing duration, token
//! usage, tool calls, and plan steps. Traces are persisted to SQLite so that
//! any channel adapter can retrieve them later (e.g. Telegram inline buttons).

use sqlx::SqlitePool;

/// Summary of a single agent turn execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionTrace {
    pub duration_secs: u64,
    pub iterations: usize,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub thinking_ms: u64,
    /// Truncated reasoning text (first ~500 chars).
    pub thinking_preview: String,
    /// Plan steps with status.
    pub plan_steps: Vec<String>,
    /// Tool execution records.
    pub tools: Vec<ToolTraceEntry>,
    /// Rara internal message ID for end-to-end correlation.
    pub rara_message_id: String,
}

/// Record of a single tool invocation within a turn.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolTraceEntry {
    pub name: String,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    pub success: bool,
    pub summary: String,
    pub error: Option<String>,
}

/// Persistent store for execution traces backed by SQLite.
#[derive(Debug, Clone)]
pub struct TraceService {
    pool: SqlitePool,
}

impl TraceService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Save an execution trace. Returns the generated ULID.
    pub async fn save(
        &self,
        session_id: &str,
        trace: &ExecutionTrace,
    ) -> Result<String, sqlx::Error> {
        let id = ulid::Ulid::new().to_string();
        let trace_data = serde_json::to_string(trace)
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

        sqlx::query(
            "INSERT INTO execution_traces (id, session_id, trace_data) VALUES (?, ?, ?)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(&trace_data)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Retrieve an execution trace by ID.
    pub async fn get(&self, id: &str) -> Result<Option<ExecutionTrace>, sqlx::Error> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT trace_data FROM execution_traces WHERE id = ?",
        )
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
}
