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

//! [`JobSpyDriver`] — job scraping driver powered by python-jobspy.
//!
//! This module provides the primary job discovery implementation using
//! the `jobspy-sys` crate (a PyO3 wrapper around python-jobspy) to
//! scrape 8+ job boards including LinkedIn, Indeed, Glassdoor, Google,
//! ZipRecruiter, Bayt, Naukri, and BDJobs.

use crate::{
    err::SourceError,
    types::{DiscoveryCriteria, RawJob},
};

/// Source name constant for the JobSpy driver.
pub const JOBSPY_SOURCE_NAME: &str = "jobspy";

/// Default sites to scrape: Indeed, LinkedIn, Glassdoor.
const DEFAULT_SITES: &[jobspy_sys::types::SiteName] = &[
    jobspy_sys::types::SiteName::Indeed,
    jobspy_sys::types::SiteName::LinkedIn,
    jobspy_sys::types::SiteName::Glassdoor,
];

/// Job source driver that uses python-jobspy to scrape multiple job boards.
///
/// Wraps the `jobspy_sys::JobSpy` Python bridge and translates between
/// the domain's [`DiscoveryCriteria`] / [`RawJob`] types and the
/// `jobspy_sys::types::ScrapeParams` / `ScrapedJob` types.
#[derive(derive_more::Debug)]
pub struct JobSpyDriver {
    #[debug(skip)]
    jobspy: jobspy_sys::JobSpy,
}

impl JobSpyDriver {
    /// Create a new `JobSpyDriver`.
    ///
    /// Initializes the underlying Python environment and JobSpy library.
    /// Scrapes Indeed, LinkedIn, and Glassdoor by default.
    pub fn new() -> Result<Self, SourceError> {
        let jobspy = jobspy_sys::JobSpy::new().map_err(|e| SourceError::NonRetryable {
            source_name: JOBSPY_SOURCE_NAME.to_owned(),
            message:     format!("Failed to initialize JobSpy: {e}"),
        })?;
        Ok(Self { jobspy })
    }

    /// Fetch raw job listings that match the given criteria.
    pub fn fetch_jobs(&self, query: &DiscoveryCriteria) -> Result<Vec<RawJob>, SourceError> {
        let search_term = query.keywords.join(" ");
        if search_term.is_empty() {
            return Ok(Vec::new());
        }

        // Deserialize job_type string → jobspy_sys JobType via serde aliases.
        let job_type = query.job_type.as_deref().and_then(|jt| {
            serde_json::from_value(serde_json::Value::String(jt.to_lowercase())).ok()
        });

        // Convert posted_after timestamp into an hours_old integer.
        let hours_old = query.posted_after.map(|ts| {
            let now = jiff::Timestamp::now();
            let diff_secs = now.as_second().saturating_sub(ts.as_second());
            let hours = diff_secs / 3600;
            // Clamp: minimum 1 hour, cap at u32::MAX.
            u32::try_from(hours.max(1)).unwrap_or(u32::MAX)
        });

        let params = jobspy_sys::types::ScrapeParams::builder()
            .site_name(DEFAULT_SITES.to_vec())
            .search_term(search_term)
            .maybe_location(query.location.clone())
            .maybe_job_type(job_type)
            .maybe_results_wanted(query.max_results)
            .maybe_hours_old(hours_old)
            .build();

        // Call Python — the GIL serializes execution so blocking is expected.
        let scraped = self.jobspy.scrape_jobs(&params).map_err(|e| {
            // Check for rate-limiting indicators in the error message.
            if e.contains("429") || e.to_lowercase().contains("rate limit") {
                SourceError::RateLimited {
                    source_name:      JOBSPY_SOURCE_NAME.to_owned(),
                    retry_after_secs: 60,
                }
            } else {
                SourceError::Retryable {
                    source_name: JOBSPY_SOURCE_NAME.to_owned(),
                    message:     e,
                }
            }
        })?;

        let raw_jobs = scraped.into_iter().map(RawJob::from).collect();
        Ok(raw_jobs)
    }
}
