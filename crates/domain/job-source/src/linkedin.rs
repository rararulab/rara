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

//! Stub [`LinkedInSource`] driver.
//!
//! This module provides the skeleton for a LinkedIn scraping driver.
//! The actual HTTP scraping / API integration will be added in a later
//! milestone.

use uuid::Uuid;

use crate::{
    driver::JobSourceDriver,
    types::{DiscoveryCriteria, NormalizedJob, RawJob, SourceError},
};

/// Source name constant for the LinkedIn driver.
pub const LINKEDIN_SOURCE_NAME: &str = "linkedin";

/// Stub job source driver for LinkedIn.
///
/// All methods are placeholder implementations that log a warning and
/// return sensible defaults until the real scraping logic is wired up.
#[derive(Debug, Clone, Default)]
pub struct LinkedInSource;

impl LinkedInSource {
    /// Create a new `LinkedInSource` driver.
    #[must_use]
    pub const fn new() -> Self { Self }
}

#[async_trait::async_trait]
impl JobSourceDriver for LinkedInSource {
    fn source_name(&self) -> &str { LINKEDIN_SOURCE_NAME }

    /// Stub implementation -- returns an empty list.
    async fn fetch_jobs(&self, _query: &DiscoveryCriteria) -> Result<Vec<RawJob>, SourceError> {
        tracing::warn!(
            "LinkedInSource: fetch_jobs is not yet implemented; returning empty results"
        );
        Ok(Vec::new())
    }

    /// Basic field-mapping normalization.
    ///
    /// Validates that the minimum required fields (`title`, `company`)
    /// are present, then maps the raw data into a [`NormalizedJob`].
    async fn normalize(&self, raw: RawJob) -> Result<NormalizedJob, SourceError> {
        let title = raw.title.filter(|s| !s.is_empty()).ok_or_else(|| {
            SourceError::NormalizationFailed {
                source_name:   LINKEDIN_SOURCE_NAME.to_owned(),
                source_job_id: raw.source_job_id.clone(),
                message:       "title is required".to_owned(),
            }
        })?;

        let company = raw.company.filter(|s| !s.is_empty()).ok_or_else(|| {
            SourceError::NormalizationFailed {
                source_name:   LINKEDIN_SOURCE_NAME.to_owned(),
                source_job_id: raw.source_job_id.clone(),
                message:       "company is required".to_owned(),
            }
        })?;

        // Trim whitespace from text fields as a basic cleaning step.
        let title = title.trim().to_owned();
        let company = company.trim().to_owned();
        let location = raw.location.map(|l| l.trim().to_owned());

        Ok(NormalizedJob {
            id: Uuid::new_v4(),
            source_job_id: raw.source_job_id,
            source_name: LINKEDIN_SOURCE_NAME.to_owned(),
            title,
            company,
            location,
            description: raw.description,
            url: raw.url,
            salary_min: raw.salary_min,
            salary_max: raw.salary_max,
            salary_currency: raw.salary_currency,
            tags: raw.tags,
            raw_data: raw.raw_data,
            posted_at: raw.posted_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_jobs_returns_empty() {
        let source = LinkedInSource::new();
        let criteria = DiscoveryCriteria::default();
        let jobs = source.fetch_jobs(&criteria).await.unwrap();
        assert!(jobs.is_empty());
    }

    #[tokio::test]
    async fn normalize_trims_fields() {
        let source = LinkedInSource::new();
        let raw = RawJob {
            source_job_id:   "li-42".to_owned(),
            source_name:     LINKEDIN_SOURCE_NAME.to_owned(),
            title:           Some("  Senior Rust Dev  ".to_owned()),
            company:         Some("  LinkedIn Corp  ".to_owned()),
            location:        Some("  San Francisco, CA  ".to_owned()),
            description:     Some("A great role.".to_owned()),
            url:             Some("https://linkedin.com/jobs/42".to_owned()),
            salary_min:      Some(150_000),
            salary_max:      Some(200_000),
            salary_currency: Some("USD".to_owned()),
            tags:            vec!["rust".to_owned(), "backend".to_owned()],
            raw_data:        None,
            posted_at:       None,
        };

        let normalized = source.normalize(raw).await.unwrap();
        assert_eq!(normalized.title, "Senior Rust Dev");
        assert_eq!(normalized.company, "LinkedIn Corp");
        assert_eq!(normalized.location.as_deref(), Some("San Francisco, CA"));
    }

    #[tokio::test]
    async fn normalize_fails_without_company() {
        let source = LinkedInSource::new();
        let raw = RawJob {
            source_job_id:   "li-99".to_owned(),
            source_name:     LINKEDIN_SOURCE_NAME.to_owned(),
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
        let result = source.normalize(raw).await;
        assert!(result.is_err());
    }
}
