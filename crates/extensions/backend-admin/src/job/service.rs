// Copyright 2025 Rararulab
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

//! Unified job service -- discovery and AI-powered JD parsing.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use super::{
    dedup::{self, FuzzyKey, SourceKey},
    error::SourceError,
    japandev::JapanDevDriver,
    jobspy::JobSpyDriver,
    repository::JobRepository,
    types::{DiscoveryCriteria, NormalizedJob, RawJob},
};

// ===========================================================================
// JobService
// ===========================================================================

/// Unified service for job discovery.
#[derive(Clone)]
pub struct JobService {
    driver:   Arc<JobSpyDriver>,
    japandev: Option<Arc<JapanDevDriver>>,
    job_repo: Arc<dyn JobRepository>,
}

impl JobService {
    /// Create a new unified job service.
    pub fn new(
        driver: JobSpyDriver,
        japandev: Option<JapanDevDriver>,
        job_repo: Arc<dyn JobRepository>,
    ) -> Self {
        Self {
            driver: Arc::new(driver),
            japandev: japandev.map(Arc::new),
            job_repo,
        }
    }

    // -- Accessors ----------------------------------------------------------

    /// Access the job repository directly.
    pub fn job_repo(&self) -> &Arc<dyn JobRepository> { &self.job_repo }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Discover jobs matching the criteria, returning deduplicated results.
    ///
    /// `existing_source_keys` and `existing_fuzzy_keys` represent jobs that
    /// already exist in the database so they can be excluded during
    /// deduplication.
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

        // Normalize raw -> NormalizedJob via TryFrom.
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

    /// Discover jobs from all configured sources (JobSpy + JapanDev),
    /// running them concurrently and merging the results.
    ///
    /// This is the async counterpart of [`Self::discover`] and is the
    /// preferred entry point for callers in async contexts.
    pub async fn discover_all(
        &self,
        criteria: &DiscoveryCriteria,
        existing_source_keys: &HashSet<SourceKey>,
        existing_fuzzy_keys: &HashSet<FuzzyKey>,
    ) -> DiscoveryResult {
        let mut raw_jobs = Vec::new();
        let mut errors: Vec<SourceError> = Vec::new();

        // Determine which drivers to run based on `criteria.sites`.
        // If sites is empty, run all drivers. Otherwise, run only those
        // explicitly listed. "japandev" enables the JapanDev driver;
        // any other site name is forwarded to JobSpy.
        let sites = &criteria.sites;
        let run_all = sites.is_empty();
        let run_japandev = run_all || sites.iter().any(|s| s.eq_ignore_ascii_case("japandev"));
        let jobspy_sites: Vec<String> = sites
            .iter()
            .filter(|s| !s.eq_ignore_ascii_case("japandev"))
            .cloned()
            .collect();
        let run_jobspy = run_all || !jobspy_sites.is_empty();

        // 1. JobSpy (sync / blocking -- run in spawn_blocking).
        if run_jobspy {
            let driver = self.driver.clone();
            let mut criteria_clone = criteria.clone();
            criteria_clone.sites = jobspy_sites;
            let jobspy_result =
                tokio::task::spawn_blocking(move || driver.fetch_jobs(&criteria_clone)).await;

            match jobspy_result {
                Ok(Ok(jobs)) => {
                    tracing::info!(count = jobs.len(), "JobSpy returned raw jobs");
                    raw_jobs.extend(jobs);
                }
                Ok(Err(e)) => {
                    tracing::error!(error = %e, "JobSpy driver failed during discovery");
                    errors.push(e);
                }
                Err(e) => {
                    tracing::error!(error = %e, "JobSpy spawn_blocking join error");
                    errors.push(SourceError::NonRetryable {
                        source_name: "jobspy".to_owned(),
                        message:     format!("task join error: {e}"),
                    });
                }
            }
        }

        // 2. JapanDev (async).
        if run_japandev {
            if let Some(ref jd) = self.japandev {
                match jd.fetch_jobs(criteria).await {
                    Ok(jobs) => {
                        tracing::info!(count = jobs.len(), "JapanDev returned raw jobs");
                        raw_jobs.extend(jobs);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "JapanDev driver failed during discovery");
                        errors.push(e);
                    }
                }
            }
        }

        if raw_jobs.is_empty() {
            return DiscoveryResult {
                jobs:  vec![],
                error: errors.into_iter().next(),
            };
        }

        log_description_coverage_by_source(&raw_jobs);

        // Normalize raw -> NormalizedJob via TryFrom.
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
            // Return only the first error if any (non-fatal -- we still return
            // whatever jobs were collected from successful drivers).
            error: errors.into_iter().next(),
        }
    }
}

// ===========================================================================
// DiscoveryResult
// ===========================================================================

/// The outcome of a discovery run.
#[derive(Debug)]
pub struct DiscoveryResult {
    /// Successfully normalized and deduplicated jobs.
    pub jobs:  Vec<NormalizedJob>,
    /// If the driver encountered an unrecoverable error, it is captured here.
    pub error: Option<SourceError>,
}

// ===========================================================================
// Private helpers (description coverage logging)
// ===========================================================================

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

// ===========================================================================
// Tests
// ===========================================================================

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
