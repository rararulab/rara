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

//! [`JobSourceService`] orchestrates job discovery via [`JobSpyDriver`]
//! and applies deduplication.

use std::collections::HashSet;

use crate::{
    dedup::{self, FuzzyKey, SourceKey},
    err::SourceError,
    jobspy::JobSpyDriver,
    types::{DiscoveryCriteria, NormalizedJob},
};

/// Orchestrator that drives job discovery and deduplication.
#[derive(Debug)]
pub struct JobSourceService {
    driver: JobSpyDriver,
}

impl JobSourceService {
    /// Create a new service backed by the given [`JobSpyDriver`].
    #[must_use]
    pub fn new(driver: JobSpyDriver) -> Self { Self { driver } }

    /// Discover jobs matching the criteria, returning deduplicated
    /// results.
    ///
    /// `existing_source_keys` and `existing_fuzzy_keys` represent
    /// jobs that already exist in the database so they can be
    /// excluded during deduplication.
    pub fn discover(
        &self,
        criteria: &DiscoveryCriteria,
        existing_source_keys: &HashSet<SourceKey>,
        existing_fuzzy_keys: &HashSet<FuzzyKey>,
    ) -> DiscoveryResult {
        let raw_jobs = match self.driver.fetch_jobs(criteria) {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::error!(error = %e, "JobSpy driver failed during discovery");
                return DiscoveryResult {
                    jobs:  vec![],
                    error: Some(e),
                };
            }
        };

        tracing::info!(count = raw_jobs.len(), "JobSpy returned raw jobs");

        // Normalize raw → NormalizedJob via TryFrom.
        let mut normalized = Vec::with_capacity(raw_jobs.len());
        for raw in raw_jobs {
            match NormalizedJob::try_from(raw) {
                Ok(job) => normalized.push(job),
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping job that failed normalization");
                }
            }
        }
        tracing::info!(
            normalized_count = normalized.len(),
            "normalization complete"
        );

        let deduped = dedup::deduplicate(normalized, existing_source_keys, existing_fuzzy_keys);
        tracing::info!(
            deduped_count = deduped.len(),
            "deduplication complete"
        );

        DiscoveryResult {
            jobs:  deduped,
            error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// DiscoveryResult
// ---------------------------------------------------------------------------

/// The outcome of a discovery run.
#[derive(Debug)]
pub struct DiscoveryResult {
    /// Successfully normalized and deduplicated jobs.
    pub jobs:  Vec<NormalizedJob>,
    /// If the driver encountered an unrecoverable error, it is
    /// captured here.
    pub error: Option<SourceError>,
}
