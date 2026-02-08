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

//! Metrics snapshot entity: periodic statistics aggregation.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Aggregation period for a metrics snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "metrics_period", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum MetricsPeriod {
    Daily,
    Weekly,
    Monthly,
}

impl std::fmt::Display for MetricsPeriod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Daily => write!(f, "daily"),
            Self::Weekly => write!(f, "weekly"),
            Self::Monthly => write!(f, "monthly"),
        }
    }
}

/// A point-in-time statistics snapshot for a given period.
///
/// The combination of `period` and `snapshot_date` is unique, ensuring
/// exactly one snapshot per period per date.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MetricsSnapshot {
    pub id: Uuid,
    pub period: MetricsPeriod,
    pub snapshot_date: NaiveDate,
    pub jobs_discovered: i32,
    pub applications_sent: i32,
    pub interviews_scheduled: i32,
    pub offers_received: i32,
    pub rejections: i32,
    pub ai_runs_count: i32,
    pub ai_total_cost_cents: i32,
    /// Additional metrics as flexible JSON.
    pub extra: Option<serde_json::Value>,
    pub trace_id: Option<String>,
    pub created_at: DateTime<Utc>,
}
