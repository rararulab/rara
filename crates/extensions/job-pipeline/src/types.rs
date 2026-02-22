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

//! Domain types for pipeline runs and events.

use jiff::Timestamp;
use rara_domain_shared::convert::{chrono_opt_to_timestamp, chrono_to_timestamp};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, FromRepr};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// PipelineRunStatus
// ---------------------------------------------------------------------------

/// Status of a pipeline run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, FromRepr)]
#[repr(u8)]
pub enum PipelineRunStatus {
    /// Pipeline is currently executing.
    Running = 0,
    /// Pipeline finished successfully.
    Completed = 1,
    /// Pipeline failed with an error.
    Failed = 2,
    /// Pipeline was cancelled by the user.
    Cancelled = 3,
}

// ---------------------------------------------------------------------------
// PipelineRun
// ---------------------------------------------------------------------------

/// A single pipeline execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: Uuid,
    pub status: PipelineRunStatus,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub jobs_found: i32,
    pub jobs_scored: i32,
    pub jobs_applied: i32,
    pub jobs_notified: i32,
    pub summary: Option<String>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// PipelineEvent
// ---------------------------------------------------------------------------

/// A single event emitted during a pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineEvent {
    pub id: i64,
    pub run_id: Uuid,
    pub seq: i32,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: Timestamp,
}

// ---------------------------------------------------------------------------
// DB row models (sqlx::FromRow)
// ---------------------------------------------------------------------------

/// Database row representation of a pipeline run.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct PipelineRunRow {
    pub id: Uuid,
    pub status: i16,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub jobs_found: i32,
    pub jobs_scored: i32,
    pub jobs_applied: i32,
    pub jobs_notified: i32,
    pub summary: Option<String>,
    pub error: Option<String>,
}

/// Database row representation of a pipeline event.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct PipelineEventRow {
    pub id: i64,
    pub run_id: Uuid,
    pub seq: i32,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// DB -> Domain conversions
// ---------------------------------------------------------------------------

impl From<PipelineRunRow> for PipelineRun {
    fn from(row: PipelineRunRow) -> Self {
        Self {
            id: row.id,
            status: PipelineRunStatus::from_repr(row.status as u8)
                .unwrap_or(PipelineRunStatus::Failed),
            started_at: chrono_to_timestamp(row.started_at),
            finished_at: chrono_opt_to_timestamp(row.finished_at),
            jobs_found: row.jobs_found,
            jobs_scored: row.jobs_scored,
            jobs_applied: row.jobs_applied,
            jobs_notified: row.jobs_notified,
            summary: row.summary,
            error: row.error,
        }
    }
}

impl From<PipelineEventRow> for PipelineEvent {
    fn from(row: PipelineEventRow) -> Self {
        Self {
            id: row.id,
            run_id: row.run_id,
            seq: row.seq,
            event_type: row.event_type,
            payload: row.payload,
            created_at: chrono_to_timestamp(row.created_at),
        }
    }
}

// ---------------------------------------------------------------------------
// DiscoveredJobAction
// ---------------------------------------------------------------------------

/// What happened to a discovered job during the pipeline run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, FromRepr)]
#[repr(u8)]
pub enum DiscoveredJobAction {
    Discovered = 0,
    Notified = 1,
    Applied = 2,
    Skipped = 3,
}

// ---------------------------------------------------------------------------
// DiscoveredJob
// ---------------------------------------------------------------------------

/// A job discovered during a pipeline run (slim, FK-only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredJob {
    pub id: Uuid,
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub score: Option<i32>,
    pub action: DiscoveredJobAction,
    pub created_at: Timestamp,
}

/// Database row representation of a discovered job.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct DiscoveredJobRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub score: Option<i32>,
    pub action: i16,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<DiscoveredJobRow> for DiscoveredJob {
    fn from(row: DiscoveredJobRow) -> Self {
        Self {
            id: row.id,
            run_id: row.run_id,
            job_id: row.job_id,
            score: row.score,
            action: DiscoveredJobAction::from_repr(row.action as u8)
                .unwrap_or(DiscoveredJobAction::Discovered),
            created_at: chrono_to_timestamp(row.created_at),
        }
    }
}

