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

//! The [`JobSourceDriver`] trait that every concrete driver must
//! implement.
//!
//! A driver is responsible for:
//! 1. Fetching raw job listings from an external platform.
//! 2. Normalizing the raw data into a canonical [`NormalizedJob`]
//!    record.

use crate::types::{DiscoveryCriteria, NormalizedJob, RawJob, SourceError};

/// Trait that every job source driver must implement.
///
/// Drivers are [`Send`] + [`Sync`] so they can be shared across async
/// tasks and stored in an `Arc`.
#[async_trait::async_trait]
pub trait JobSourceDriver: Send + Sync {
    /// Human-readable name of this source (e.g. "linkedin", "manual").
    fn source_name(&self) -> &str;

    /// Fetch raw job listings that match the given criteria.
    ///
    /// Implementations should translate the high-level
    /// [`DiscoveryCriteria`] into whatever query the backing
    /// platform supports and return the results as [`RawJob`]s.
    async fn fetch_jobs(
        &self,
        query: &DiscoveryCriteria,
    ) -> Result<Vec<RawJob>, SourceError>;

    /// Normalize a single [`RawJob`] into a [`NormalizedJob`].
    ///
    /// This step validates required fields and applies any
    /// source-specific cleaning logic.
    async fn normalize(&self, raw: RawJob) -> Result<NormalizedJob, SourceError>;
}
