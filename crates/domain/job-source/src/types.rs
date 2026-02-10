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

//! Domain types for job source discovery and normalization.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use snafu::Snafu;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// DiscoveryCriteria
// ---------------------------------------------------------------------------

/// Search parameters used to discover jobs from a source.
///
/// Drivers translate these high-level criteria into whatever query
/// format their backing platform requires.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveryCriteria {
    /// Keyword terms to search for (e.g. "rust engineer").
    pub keywords:     Vec<String>,
    /// Preferred location or "remote".
    pub location:     Option<String>,
    /// Job type filter (e.g. "full-time", "contract").
    pub job_type:     Option<String>,
    /// Maximum number of results to return per source.
    pub max_results:  Option<u32>,
    /// Only return jobs posted after this timestamp.
    pub posted_after: Option<Timestamp>,
}

// ---------------------------------------------------------------------------
// RawJob
// ---------------------------------------------------------------------------

/// Raw job data as received from an external source, before
/// normalization.
///
/// Every field is optional except the opaque `source_job_id` that the
/// source platform uses to identify the listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawJob {
    /// Opaque identifier assigned by the source platform.
    pub source_job_id:   String,
    /// Name of the source that produced this record (e.g. "linkedin").
    pub source_name:     String,
    /// Raw title string.
    pub title:           Option<String>,
    /// Raw company name.
    pub company:         Option<String>,
    /// Raw location string.
    pub location:        Option<String>,
    /// Raw description / body text.
    pub description:     Option<String>,
    /// URL to the original listing.
    pub url:             Option<String>,
    /// Minimum salary (if provided).
    pub salary_min:      Option<i32>,
    /// Maximum salary (if provided).
    pub salary_max:      Option<i32>,
    /// Salary currency code (e.g. "USD").
    pub salary_currency: Option<String>,
    /// Free-form tags / labels.
    pub tags:            Vec<String>,
    /// The full raw payload for archival purposes.
    pub raw_data:        Option<serde_json::Value>,
    /// When the listing was originally posted.
    pub posted_at:       Option<Timestamp>,
}

// ---------------------------------------------------------------------------
// NormalizedJob
// ---------------------------------------------------------------------------

/// A cleaned, standardized job record ready for persistence.
///
/// All required fields are guaranteed to be present after
/// normalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedJob {
    /// Internal UUID for this job record.
    pub id:              Uuid,
    /// Identifier from the source platform.
    pub source_job_id:   String,
    /// Name of the source (e.g. "manual", "linkedin").
    pub source_name:     String,
    /// Cleaned job title.
    pub title:           String,
    /// Cleaned company name.
    pub company:         String,
    /// Optional location / "remote".
    pub location:        Option<String>,
    /// Optional full description.
    pub description:     Option<String>,
    /// URL to the listing.
    pub url:             Option<String>,
    /// Salary range lower bound.
    pub salary_min:      Option<i32>,
    /// Salary range upper bound.
    pub salary_max:      Option<i32>,
    /// Salary currency code.
    pub salary_currency: Option<String>,
    /// Normalized tags.
    pub tags:            Vec<String>,
    /// The original raw payload, kept for debugging.
    pub raw_data:        Option<serde_json::Value>,
    /// When the listing was originally posted.
    pub posted_at:       Option<Timestamp>,
}

// ---------------------------------------------------------------------------
// RawJob → NormalizedJob
// ---------------------------------------------------------------------------

impl TryFrom<RawJob> for NormalizedJob {
    type Error = SourceError;

    fn try_from(raw: RawJob) -> Result<Self, Self::Error> {
        let title = raw.title.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
            SourceError::NormalizationFailed {
                source_name:   raw.source_name.clone(),
                source_job_id: raw.source_job_id.clone(),
                message:       "title is required".to_owned(),
            }
        })?;

        let company =
            raw.company
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| SourceError::NormalizationFailed {
                    source_name:   raw.source_name.clone(),
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
// SourceError
// ---------------------------------------------------------------------------

/// Errors that a job source driver can produce.
///
/// The variants carry enough information for callers to decide whether
/// to retry, back off, or give up.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SourceError {
    /// A transient failure that can be retried.
    #[snafu(display("Retryable error from source '{source_name}': {message}"))]
    Retryable {
        source_name: String,
        message:     String,
    },

    /// A permanent failure that should not be retried.
    #[snafu(display("Non-retryable error from source '{source_name}': {message}"))]
    NonRetryable {
        source_name: String,
        message:     String,
    },

    /// The source has rate-limited us.
    #[snafu(display("Rate limited by source '{source_name}', retry after {retry_after_secs}s"))]
    RateLimited {
        source_name:      String,
        retry_after_secs: u64,
    },

    /// Authentication / authorization failure.
    #[snafu(display("Auth error for source '{source_name}': {message}"))]
    AuthError {
        source_name: String,
        message:     String,
    },

    /// The raw data could not be normalized into a valid
    /// [`NormalizedJob`].
    #[snafu(display(
        "Normalization failed for job '{source_job_id}' from '{source_name}': {message}"
    ))]
    NormalizationFailed {
        source_name:   String,
        source_job_id: String,
        message:       String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_raw(title: Option<&str>, company: Option<&str>) -> RawJob {
        RawJob {
            source_job_id:   "test-1".to_owned(),
            source_name:     "test".to_owned(),
            title:           title.map(str::to_owned),
            company:         company.map(str::to_owned),
            location:        Some("Remote".to_owned()),
            description:     None,
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
    fn try_from_succeeds_with_required_fields() {
        let raw = make_raw(Some("Rust Engineer"), Some("Acme Corp"));
        let job = NormalizedJob::try_from(raw).unwrap();
        assert_eq!(job.title, "Rust Engineer");
        assert_eq!(job.company, "Acme Corp");
        assert_eq!(job.source_name, "test");
    }

    #[test]
    fn try_from_trims_whitespace() {
        let raw = make_raw(Some("  Rust Engineer  "), Some("  Acme Corp  "));
        let job = NormalizedJob::try_from(raw).unwrap();
        assert_eq!(job.title, "Rust Engineer");
        assert_eq!(job.company, "Acme Corp");
    }

    #[test]
    fn try_from_fails_without_title() {
        let raw = make_raw(None, Some("Acme Corp"));
        assert!(NormalizedJob::try_from(raw).is_err());
    }

    #[test]
    fn try_from_fails_with_blank_title() {
        let raw = make_raw(Some("   "), Some("Acme Corp"));
        assert!(NormalizedJob::try_from(raw).is_err());
    }

    #[test]
    fn try_from_fails_without_company() {
        let raw = make_raw(Some("Rust Engineer"), None);
        assert!(NormalizedJob::try_from(raw).is_err());
    }
}
