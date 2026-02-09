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

//! Domain types for metrics analytics.

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, FromRepr};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Metrics period
// ---------------------------------------------------------------------------

/// Aggregation period for a metrics snapshot.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, FromRepr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MetricsPeriod {
    Daily = 0,
    Weekly = 1,
    Monthly = 2,
}

// ---------------------------------------------------------------------------
// Domain model
// ---------------------------------------------------------------------------

/// A point-in-time statistics snapshot for a given period (domain representation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub id:                   Uuid,
    pub period:               MetricsPeriod,
    pub snapshot_date:        Date,
    pub jobs_discovered:      i32,
    pub applications_sent:    i32,
    pub interviews_scheduled: i32,
    pub offers_received:      i32,
    pub rejections:           i32,
    pub ai_runs_count:        i32,
    pub ai_total_cost_cents:  i32,
    pub extra:                Option<serde_json::Value>,
    pub trace_id:             Option<String>,
    pub created_at:           Timestamp,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Parameters for creating a new metrics snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSnapshotRequest {
    pub period:               MetricsPeriod,
    pub snapshot_date:        Date,
    pub jobs_discovered:      i32,
    pub applications_sent:    i32,
    pub interviews_scheduled: i32,
    pub offers_received:      i32,
    pub rejections:           i32,
    pub ai_runs_count:        i32,
    pub ai_total_cost_cents:  i32,
    pub extra:                Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// Criteria for listing/searching metrics snapshots.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotFilter {
    pub period:    Option<MetricsPeriod>,
    pub date_from: Option<Date>,
    pub date_to:   Option<Date>,
    pub limit:     Option<i64>,
}
