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

//! Store models for the analytics/metrics domain.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A point-in-time statistics snapshot for a given period (DB row).
///
/// The combination of `period` and `snapshot_date` is unique, ensuring
/// exactly one snapshot per period per date.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MetricsSnapshot {
    pub id:                   Uuid,
    pub period:               i16,
    pub snapshot_date:        NaiveDate,
    pub jobs_discovered:      i32,
    pub applications_sent:    i32,
    pub interviews_scheduled: i32,
    pub offers_received:      i32,
    pub rejections:           i32,
    pub ai_runs_count:        i32,
    pub ai_total_cost_cents:  i32,
    pub extra:                Option<serde_json::Value>,
    pub trace_id:             Option<String>,
    pub created_at:           DateTime<Utc>,
}
