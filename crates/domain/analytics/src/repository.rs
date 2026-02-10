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

//! Repository trait for analytics persistence.

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    error::AnalyticsError,
    types::{MetricsPeriod, MetricsSnapshot, SnapshotFilter},
};

/// Abstract repository for metrics snapshot persistence.
#[async_trait]
pub trait AnalyticsRepository: Send + Sync {
    /// Persist a new snapshot. Returns the saved snapshot.
    async fn save_snapshot(
        &self,
        snapshot: &MetricsSnapshot,
    ) -> Result<MetricsSnapshot, AnalyticsError>;

    /// Find a snapshot by its ID.
    async fn find_by_id(&self, id: Uuid) -> Result<Option<MetricsSnapshot>, AnalyticsError>;

    /// Get the most recent snapshot for a given period.
    async fn get_latest(
        &self,
        period: MetricsPeriod,
    ) -> Result<Option<MetricsSnapshot>, AnalyticsError>;

    /// List snapshots matching the given filter.
    async fn list_snapshots(
        &self,
        filter: &SnapshotFilter,
    ) -> Result<Vec<MetricsSnapshot>, AnalyticsError>;

    /// Hard-delete a snapshot by ID.
    async fn delete_snapshot(&self, id: Uuid) -> Result<(), AnalyticsError>;
}
