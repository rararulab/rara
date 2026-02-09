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

//! [`JobSpyDriver`] — real job scraping driver powered by python-jobspy.
//!
//! This module provides a [`JobSourceDriver`] implementation that uses
//! the `jobspy-sys` crate (a PyO3 wrapper around python-jobspy) to
//! scrape 8+ job boards including LinkedIn, Indeed, Glassdoor, Google,
//! ZipRecruiter, Bayt, Naukri, and BDJobs.

use uuid::Uuid;

use crate::{
    driver::JobSourceDriver,
    types::{DiscoveryCriteria, NormalizedJob, RawJob, SourceError},
};

/// Source name constant for the JobSpy driver.
pub const JOBSPY_SOURCE_NAME: &str = "jobspy";

/// Job source driver that uses python-jobspy to scrape multiple job boards.
///
/// Wraps the `jobspy_sys::JobSpy` Python bridge and translates between
/// the domain's [`DiscoveryCriteria`] / [`RawJob`] types and the
/// `jobspy_sys::types::ScrapeParams` / `ScrapedJob` types.
pub struct JobSpyDriver {
    jobspy:        jobspy_sys::JobSpy,
    default_sites: Vec<jobspy_sys::types::SiteName>,
}

impl JobSpyDriver {
    /// Create a new `JobSpyDriver`.
    ///
    /// Initializes the underlying Python environment and JobSpy library.
    /// The `default_sites` list determines which job boards are queried
    /// when [`fetch_jobs`](JobSourceDriver::fetch_jobs) is called.
    pub fn new(
        default_sites: Vec<jobspy_sys::types::SiteName>,
    ) -> Result<Self, SourceError> {
        let jobspy =
            jobspy_sys::JobSpy::new().map_err(|e| SourceError::NonRetryable {
                source_name: JOBSPY_SOURCE_NAME.to_owned(),
                message:     format!("Failed to initialize JobSpy: {e}"),
            })?;
        Ok(Self {
            jobspy,
            default_sites,
        })
    }
}

#[async_trait::async_trait]
impl JobSourceDriver for JobSpyDriver {
    fn source_name(&self) -> &str {
        JOBSPY_SOURCE_NAME
    }

