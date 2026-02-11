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

//! Analytics service — snapshot management and derived statistics.

use std::sync::Arc;

use jiff::Timestamp;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::{
    error::AnalyticsError,
    repository::AnalyticsRepository,
    types::{CreateSnapshotRequest, MetricsPeriod, MetricsSnapshot, SnapshotFilter},
};

/// Application service for analytics / metrics snapshots.
#[derive(Clone)]
pub struct AnalyticsService {
    repo: Arc<dyn AnalyticsRepository>,
}

impl AnalyticsService {
    /// Create a new analytics service backed by the given repository.
    pub fn new(repo: Arc<dyn AnalyticsRepository>) -> Self { Self { repo } }

    /// Create a new metrics snapshot.
    #[instrument(skip(self, req))]
    pub async fn create_snapshot(
        &self,
        req: CreateSnapshotRequest,
    ) -> Result<MetricsSnapshot, AnalyticsError> {
        let snapshot = MetricsSnapshot {
            id:                   Uuid::new_v4(),
            period:               req.period,
            snapshot_date:        req.snapshot_date,
            jobs_discovered:      req.jobs_discovered,
            applications_sent:    req.applications_sent,
            interviews_scheduled: req.interviews_scheduled,
            offers_received:      req.offers_received,
            rejections:           req.rejections,
            ai_runs_count:        req.ai_runs_count,
            ai_total_cost_cents:  req.ai_total_cost_cents,
            extra:                req.extra,
            trace_id:             None,
            created_at:           Timestamp::now(),
        };

        let saved = self.repo.save_snapshot(&snapshot).await?;
        info!(id = %saved.id, period = %saved.period, "metrics snapshot created");
        Ok(saved)
    }

    /// Get a snapshot by ID.
    #[instrument(skip(self))]
    pub async fn get_snapshot(&self, id: Uuid) -> Result<MetricsSnapshot, AnalyticsError> {
        self.repo
            .find_by_id(id)
            .await?
            .ok_or(AnalyticsError::NotFound { id })
    }

    /// Get the most recent snapshot for a period.
    #[instrument(skip(self))]
    pub async fn get_latest(
        &self,
        period: MetricsPeriod,
    ) -> Result<Option<MetricsSnapshot>, AnalyticsError> {
        self.repo.get_latest(period).await
    }

    /// List snapshots matching a filter.
    #[instrument(skip(self, filter))]
    pub async fn list_snapshots(
        &self,
        filter: &SnapshotFilter,
    ) -> Result<Vec<MetricsSnapshot>, AnalyticsError> {
        self.repo.list_snapshots(filter).await
    }

    /// Delete a snapshot.
    #[instrument(skip(self))]
    pub async fn delete_snapshot(&self, id: Uuid) -> Result<(), AnalyticsError> {
        self.repo.delete_snapshot(id).await
    }

    // -------------------------------------------------------------------
    // Derived statistics (computed from a snapshot)
    // -------------------------------------------------------------------

    /// Compute offer rate (offers / applications) for a snapshot.
    pub fn offer_rate(snapshot: &MetricsSnapshot) -> Option<f64> {
        if snapshot.applications_sent == 0 {
            return None;
        }
        Some(f64::from(snapshot.offers_received) / f64::from(snapshot.applications_sent))
    }

    /// Compute interview rate (interviews / applications) for a snapshot.
    pub fn interview_rate(snapshot: &MetricsSnapshot) -> Option<f64> {
        if snapshot.applications_sent == 0 {
            return None;
        }
        Some(f64::from(snapshot.interviews_scheduled) / f64::from(snapshot.applications_sent))
    }

    /// Compute rejection rate (rejections / applications) for a snapshot.
    pub fn rejection_rate(snapshot: &MetricsSnapshot) -> Option<f64> {
        if snapshot.applications_sent == 0 {
            return None;
        }
        Some(f64::from(snapshot.rejections) / f64::from(snapshot.applications_sent))
    }

    /// Average AI cost per run in cents.
    pub fn avg_ai_cost_per_run(snapshot: &MetricsSnapshot) -> Option<f64> {
        if snapshot.ai_runs_count == 0 {
            return None;
        }
        Some(f64::from(snapshot.ai_total_cost_cents) / f64::from(snapshot.ai_runs_count))
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::Date;

    use super::*;

    fn sample_snapshot() -> MetricsSnapshot {
        MetricsSnapshot {
            id:                   Uuid::new_v4(),
            period:               MetricsPeriod::Daily,
            snapshot_date:        Date::new(2026, 1, 15).unwrap(),
            jobs_discovered:      50,
            applications_sent:    20,
            interviews_scheduled: 5,
            offers_received:      2,
            rejections:           10,
            ai_runs_count:        8,
            ai_total_cost_cents:  400,
            extra:                None,
            trace_id:             None,
            created_at:           Timestamp::now(),
        }
    }

    #[test]
    fn offer_rate_calculation() {
        let s = sample_snapshot();
        let rate = AnalyticsService::offer_rate(&s).unwrap();
        assert!((rate - 0.1).abs() < 0.001); // 2/20
    }

    #[test]
    fn interview_rate_calculation() {
        let s = sample_snapshot();
        let rate = AnalyticsService::interview_rate(&s).unwrap();
        assert!((rate - 0.25).abs() < 0.001); // 5/20
    }

    #[test]
    fn rejection_rate_calculation() {
        let s = sample_snapshot();
        let rate = AnalyticsService::rejection_rate(&s).unwrap();
        assert!((rate - 0.5).abs() < 0.001); // 10/20
    }

    #[test]
    fn rates_return_none_for_zero_applications() {
        let mut s = sample_snapshot();
        s.applications_sent = 0;
        assert!(AnalyticsService::offer_rate(&s).is_none());
        assert!(AnalyticsService::interview_rate(&s).is_none());
        assert!(AnalyticsService::rejection_rate(&s).is_none());
    }

    #[test]
    fn avg_ai_cost_calculation() {
        let s = sample_snapshot();
        let avg = AnalyticsService::avg_ai_cost_per_run(&s).unwrap();
        assert!((avg - 50.0).abs() < 0.001); // 400/8
    }

    #[test]
    fn avg_ai_cost_returns_none_for_zero_runs() {
        let mut s = sample_snapshot();
        s.ai_runs_count = 0;
        assert!(AnalyticsService::avg_ai_cost_per_run(&s).is_none());
    }
}
