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

//! [`JobSourceService`] orchestrates multiple [`JobSourceDriver`]s
//! and applies deduplication.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::{
    dedup::{self, FuzzyKey, SourceKey},
    driver::JobSourceDriver,
    types::{DiscoveryCriteria, NormalizedJob, SourceError},
};

/// Orchestrator that manages a registry of [`JobSourceDriver`]s and
/// provides high-level discovery operations.
pub struct JobSourceService {
    drivers: HashMap<String, Arc<dyn JobSourceDriver>>,
}

impl JobSourceService {
    /// Create a new service with no drivers registered.
    #[must_use]
    pub fn new() -> Self {
        Self {
            drivers: HashMap::new(),
        }
    }

    /// Register a driver.
    ///
    /// If a driver with the same `source_name` is already registered
    /// it will be replaced.
    pub fn register(&mut self, driver: Arc<dyn JobSourceDriver>) {
        let name = driver.source_name().to_owned();
        tracing::info!(source_name = %name, "Registered job source driver");
        self.drivers.insert(name, driver);
    }

    /// Return the names of all registered drivers.
    #[must_use]
    pub fn registered_sources(&self) -> Vec<&str> {
        self.drivers.keys().map(String::as_str).collect()
    }

    /// Run *all* registered drivers and return deduplicated results.
    ///
    /// `existing_source_keys` and `existing_fuzzy_keys` represent
    /// jobs that already exist in the database so they can be
    /// excluded during deduplication.
    pub async fn discover_all(
        &self,
        criteria: &DiscoveryCriteria,
        existing_source_keys: &HashSet<SourceKey>,
        existing_fuzzy_keys: &HashSet<FuzzyKey>,
    ) -> Vec<DiscoveryResult> {
        let mut all_normalized: Vec<NormalizedJob> = Vec::new();
        let mut errors: Vec<DiscoveryResult> = Vec::new();

        for (name, driver) in &self.drivers {
            match self.run_driver(driver.as_ref(), criteria).await {
                Ok(jobs) => {
                    tracing::info!(
                        source_name = %name,
                        count = jobs.len(),
                        "Driver returned normalized jobs"
                    );
                    all_normalized.extend(jobs);
                }
                Err(e) => {
                    tracing::error!(
                        source_name = %name,
                        error = %e,
                        "Driver failed during discovery"
                    );
                    errors.push(DiscoveryResult {
                        source_name: name.clone(),
                        jobs:        vec![],
                        error:       Some(e),
                    });
                }
            }
        }

        let deduped = dedup::deduplicate(all_normalized, existing_source_keys, existing_fuzzy_keys);

        // Group the deduplicated jobs back by source name.
        let mut by_source: HashMap<String, Vec<NormalizedJob>> = HashMap::new();
        for job in deduped {
            by_source
                .entry(job.source_name.clone())
                .or_default()
                .push(job);
        }

        let mut results: Vec<DiscoveryResult> = by_source
            .into_iter()
            .map(|(source_name, jobs)| DiscoveryResult {
                source_name,
                jobs,
                error: None,
            })
            .collect();

        results.extend(errors);
        results
    }

    /// Run a single driver identified by `source_name`.
    ///
    /// Returns `None` if no driver with the given name is registered.
    pub async fn discover_from(
        &self,
        source_name: &str,
        criteria: &DiscoveryCriteria,
        existing_source_keys: &HashSet<SourceKey>,
        existing_fuzzy_keys: &HashSet<FuzzyKey>,
    ) -> Option<DiscoveryResult> {
        let driver = self.drivers.get(source_name)?;

        let result = match self.run_driver(driver.as_ref(), criteria).await {
            Ok(jobs) => {
                let deduped = dedup::deduplicate(jobs, existing_source_keys, existing_fuzzy_keys);
                DiscoveryResult {
                    source_name: source_name.to_owned(),
                    jobs:        deduped,
                    error:       None,
                }
            }
            Err(e) => DiscoveryResult {
                source_name: source_name.to_owned(),
                jobs:        vec![],
                error:       Some(e),
            },
        };

        Some(result)
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Fetch and normalize jobs from a single driver.
    async fn run_driver(
        &self,
        driver: &dyn JobSourceDriver,
        criteria: &DiscoveryCriteria,
    ) -> Result<Vec<NormalizedJob>, SourceError> {
        let raw_jobs = driver.fetch_jobs(criteria).await?;

        let mut normalized = Vec::with_capacity(raw_jobs.len());
        for raw in raw_jobs {
            match driver.normalize(raw).await {
                Ok(job) => normalized.push(job),
                Err(e) => {
                    // Log but do not abort the entire batch.
                    tracing::warn!(
                        source_name = %driver.source_name(),
                        error = %e,
                        "Skipping job that failed normalization"
                    );
                }
            }
        }

        Ok(normalized)
    }
}

impl Default for JobSourceService {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Debug for JobSourceService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobSourceService")
            .field("drivers", &self.drivers.keys().collect::<Vec<_>>())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// DiscoveryResult
// ---------------------------------------------------------------------------

/// The outcome of running a single driver during discovery.
#[derive(Debug)]
pub struct DiscoveryResult {
    /// Which source produced (or failed to produce) these results.
    pub source_name: String,
    /// Successfully normalized and deduplicated jobs.
    pub jobs:        Vec<NormalizedJob>,
    /// If the driver encountered an unrecoverable error, it is
    /// captured here.
    pub error:       Option<SourceError>,
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use super::*;
    use crate::{linkedin::LinkedInSource, manual::ManualSource};

    #[tokio::test]
    async fn service_registers_and_lists_sources() {
        let mut svc = JobSourceService::new();
        svc.register(Arc::new(ManualSource::new()));
        svc.register(Arc::new(LinkedInSource::new()));

        let mut sources = svc.registered_sources();
        sources.sort_unstable();
        assert_eq!(sources, vec!["linkedin", "manual"]);
    }

    #[tokio::test]
    async fn discover_all_runs_every_driver() {
        let mut svc = JobSourceService::new();
        svc.register(Arc::new(ManualSource::new()));
        svc.register(Arc::new(LinkedInSource::new()));

        let criteria = DiscoveryCriteria::default();
        let results = svc
            .discover_all(&criteria, &HashSet::new(), &HashSet::new())
            .await;

        // Both drivers return empty lists, so we expect no errors
        // and no jobs.
        for r in &results {
            assert!(r.error.is_none());
            assert!(r.jobs.is_empty());
        }
    }

    #[tokio::test]
    async fn discover_from_unknown_returns_none() {
        let svc = JobSourceService::new();
        let criteria = DiscoveryCriteria::default();
        let result = svc
            .discover_from("nonexistent", &criteria, &HashSet::new(), &HashSet::new())
            .await;
        assert!(result.is_none());
    }
}
