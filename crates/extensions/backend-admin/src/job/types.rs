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

//! Domain types for job discovery, normalization, and saved job tracking.

use jiff::Timestamp;
use rara_domain_shared::convert;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{error::SourceError, jobspy::JOBSPY_SOURCE_NAME};

// ===========================================================================
// Discovery types
// ===========================================================================

// ---------------------------------------------------------------------------
// DiscoveryCriteria
// ---------------------------------------------------------------------------

/// Search parameters used to discover jobs from a source.
///
/// Drivers translate these high-level criteria into whatever query
/// format their backing platform requires.
#[derive(Debug, Clone, Default, Serialize, Deserialize, bon::Builder, utoipa::ToSchema)]
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
    #[schema(value_type = Option<String>)]
    pub posted_after: Option<Timestamp>,
    /// Which job sites to search (e.g. "linkedin", "indeed").
    /// If empty, the driver uses its default set.
    #[serde(default)]
    #[builder(default)]
    pub sites:        Vec<String>,
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

/// Intermediate struct for deserializing AI-parsed job descriptions.
#[derive(Debug, Deserialize)]
pub struct ParsedJob {
    pub title:           String,
    pub company:         String,
    pub location:        Option<String>,
    pub description:     Option<String>,
    pub url:             Option<String>,
    pub salary_min:      Option<i32>,
    pub salary_max:      Option<i32>,
    pub salary_currency: Option<String>,
    pub tags:            Option<Vec<String>>,
}

/// A cleaned, standardized job record ready for persistence.
///
/// All required fields are guaranteed to be present after
/// normalization.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
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
    #[schema(value_type = Option<Object>)]
    pub raw_data:        Option<serde_json::Value>,
    /// When the listing was originally posted.
    #[schema(value_type = Option<String>)]
    pub posted_at:       Option<Timestamp>,
}

impl NormalizedJob {
    /// Build a `NormalizedJob` from an AI-parsed result.
    #[must_use]
    pub fn from_parsed(parsed: ParsedJob, raw_text: &str) -> Self {
        Self {
            id:              Uuid::new_v4(),
            source_job_id:   Uuid::new_v4().to_string(),
            source_name:     "telegram".to_owned(),
            title:           parsed.title,
            company:         parsed.company,
            location:        parsed.location,
            description:     parsed.description,
            url:             parsed.url,
            salary_min:      parsed.salary_min,
            salary_max:      parsed.salary_max,
            salary_currency: parsed.salary_currency,
            tags:            parsed.tags.unwrap_or_default(),
            raw_data:        serde_json::to_value(raw_text).ok(),
            posted_at:       None,
        }
    }
}

/// API response model for the job discovery endpoint.
///
/// This keeps the existing fields and exposes a stable subset of
/// detail fields extracted from the source payload.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DiscoveryJobResponse {
    pub id:               Uuid,
    pub source_job_id:    String,
    pub source_name:      String,
    pub title:            String,
    pub company:          String,
    pub location:         Option<String>,
    pub description:      Option<String>,
    pub url:              Option<String>,
    pub salary_min:       Option<i32>,
    pub salary_max:       Option<i32>,
    pub salary_currency:  Option<String>,
    pub tags:             Vec<String>,
    #[schema(value_type = Option<String>)]
    pub posted_at:        Option<Timestamp>,
    pub job_type:         Option<String>,
    pub is_remote:        Option<bool>,
    pub salary_interval:  Option<String>,
    pub salary_source:    Option<String>,
    pub job_level:        Option<String>,
    pub company_url:      Option<String>,
    pub company_industry: Option<String>,
}

// ---------------------------------------------------------------------------
// RawJob -> NormalizedJob
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
// ScrapedJob -> RawJob
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

