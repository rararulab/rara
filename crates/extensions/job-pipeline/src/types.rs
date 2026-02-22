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
