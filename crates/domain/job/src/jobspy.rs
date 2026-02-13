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
    error::SourceError,
    types::{DiscoveryCriteria, RawJob},
};

/// Source name constant for the JobSpy driver.
pub const JOBSPY_SOURCE_NAME: &str = "jobspy";

/// Default sites to scrape: Indeed, LinkedIn.
const DEFAULT_SITES: &[jobspy_sys::types::SiteName] = &[
    jobspy_sys::types::SiteName::Indeed,
    jobspy_sys::types::SiteName::LinkedIn,
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

        // Deserialize job_type string -> jobspy_sys JobType via serde aliases.
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

        // Use caller-provided sites when non-empty; fall back to defaults.
        let sites = resolve_sites(&query.sites);
        let linkedin_fetch_description = resolve_linkedin_fetch_description(&sites);

        let params = jobspy_sys::types::ScrapeParams::builder()
            .site_name(sites)
            .search_term(search_term)
            .maybe_location(query.location.clone())
            .maybe_job_type(job_type)
            .maybe_results_wanted(query.max_results)
            .maybe_hours_old(hours_old)
            .maybe_linkedin_fetch_description(linkedin_fetch_description)
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

fn resolve_sites(query_sites: &[String]) -> Vec<jobspy_sys::types::SiteName> {
    if query_sites.is_empty() {
        return DEFAULT_SITES.to_vec();
    }

    let parsed: Vec<_> = query_sites
        .iter()
        .filter_map(|s| serde_json::from_value(serde_json::Value::String(s.to_lowercase())).ok())
        .collect();
    if parsed.is_empty() {
        DEFAULT_SITES.to_vec()
    } else {
        parsed
    }
}

fn resolve_linkedin_fetch_description(sites: &[jobspy_sys::types::SiteName]) -> Option<bool> {
    resolve_linkedin_fetch_description_with_override(
        sites,
        parse_env_bool("JOBSPY_LINKEDIN_FETCH_DESCRIPTION"),
    )
}

fn resolve_linkedin_fetch_description_with_override(
    sites: &[jobspy_sys::types::SiteName],
    env_override: Option<bool>,
) -> Option<bool> {
    if !sites.contains(&jobspy_sys::types::SiteName::LinkedIn) {
        return None;
    }

    env_override.or(Some(true))
}

fn parse_env_bool(name: &str) -> Option<bool> {
    let value = std::env::var(name).ok()?;
    parse_bool_value(&value)
}

fn parse_bool_value(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use jobspy_sys::types::SiteName;

    use super::*;

    #[test]
    fn resolve_sites_falls_back_to_default_when_empty() {
        assert_eq!(resolve_sites(&[]), DEFAULT_SITES.to_vec());
    }

    #[test]
    fn resolve_sites_parses_known_values() {
        let sites = resolve_sites(&["linkedin".to_owned(), "indeed".to_owned()]);
        assert_eq!(sites, vec![SiteName::LinkedIn, SiteName::Indeed]);
    }

    #[test]
    fn resolve_sites_falls_back_to_default_when_all_invalid() {
        let sites = resolve_sites(&["bogus".to_owned()]);
        assert_eq!(sites, DEFAULT_SITES.to_vec());
    }

    #[test]
    fn linkedin_fetch_description_defaults_true_for_linkedin() {
        let value = resolve_linkedin_fetch_description_with_override(&[SiteName::LinkedIn], None);
        assert_eq!(value, Some(true));
    }

    #[test]
    fn linkedin_fetch_description_respects_env_override() {
        let value =
            resolve_linkedin_fetch_description_with_override(&[SiteName::LinkedIn], Some(false));
        assert_eq!(value, Some(false));
    }

    #[test]
    fn linkedin_fetch_description_is_none_without_linkedin() {
        let value =
            resolve_linkedin_fetch_description_with_override(&[SiteName::Indeed], Some(true));
        assert_eq!(value, None);
    }

    #[test]
    fn parse_bool_value_handles_expected_inputs() {
        assert_eq!(parse_bool_value("true"), Some(true));
        assert_eq!(parse_bool_value("1"), Some(true));
        assert_eq!(parse_bool_value("off"), Some(false));
        assert_eq!(parse_bool_value("0"), Some(false));
        assert_eq!(parse_bool_value("unknown"), None);
    }

    #[test]
    fn test_name() {
        common_telemetry::logging::init_default_ut_logging();

        let d = JobSpyDriver::new().unwrap();
        let v = d
            .fetch_jobs(&DiscoveryCriteria::builder().keywords(["golang"]).build())
            .unwrap();
        println!("{:?}", v)
    }
}
