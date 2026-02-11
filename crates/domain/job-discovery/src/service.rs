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

use std::collections::{BTreeMap, HashSet};

use crate::{
    dedup::{self, FuzzyKey, SourceKey},
    err::SourceError,
    jobspy::JobSpyDriver,
    types::{DiscoveryCriteria, NormalizedJob, RawJob},
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
        log_description_coverage_by_source(&raw_jobs);

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

        let mut deduped = dedup::deduplicate(normalized, existing_source_keys, existing_fuzzy_keys);

        // Sort by posted_at descending (newest first, None last).
        deduped.sort_by(|a, b| b.posted_at.cmp(&a.posted_at));

        tracing::info!(
            deduped_count = deduped.len(),
            "deduplication and sorting complete"
        );

        DiscoveryResult {
            jobs:  deduped,
            error: None,
        }
    }
}

fn log_description_coverage_by_source(raw_jobs: &[RawJob]) {
    for (source, stat) in description_coverage_by_source(raw_jobs) {
        tracing::info!(
            source,
            total = stat.total,
            with_description = stat.with_description,
            without_description = stat.without_description,
            coverage_pct = stat.coverage_pct(),
            "job description coverage by source"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DescriptionCoverage {
    total:               u32,
    with_description:    u32,
    without_description: u32,
}

impl DescriptionCoverage {
    fn coverage_pct(self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.with_description as f64 * 100.0 / self.total as f64
        }
    }
}

fn description_coverage_by_source(raw_jobs: &[RawJob]) -> BTreeMap<String, DescriptionCoverage> {
    let mut by_source: BTreeMap<String, DescriptionCoverage> = BTreeMap::new();
    for job in raw_jobs {
        let stat = by_source
            .entry(job.source_name.clone())
            .or_insert(DescriptionCoverage {
                total:               0,
                with_description:    0,
                without_description: 0,
            });
        stat.total += 1;
        if has_description(job.description.as_deref()) {
            stat.with_description += 1;
        } else {
            stat.without_description += 1;
        }
    }
    by_source
}

fn has_description(description: Option<&str>) -> bool {
    description.is_some_and(|text| !text.trim().is_empty())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_raw(source_name: &str, description: Option<&str>) -> RawJob {
        RawJob {
            source_job_id:   format!("{source_name}-id"),
            source_name:     source_name.to_owned(),
            title:           Some("title".to_owned()),
            company:         Some("company".to_owned()),
            location:        None,
            description:     description.map(ToOwned::to_owned),
            url:             None,
            salary_min:      None,
            salary_max:      None,
            salary_currency: None,
            tags:            vec![],
            raw_data:        None,
            posted_at:       None,
        }
    }

    #[test]
    fn description_coverage_by_source_counts_per_source() {
        let raw_jobs = vec![
            make_raw("indeed", Some("job details")),
            make_raw("indeed", Some("  ")),
            make_raw("linkedin", None),
            make_raw("linkedin", Some("desc")),
        ];

        let stats = description_coverage_by_source(&raw_jobs);
        let indeed = stats.get("indeed").unwrap();
        let linkedin = stats.get("linkedin").unwrap();

        assert_eq!(indeed.total, 2);
        assert_eq!(indeed.with_description, 1);
        assert_eq!(indeed.without_description, 1);
        assert_eq!(indeed.coverage_pct(), 50.0);

        assert_eq!(linkedin.total, 2);
        assert_eq!(linkedin.with_description, 1);
        assert_eq!(linkedin.without_description, 1);
        assert_eq!(linkedin.coverage_pct(), 50.0);
    }

    #[test]
    fn has_description_treats_blank_text_as_missing() {
        assert!(has_description(Some("x")));
        assert!(!has_description(Some("   ")));
        assert!(!has_description(None));
    }
}
