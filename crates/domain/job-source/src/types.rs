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
use job_domain_shared::convert;
use job_model::job::{Job, JobStatus};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{err::SourceError, jobspy::JOBSPY_SOURCE_NAME};

// ---------------------------------------------------------------------------
// DiscoveryCriteria
// ---------------------------------------------------------------------------

/// Search parameters used to discover jobs from a source.
///
/// Drivers translate these high-level criteria into whatever query
/// format their backing platform requires.
#[derive(Debug, Clone, Default, Serialize, Deserialize, bon::Builder)]
pub struct DiscoveryCriteria {
    /// Keyword terms to search for (e.g. "rust engineer").
    #[builder(with = |it: impl IntoIterator<Item = impl Into<String>>| {
        it.into_iter().map(Into::into).collect::<Vec<String>>()
    })]
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

        let company = raw
            .company
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
// ScrapedJob → RawJob
// ---------------------------------------------------------------------------

impl From<jobspy_sys::types::ScrapedJob> for RawJob {
    fn from(job: jobspy_sys::types::ScrapedJob) -> Self {
        // Use job_url as the source identifier; fall back to "unknown".
        let source_job_id = job.job_url.as_deref().unwrap_or("unknown").to_owned();
        let source_name = job.site.as_deref().unwrap_or(JOBSPY_SOURCE_NAME).to_owned();

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

        Self {
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
// Job (DB model) → NormalizedJob
// ---------------------------------------------------------------------------

impl From<Job> for NormalizedJob {
    fn from(row: Job) -> Self {
        Self {
            id:              row.id,
            source_job_id:   row.source_job_id,
            source_name:     row.source_name,
            title:           row.title,
            company:         row.company,
            location:        row.location,
            description:     row.description,
            url:             row.url,
            salary_min:      row.salary_min,
            salary_max:      row.salary_max,
            salary_currency: row.salary_currency,
            tags:            row.tags,
            raw_data:        row.raw_data,
            posted_at:       convert::chrono_opt_to_timestamp(row.posted_at),
        }
    }
}

// ---------------------------------------------------------------------------
// NormalizedJob → Job (for INSERT — fills in defaults)
// ---------------------------------------------------------------------------

impl From<NormalizedJob> for Job {
    fn from(nj: NormalizedJob) -> Self {
        let now = chrono::Utc::now();
        Self {
            id:              nj.id,
            source_job_id:   nj.source_job_id,
            source_name:     nj.source_name,
            title:           nj.title,
            company:         nj.company,
            location:        nj.location,
            description:     nj.description,
            url:             nj.url,
            salary_min:      nj.salary_min,
            salary_max:      nj.salary_max,
            salary_currency: nj.salary_currency,
            tags:            nj.tags,
            status:          JobStatus::Active,
            raw_data:        nj.raw_data,
            trace_id:        None,
            is_deleted:      false,
            deleted_at:      None,
            posted_at:       convert::timestamp_opt_to_chrono(nj.posted_at),
            created_at:      now,
            updated_at:      now,
        }
    }
}
