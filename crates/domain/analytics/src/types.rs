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

use jiff::{Timestamp, civil::Date};
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

/// A point-in-time statistics snapshot for a given period (domain
/// representation).
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

// ---------------------------------------------------------------------------
// DB model conversions
// ---------------------------------------------------------------------------

use job_domain_shared::convert::{
    chrono_to_timestamp, civil_to_naive_date, naive_date_to_civil, timestamp_to_chrono, u8_from_i16,
};
use job_model::metrics::MetricsSnapshot as StoreMetricsSnapshot;

fn period_from_i16(value: i16) -> MetricsPeriod {
    let repr = u8_from_i16(value, "metrics.period");
    MetricsPeriod::from_repr(repr).unwrap_or_else(|| panic!("invalid metrics.period: {value}"))
}

/// Store `MetricsSnapshot` -> Domain `MetricsSnapshot`.
impl From<StoreMetricsSnapshot> for MetricsSnapshot {
    fn from(r: StoreMetricsSnapshot) -> Self {
        Self {
            id:                   r.id,
            period:               period_from_i16(r.period),
            snapshot_date:        naive_date_to_civil(r.snapshot_date),
            jobs_discovered:      r.jobs_discovered,
            applications_sent:    r.applications_sent,
            interviews_scheduled: r.interviews_scheduled,
            offers_received:      r.offers_received,
            rejections:           r.rejections,
            ai_runs_count:        r.ai_runs_count,
            ai_total_cost_cents:  r.ai_total_cost_cents,
            extra:                r.extra,
            trace_id:             r.trace_id,
            created_at:           chrono_to_timestamp(r.created_at),
        }
    }
}

/// Domain `MetricsSnapshot` -> Store `MetricsSnapshot`.
impl From<MetricsSnapshot> for StoreMetricsSnapshot {
    fn from(r: MetricsSnapshot) -> Self {
        Self {
            id:                   r.id,
            period:               r.period as u8 as i16,
            snapshot_date:        civil_to_naive_date(r.snapshot_date),
            jobs_discovered:      r.jobs_discovered,
            applications_sent:    r.applications_sent,
            interviews_scheduled: r.interviews_scheduled,
            offers_received:      r.offers_received,
            rejections:           r.rejections,
            ai_runs_count:        r.ai_runs_count,
            ai_total_cost_cents:  r.ai_total_cost_cents,
            extra:                r.extra,
            trace_id:             r.trace_id,
            created_at:           timestamp_to_chrono(r.created_at),
        }
    }
}
