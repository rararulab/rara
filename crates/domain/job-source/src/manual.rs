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

//! [`ManualSource`] -- a driver for jobs entered manually through
//! the API.
//!
//! Since manually entered data is already in the user's own format,
//! normalization is essentially a pass-through that validates that the
//! required fields are present.

use uuid::Uuid;

use crate::{
    driver::JobSourceDriver,
    types::{DiscoveryCriteria, NormalizedJob, RawJob, SourceError},
};

/// Source name constant for the manual driver.
pub const MANUAL_SOURCE_NAME: &str = "manual";

/// A job source driver for manually entered job listings.
///
/// This driver does not communicate with any external service.
/// [`fetch_jobs`](ManualSource::fetch_jobs) always returns an empty
/// vec -- the expectation is that manual jobs are pushed into the
/// system via the API and only need to pass through
/// [`normalize`](ManualSource::normalize) before being persisted.
#[derive(Debug, Clone, Default)]
pub struct ManualSource;

impl ManualSource {
    /// Create a new `ManualSource` driver.
    #[must_use]
    pub const fn new() -> Self { Self }
}

#[async_trait::async_trait]
impl JobSourceDriver for ManualSource {
    fn source_name(&self) -> &str { MANUAL_SOURCE_NAME }

    /// Manual jobs are created via the API, not fetched.
    ///
    /// This method always returns an empty list. A future
    /// implementation may query the database for recently-added
    /// manual entries that have not yet been normalized.
    async fn fetch_jobs(&self, _query: &DiscoveryCriteria) -> Result<Vec<RawJob>, SourceError> {
        tracing::debug!("ManualSource: fetch_jobs is a no-op; manual jobs are pushed via API");
        Ok(Vec::new())
    }

    /// Pass-through normalization for manually entered jobs.
    ///
    /// Since the data is entered by the user, it is already
    /// considered clean. The only validation is that `title` and
    /// `company` are present.
    async fn normalize(&self, raw: RawJob) -> Result<NormalizedJob, SourceError> {
        let title = raw.title.filter(|s| !s.is_empty()).ok_or_else(|| {
            SourceError::NormalizationFailed {
                source_name:   MANUAL_SOURCE_NAME.to_owned(),
                source_job_id: raw.source_job_id.clone(),
                message:       "title is required".to_owned(),
            }
        })?;

        let company = raw.company.filter(|s| !s.is_empty()).ok_or_else(|| {
            SourceError::NormalizationFailed {
                source_name:   MANUAL_SOURCE_NAME.to_owned(),
                source_job_id: raw.source_job_id.clone(),
                message:       "company is required".to_owned(),
            }
        })?;

        Ok(NormalizedJob {
            id: Uuid::new_v4(),
            source_job_id: raw.source_job_id,
            source_name: MANUAL_SOURCE_NAME.to_owned(),
            title,
            company,
            location: raw.location,
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
        let source = ManualSource::new();
        let criteria = DiscoveryCriteria::default();
        let jobs = source.fetch_jobs(&criteria).await.unwrap();
        assert!(jobs.is_empty());
    }

    #[tokio::test]
    async fn normalize_succeeds_with_required_fields() {
        let source = ManualSource::new();
        let raw = RawJob {
            source_job_id:   "manual-1".to_owned(),
            source_name:     MANUAL_SOURCE_NAME.to_owned(),
            title:           Some("Rust Engineer".to_owned()),
            company:         Some("Acme Corp".to_owned()),
            location:        Some("Remote".to_owned()),
            description:     None,
            url:             None,
            salary_min:      None,
            salary_max:      None,
            salary_currency: None,
            tags:            vec![],
            raw_data:        None,
            posted_at:       None,
        };
        let normalized = source.normalize(raw).await.unwrap();
        assert_eq!(normalized.title, "Rust Engineer");
        assert_eq!(normalized.company, "Acme Corp");
        assert_eq!(normalized.source_name, MANUAL_SOURCE_NAME);
    }

    #[tokio::test]
    async fn normalize_fails_without_title() {
        let source = ManualSource::new();
        let raw = RawJob {
            source_job_id:   "manual-2".to_owned(),
            source_name:     MANUAL_SOURCE_NAME.to_owned(),
            title:           None,
            company:         Some("Acme Corp".to_owned()),
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