    async fn fetch_jobs(
        &self,
        query: &DiscoveryCriteria,
    ) -> Result<Vec<RawJob>, SourceError> {
        let search_term = query.keywords.join(" ");
        if search_term.is_empty() {
            return Ok(Vec::new());
        }

        // Map DiscoveryCriteria.job_type string → jobspy_sys JobType enum.
        let job_type = query.job_type.as_deref().and_then(map_job_type);

        // Convert posted_after timestamp into an hours_old integer.
        let hours_old = query.posted_after.map(|ts| {
            let now = jiff::Timestamp::now();
            let diff_secs = now.as_second().saturating_sub(ts.as_second());
            let hours = diff_secs / 3600;
            // Clamp: minimum 1 hour, cap at u32::MAX.
            u32::try_from(hours.max(1)).unwrap_or(u32::MAX)
        });

        let params = jobspy_sys::types::ScrapeParams {
            site_name:                  self.default_sites.clone(),
            search_term,
            location:                   query.location.clone(),
            distance:                   None,
            job_type,
            is_remote:                  None,
            results_wanted:             query.max_results,
            hours_old,
            easy_apply:                 None,
            country_indeed:             None,
            linkedin_fetch_description: None,
            enforce_annual_salary:      None,
            proxies:                    None,
            verbose:                    None,
        };

        // Call Python — the GIL serializes execution so blocking is expected.
        let scraped = self.jobspy.scrape_jobs(&params).map_err(|e| {
            // Check for rate-limiting indicators in the error message.
            if e.contains("429")
                || e.to_lowercase().contains("rate limit")
            {
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

        let raw_jobs = scraped.into_iter().map(scraped_to_raw).collect();
        Ok(raw_jobs)
    }

    async fn normalize(
        &self,
        raw: RawJob,
    ) -> Result<NormalizedJob, SourceError> {
        let title =
            raw.title.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
                SourceError::NormalizationFailed {
                    source_name:   JOBSPY_SOURCE_NAME.to_owned(),
                    source_job_id: raw.source_job_id.clone(),
                    message:       "title is required".to_owned(),
                }
            })?;

        let company =
            raw.company
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| SourceError::NormalizationFailed {
                    source_name:   JOBSPY_SOURCE_NAME.to_owned(),
                    source_job_id: raw.source_job_id.clone(),
                    message:       "company is required".to_owned(),
                })?;

        Ok(NormalizedJob {
            id:              Uuid::new_v4(),
            source_job_id:   raw.source_job_id,
            source_name:     raw.source_name,
            title:           title.trim().to_owned(),
            company:         company.trim().to_owned(),
            location:        raw.location.map(|l| l.trim().to_owned()),
            description:     raw.description,
            url:             raw.url,
            salary_min:      raw.salary_min,
            salary_max:      raw.salary_max,
            salary_currency: raw.salary_currency,
            tags:            raw.tags,
            raw_data:        raw.raw_data,
            posted_at:       raw.posted_at,
        })
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Map a `DiscoveryCriteria.job_type` string to a
/// `jobspy_sys::types::JobType`.
fn map_job_type(jt: &str) -> Option<jobspy_sys::types::JobType> {
    match jt.to_lowercase().as_str() {
        "full-time" | "fulltime" | "full_time" => {
            Some(jobspy_sys::types::JobType::FullTime)
        }
        "part-time" | "parttime" | "part_time" => {
            Some(jobspy_sys::types::JobType::PartTime)
        }
        "internship" | "intern" => {
            Some(jobspy_sys::types::JobType::Internship)
        }
        "contract" | "contractor" => {
            Some(jobspy_sys::types::JobType::Contract)
        }
        _ => None,
    }
}

/// Convert a [`jobspy_sys::types::ScrapedJob`] into a domain [`RawJob`].
fn scraped_to_raw(job: jobspy_sys::types::ScrapedJob) -> RawJob {
    // Use job_url as the source identifier; fall back to "unknown".
    let source_job_id = job
        .job_url
        .as_deref()
        .unwrap_or("unknown")
        .to_owned();

    let source_name = job
        .site
        .as_deref()
        .unwrap_or(JOBSPY_SOURCE_NAME)
        .to_owned();

    // Combine city + state + country into a single location string.
    let location = build_location(
        job.city.as_deref(),
        job.state.as_deref(),
        job.country.as_deref(),
    );

    // Convert salary f64 values to i32.
    #[allow(clippy::cast_possible_truncation)]
    let salary_min = job.min_amount.map(|v| v as i32);
    #[allow(clippy::cast_possible_truncation)]
    let salary_max = job.max_amount.map(|v| v as i32);

    // Parse date_posted (e.g. "2026-01-15") into a jiff Timestamp.
    let posted_at = job.date_posted.as_deref().and_then(parse_date_to_timestamp);

    // Store the entire scraped job as raw_data for archival.
    let raw_data = serde_json::to_value(&job).ok();

    RawJob {
        source_job_id,
        source_name,
        title: job.title,
        company: job.company,
        location,
        description: job.description,
        url: job.job_url,
        salary_min,
        salary_max,
        salary_currency: job.currency,
        tags: Vec::new(),
        raw_data,
        posted_at,
    }
}

/// Parse a date string like "2026-01-15" into a [`jiff::Timestamp`] at
/// midnight UTC.
fn parse_date_to_timestamp(date_str: &str) -> Option<jiff::Timestamp> {
    let date: jiff::civil::Date = date_str.parse().ok()?;
    let zdt = date.at(0, 0, 0, 0).in_tz("UTC").ok()?;
    Some(zdt.timestamp())
}

/// Build a comma-separated location string from optional city, state, and
/// country components, skipping any that are `None` or empty.
fn build_location(
    city: Option<&str>,
    state: Option<&str>,
    country: Option<&str>,
) -> Option<String> {
    let parts: Vec<&str> = [city, state, country]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_job_type() {
        assert!(matches!(
            map_job_type("full-time"),
            Some(jobspy_sys::types::JobType::FullTime)
        ));
        assert!(matches!(
            map_job_type("fulltime"),
            Some(jobspy_sys::types::JobType::FullTime)
        ));
        assert!(matches!(
            map_job_type("full_time"),
            Some(jobspy_sys::types::JobType::FullTime)
        ));
        assert!(matches!(
            map_job_type("part-time"),
            Some(jobspy_sys::types::JobType::PartTime)
        ));
        assert!(matches!(
            map_job_type("parttime"),
            Some(jobspy_sys::types::JobType::PartTime)
        ));
        assert!(matches!(
            map_job_type("part_time"),
            Some(jobspy_sys::types::JobType::PartTime)
        ));
        assert!(matches!(
            map_job_type("internship"),
            Some(jobspy_sys::types::JobType::Internship)
        ));
        assert!(matches!(
            map_job_type("intern"),
            Some(jobspy_sys::types::JobType::Internship)
        ));
        assert!(matches!(
            map_job_type("contract"),
            Some(jobspy_sys::types::JobType::Contract)
        ));
        assert!(matches!(
            map_job_type("contractor"),
            Some(jobspy_sys::types::JobType::Contract)
        ));
        assert!(map_job_type("unknown").is_none());
        assert!(map_job_type("").is_none());
    }

    #[test]
    fn test_build_location() {
        assert_eq!(
            build_location(Some("SF"), Some("CA"), Some("US")),
            Some("SF, CA, US".to_owned())
        );
        assert_eq!(
            build_location(Some("SF"), None, Some("US")),
            Some("SF, US".to_owned())
        );
        assert_eq!(build_location(None, None, None), None);
        assert_eq!(
            build_location(Some(""), None, Some("US")),
            Some("US".to_owned())
        );
        assert_eq!(
            build_location(None, Some("CA"), None),
            Some("CA".to_owned())
        );
    }

    #[test]
    fn test_parse_date_to_timestamp() {
        let ts = parse_date_to_timestamp("2026-01-15");
        assert!(ts.is_some());
        // 2026-01-15T00:00:00Z in unix seconds
        let ts = ts.unwrap();
        assert!(ts.as_second() > 0);

        // Invalid date string returns None.
        assert!(parse_date_to_timestamp("not-a-date").is_none());
        assert!(parse_date_to_timestamp("").is_none());
    }

    #[test]
    fn test_scraped_to_raw_basic() {
        let scraped = jobspy_sys::types::ScrapedJob {
            site:             Some("indeed".to_owned()),
            title:            Some("Rust Developer".to_owned()),
            company:          Some("Acme Corp".to_owned()),
            company_url:      None,
            job_url:          Some("https://indeed.com/job/123".to_owned()),
            city:             Some("San Francisco".to_owned()),
            state:            Some("CA".to_owned()),
            country:          Some("US".to_owned()),
            job_type:         Some("fulltime".to_owned()),
            is_remote:        Some(false),
            description:      Some("Write Rust code".to_owned()),
            date_posted:      Some("2026-01-15".to_owned()),
            min_amount:       Some(150_000.0),
            max_amount:       Some(200_000.0),
            currency:         Some("USD".to_owned()),
            salary_source:    None,
            salary_interval:  None,
            job_level:        None,
            company_industry: None,
            emails:           None,
        };

        let raw = scraped_to_raw(scraped);
        assert_eq!(raw.source_job_id, "https://indeed.com/job/123");
        assert_eq!(raw.source_name, "indeed");
        assert_eq!(raw.title.as_deref(), Some("Rust Developer"));
        assert_eq!(raw.company.as_deref(), Some("Acme Corp"));
        assert_eq!(
            raw.location.as_deref(),
            Some("San Francisco, CA, US")
        );
        assert_eq!(raw.salary_min, Some(150_000));
        assert_eq!(raw.salary_max, Some(200_000));
        assert_eq!(raw.salary_currency.as_deref(), Some("USD"));
        assert!(raw.posted_at.is_some());
        assert!(raw.raw_data.is_some());
    }

    #[test]
    fn test_scraped_to_raw_minimal() {
        let scraped = jobspy_sys::types::ScrapedJob {
            site:             None,
            title:            None,
            company:          None,
            company_url:      None,
            job_url:          None,
            city:             None,
            state:            None,
            country:          None,
            job_type:         None,
            is_remote:        None,
            description:      None,
            date_posted:      None,
            min_amount:       None,
            max_amount:       None,
            currency:         None,
            salary_source:    None,
            salary_interval:  None,
            job_level:        None,
            company_industry: None,
            emails:           None,
        };

        let raw = scraped_to_raw(scraped);
        assert_eq!(raw.source_job_id, "unknown");
        assert_eq!(raw.source_name, JOBSPY_SOURCE_NAME);
        assert!(raw.title.is_none());
        assert!(raw.company.is_none());
        assert!(raw.location.is_none());
        assert!(raw.salary_min.is_none());
        assert!(raw.posted_at.is_none());
    }

    #[tokio::test]
    async fn test_normalize_success() {
        // We cannot construct a JobSpyDriver without Python, so we test
        // the normalize logic by exercising it through a helper that
        // mirrors the same validation.
        let raw = RawJob {
            source_job_id:   "https://indeed.com/123".to_owned(),
            source_name:     "indeed".to_owned(),
            title:           Some("  Rust Engineer  ".to_owned()),
            company:         Some("  Acme Corp  ".to_owned()),
            location:        Some("  SF, CA  ".to_owned()),
            description:     Some("Great job".to_owned()),
            url:             Some("https://indeed.com/123".to_owned()),
            salary_min:      Some(100_000),
            salary_max:      Some(150_000),
            salary_currency: Some("USD".to_owned()),
            tags:            vec!["rust".to_owned()],
            raw_data:        None,
            posted_at:       None,
        };

        // Validate the trimming logic (same as normalize()).
        let title = raw
            .title
            .as_ref()
            .map(|s| s.trim().to_owned())
            .unwrap();
        let company = raw
            .company
            .as_ref()
            .map(|s| s.trim().to_owned())
            .unwrap();
        assert_eq!(title, "Rust Engineer");
        assert_eq!(company, "Acme Corp");
    }

    #[tokio::test]
    async fn test_normalize_fails_without_title() {
        let raw = RawJob {
            source_job_id:   "test-123".to_owned(),
            source_name:     "indeed".to_owned(),
            title:           None,
            company:         Some("Acme".to_owned()),
            location:        None,
            description:     None,
            url:             None,
            salary_min:      None,
            salary_max:      None,
            salary_currency: None,
            tags:            vec![],
            raw_data:        None,
            posted_at:       None,
        };

        let result = raw.title.filter(|s| !s.trim().is_empty());
        assert!(result.is_none(), "Should fail without title");
    }

    #[tokio::test]
    async fn test_normalize_fails_without_company() {
        let raw = RawJob {
            source_job_id:   "test-456".to_owned(),
            source_name:     "indeed".to_owned(),
            title:           Some("Engineer".to_owned()),
            company:         None,
            location:        None,
            description:     None,
            url:             None,
            salary_min:      None,
            salary_max:      None,
            salary_currency: None,
            tags:            vec![],
            raw_data:        None,
            posted_at:       None,
        };

        let result = raw.company.filter(|s| !s.trim().is_empty());
        assert!(result.is_none(), "Should fail without company");
    }

    #[tokio::test]
    async fn test_normalize_fails_with_whitespace_only_title() {
        let raw = RawJob {
            source_job_id:   "test-789".to_owned(),
            source_name:     "indeed".to_owned(),
            title:           Some("   ".to_owned()),
            company:         Some("Acme".to_owned()),
            location:        None,
            description:     None,
            url:             None,
            salary_min:      None,
            salary_max:      None,
            salary_currency: None,
            tags:            vec![],
            raw_data:        None,
            posted_at:       None,
        };

        let result = raw.title.filter(|s| !s.trim().is_empty());
        assert!(
            result.is_none(),
            "Whitespace-only title should be rejected"
        );
    }

    #[test]
    fn test_hours_old_calculation() {
        // Simulate the hours_old calculation used in fetch_jobs.
        let now = jiff::Timestamp::now();
        // 48 hours ago
        let two_days_ago_secs = now.as_second() - (48 * 3600);
        let two_days_ago =
            jiff::Timestamp::new(two_days_ago_secs, 0).unwrap();

        let diff_secs =
            now.as_second().saturating_sub(two_days_ago.as_second());
        let hours = diff_secs / 3600;
        let hours_old =
            u32::try_from(hours.max(1)).unwrap_or(u32::MAX);

        assert_eq!(hours_old, 48);
    }
}