/// Parse a date or datetime string into a [`jiff::Timestamp`].
///
/// Handles both ISO 8601 datetime (`"2026-01-15T00:00:00.000Z"`, as
/// produced by pandas `to_json(date_format="iso")`) and plain date
/// strings (`"2026-01-15"`).
fn parse_date_to_timestamp(date_str: &str) -> Option<jiff::Timestamp> {
    // Try full ISO 8601 datetime first (e.g. "2026-01-15T00:00:00.000Z").
    if let Ok(ts) = date_str.parse::<jiff::Timestamp>() {
        return Some(ts);
    }
    // Fall back to date-only (e.g. "2026-01-15") -> midnight UTC.
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
// JobRow (DB model) -> NormalizedJob
// ---------------------------------------------------------------------------

use super::pg_repository::{JobRow, JobStatusDb};

impl From<JobRow> for NormalizedJob {
    fn from(row: JobRow) -> Self {
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
// NormalizedJob -> JobRow (for INSERT -- fills in defaults)
// ---------------------------------------------------------------------------

impl From<NormalizedJob> for JobRow {
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
            status:          JobStatusDb::Active,
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

impl From<NormalizedJob> for DiscoveryJobResponse {
    fn from(job: NormalizedJob) -> Self {
        let detail = job
            .raw_data
            .as_ref()
            .map(DiscoveryDetailFields::from_raw_data)
            .unwrap_or_default();

        Self {
            id:               job.id,
            source_job_id:    job.source_job_id,
            source_name:      job.source_name,
            title:            job.title,
            company:          job.company,
            location:         job.location,
            description:      job.description,
            url:              job.url,
            salary_min:       job.salary_min,
            salary_max:       job.salary_max,
            salary_currency:  job.salary_currency,
            tags:             job.tags,
            posted_at:        job.posted_at,
            job_type:         detail.job_type,
            is_remote:        detail.is_remote,
            salary_interval:  detail.salary_interval,
            salary_source:    detail.salary_source,
            job_level:        detail.job_level,
            company_url:      detail.company_url,
            company_industry: detail.company_industry,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DiscoveryDetailFields {
    job_type:         Option<String>,
    is_remote:        Option<bool>,
    salary_interval:  Option<String>,
    salary_source:    Option<String>,
    job_level:        Option<String>,
    company_url:      Option<String>,
    company_industry: Option<String>,
}

impl DiscoveryDetailFields {
    fn from_raw_data(raw: &serde_json::Value) -> Self {
        Self {
            job_type:         json_opt_non_empty_str(raw, "job_type"),
            is_remote:        json_opt_bool(raw, "is_remote"),
            salary_interval:  json_opt_non_empty_str(raw, "salary_interval"),
            salary_source:    json_opt_non_empty_str(raw, "salary_source"),
            job_level:        json_opt_non_empty_str(raw, "job_level"),
            company_url:      json_opt_non_empty_str(raw, "company_url"),
            company_industry: json_opt_non_empty_str(raw, "company_industry"),
        }
    }
}

fn json_opt_non_empty_str(raw: &serde_json::Value, key: &str) -> Option<String> {
    let value = raw.get(key)?.as_str()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn json_opt_bool(raw: &serde_json::Value, key: &str) -> Option<bool> { raw.get(key)?.as_bool() }


// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Discovery tests --------------------------------------------------

    #[test]
    fn discovery_detail_fields_extract_expected_values() {
        let raw = serde_json::json!({
            "job_type": "fulltime",
            "is_remote": true,
            "salary_interval": "yearly",
            "salary_source": "direct_data",
            "job_level": "senior",
            "company_url": "https://example.com",
            "company_industry": "software"
        });

        let detail = DiscoveryDetailFields::from_raw_data(&raw);
        assert_eq!(
            detail,
            DiscoveryDetailFields {
                job_type:         Some("fulltime".to_owned()),
                is_remote:        Some(true),
                salary_interval:  Some("yearly".to_owned()),
                salary_source:    Some("direct_data".to_owned()),
                job_level:        Some("senior".to_owned()),
                company_url:      Some("https://example.com".to_owned()),
                company_industry: Some("software".to_owned()),
            }
        );
    }

    #[test]
    fn discovery_detail_fields_treats_empty_strings_as_none() {
        let raw = serde_json::json!({
            "job_type": " ",
            "job_level": "",
            "company_url": null
        });

        let detail = DiscoveryDetailFields::from_raw_data(&raw);
        assert_eq!(detail.job_type, None);
        assert_eq!(detail.job_level, None);
        assert_eq!(detail.company_url, None);
    }

    #[test]
    fn parse_date_to_timestamp_plain_date() {
        let ts = parse_date_to_timestamp("2026-01-15").expect("should parse date-only");
        assert_eq!(ts.to_string(), "2026-01-15T00:00:00Z");
    }

    #[test]
    fn parse_date_to_timestamp_iso_datetime() {
        let ts =
            parse_date_to_timestamp("2026-01-15T00:00:00.000Z").expect("should parse ISO datetime");
        assert_eq!(ts.to_string(), "2026-01-15T00:00:00Z");
    }

    #[test]
    fn parse_date_to_timestamp_iso_datetime_with_offset() {
        let ts =
            parse_date_to_timestamp("2026-01-15T08:30:00+08:00").expect("should parse with offset");
        assert_eq!(ts.to_string(), "2026-01-15T00:30:00Z");
    }

    #[test]
    fn parse_date_to_timestamp_invalid() {
        assert!(parse_date_to_timestamp("not-a-date").is_none());
        assert!(parse_date_to_timestamp("").is_none());
    }

}