// ---------------------------------------------------------------------------
// DiscoveredJobWithDetails — JOIN with job table
// ---------------------------------------------------------------------------

/// A discovered job enriched with details from the `job` table (via JOIN).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredJobWithDetails {
    pub id: Uuid,
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub score: Option<i32>,
    pub action: DiscoveredJobAction,
    pub created_at: Timestamp,
    // Job details from JOIN
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub posted_at: Option<Timestamp>,
}

/// Database row for the JOIN query between `pipeline_discovered_jobs` and `job`.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct DiscoveredJobWithDetailsRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub score: Option<i32>,
    pub action: i16,
    pub created_at: chrono::DateTime<chrono::Utc>,
    // from job table
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub posted_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<DiscoveredJobWithDetailsRow> for DiscoveredJobWithDetails {
    fn from(row: DiscoveredJobWithDetailsRow) -> Self {
        Self {
            id: row.id,
            run_id: row.run_id,
            job_id: row.job_id,
            score: row.score,
            action: DiscoveredJobAction::from_repr(row.action as u8)
                .unwrap_or(DiscoveredJobAction::Discovered),
            created_at: chrono_to_timestamp(row.created_at),
            title: row.title,
            company: row.company,
            location: row.location,
            url: row.url,
            description: row.description,
            posted_at: chrono_opt_to_timestamp(row.posted_at),
        }
    }
}

// ---------------------------------------------------------------------------
// DiscoveredJobsStats — aggregated stats across all runs
// ---------------------------------------------------------------------------

/// Aggregated statistics for discovered jobs across all pipeline runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredJobsStats {
    pub total: i64,
    pub by_action: DiscoveredJobsActionCounts,
    pub scored_count: i64,
    pub avg_score: Option<f64>,
}

/// Breakdown of discovered jobs by action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredJobsActionCounts {
    pub discovered: i64,
    pub notified: i64,
    pub applied: i64,
    pub skipped: i64,
}

// ---------------------------------------------------------------------------
// PipelineStreamEvent — SSE streaming events
// ---------------------------------------------------------------------------

/// Events emitted during a pipeline run, streamed to SSE clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineStreamEvent {
    /// Pipeline run has started with the given run ID.
    Started { run_id: Uuid },
    /// Agent loop entered a new iteration.
    Iteration { index: usize },
    /// LLM is processing (show a "thinking" indicator).
    Thinking,
    /// LLM finished thinking.
    ThinkingDone,
    /// A tool call has started.
    ToolCallStart {
        id: String,
        name: String,
        arguments: Option<serde_json::Value>,
    },
    /// A tool call has finished.
    ToolCallEnd {
        id: String,
        name: String,
        success: bool,
        error: Option<String>,
        result: Option<serde_json::Value>,
    },
    /// Incremental text content from the LLM.
    TextDelta { text: String },
    /// Pipeline run completed successfully.
    Done {
        summary: String,
        iterations: usize,
        tool_calls: usize,
    },
    /// Pipeline run failed with an error.
    Error { message: String },
}

impl PipelineStreamEvent {
    /// Returns the SSE event type name for this event variant.
    pub fn event_type_name(&self) -> &'static str {
        match self {
            Self::Started { .. } => "started",
            Self::Iteration { .. } => "iteration",
            Self::Thinking => "thinking",
            Self::ThinkingDone => "thinking_done",
            Self::ToolCallStart { .. } => "tool_call_start",
            Self::ToolCallEnd { .. } => "tool_call_end",
            Self::TextDelta { .. } => "text_delta",
            Self::Done { .. } => "done",
            Self::Error { .. } => "error",
        }
    }
}
