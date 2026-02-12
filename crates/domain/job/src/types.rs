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
use job_domain_shared::convert::{self, chrono_opt_to_timestamp, chrono_to_timestamp, u8_from_i16};
use job_model::{
    job::{Job, JobStatus},
    saved_job::{SavedJob as StoreSavedJob, SavedJobEvent as StoreSavedJobEvent},
};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, FromRepr};
use uuid::Uuid;

use crate::{error::SourceError, jobspy::JOBSPY_SOURCE_NAME};

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

/// API response model for the job discovery endpoint.
///
/// This keeps the existing fields and exposes a stable subset of
/// detail fields extracted from the source payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
// Job (DB model) -> NormalizedJob
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
// NormalizedJob -> Job (for INSERT -- fills in defaults)
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
// Tracker types
// ===========================================================================

// ---------------------------------------------------------------------------
// Saved job status
// ---------------------------------------------------------------------------

/// Pipeline status for a saved job.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, FromRepr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SavedJobStatus {
    /// Waiting to be crawled.
    PendingCrawl = 0,
    /// Currently being crawled.
    Crawling = 1,
    /// Crawl completed successfully.
    Crawled = 2,
    /// Currently being analyzed by AI.
    Analyzing = 3,
    /// Analysis completed successfully.
    Analyzed = 4,
    /// An error occurred during crawl or analysis.
    Failed = 5,
    /// The job posting has expired.
    Expired = 6,
}

// ---------------------------------------------------------------------------
// Pipeline event enums
// ---------------------------------------------------------------------------

/// Pipeline stage for a saved job event.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, FromRepr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PipelineStage {
    Crawl = 0,
    Analyze = 1,
    Gc = 2,
}

/// Kind of pipeline event.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, FromRepr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PipelineEventKind {
    Started = 0,
    Completed = 1,
    Failed = 2,
    Info = 3,
}

// ---------------------------------------------------------------------------
// Pipeline event model
// ---------------------------------------------------------------------------

/// A pipeline event for a saved job (domain representation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineEvent {
    pub id:           Uuid,
    pub saved_job_id: Uuid,
    pub stage:        PipelineStage,
    pub event_kind:   PipelineEventKind,
    pub message:      String,
    pub metadata:     Option<serde_json::Value>,
    pub created_at:   Timestamp,
}

fn pipeline_stage_from_i16(value: i16) -> PipelineStage {
    let repr = u8_from_i16(value, "saved_job_event.stage");
    PipelineStage::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid saved_job_event.stage: {value}"))
}

fn pipeline_event_kind_from_i16(value: i16) -> PipelineEventKind {
    let repr = u8_from_i16(value, "saved_job_event.event_kind");
    PipelineEventKind::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid saved_job_event.event_kind: {value}"))
}

/// Store `SavedJobEvent` -> Domain `PipelineEvent`.
impl From<StoreSavedJobEvent> for PipelineEvent {
    fn from(r: StoreSavedJobEvent) -> Self {
        Self {
            id:           r.id,
            saved_job_id: r.saved_job_id,
            stage:        pipeline_stage_from_i16(r.stage),
            event_kind:   pipeline_event_kind_from_i16(r.event_kind),
            message:      r.message,
            metadata:     r.metadata,
            created_at:   chrono_to_timestamp(r.created_at),
        }
    }
}

// ---------------------------------------------------------------------------
// Domain model
// ---------------------------------------------------------------------------

/// A saved job posting (domain representation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedJob {
    pub id:               Uuid,
    pub url:              String,
    pub title:            Option<String>,
    pub company:          Option<String>,
    pub status:           SavedJobStatus,
    pub markdown_s3_key:  Option<String>,
    pub markdown_preview: Option<String>,
    pub analysis_result:  Option<serde_json::Value>,
    pub match_score:      Option<f32>,
    pub error_message:    Option<String>,
    pub crawled_at:       Option<Timestamp>,
    pub analyzed_at:      Option<Timestamp>,
    pub expires_at:       Option<Timestamp>,
    pub created_at:       Timestamp,
    pub updated_at:       Timestamp,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Parameters for saving a new job by URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSavedJobRequest {
    pub url: String,
}

/// Optional query filter for listing saved jobs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedJobFilter {
    /// Filter by pipeline status name (e.g. `"analyzed"`).
    pub status: Option<String>,
}

// ---------------------------------------------------------------------------
// DB model conversions
// ---------------------------------------------------------------------------

fn saved_job_status_from_i16(value: i16) -> SavedJobStatus {
    let repr = u8_from_i16(value, "saved_job.status");
    SavedJobStatus::from_repr(repr).unwrap_or_else(|| panic!("invalid saved_job.status: {value}"))
}

/// Store `SavedJob` -> Domain `SavedJob`.
impl From<StoreSavedJob> for SavedJob {
    fn from(r: StoreSavedJob) -> Self {
        Self {
            id:               r.id,
            url:              r.url,
            title:            r.title,
            company:          r.company,
            status:           saved_job_status_from_i16(r.status),
            markdown_s3_key:  r.markdown_s3_key,
            markdown_preview: r.markdown_preview,
            analysis_result:  r.analysis_result,
            match_score:      r.match_score,
            error_message:    r.error_message,
            crawled_at:       chrono_opt_to_timestamp(r.crawled_at),
            analyzed_at:      chrono_opt_to_timestamp(r.analyzed_at),
            expires_at:       chrono_opt_to_timestamp(r.expires_at),
            created_at:       chrono_to_timestamp(r.created_at),
            updated_at:       chrono_to_timestamp(r.updated_at),
        }
    }
}

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

    // -- Tracker tests ----------------------------------------------------

    #[test]
    fn test_saved_job_status_display() {
        assert_eq!(SavedJobStatus::PendingCrawl.to_string(), "pending_crawl");
        assert_eq!(SavedJobStatus::Crawling.to_string(), "crawling");
        assert_eq!(SavedJobStatus::Crawled.to_string(), "crawled");
        assert_eq!(SavedJobStatus::Analyzing.to_string(), "analyzing");
        assert_eq!(SavedJobStatus::Analyzed.to_string(), "analyzed");
        assert_eq!(SavedJobStatus::Failed.to_string(), "failed");
        assert_eq!(SavedJobStatus::Expired.to_string(), "expired");
    }

    #[test]
    fn test_saved_job_status_from_repr() {
        assert_eq!(
            SavedJobStatus::from_repr(0),
            Some(SavedJobStatus::PendingCrawl)
        );
        assert_eq!(SavedJobStatus::from_repr(1), Some(SavedJobStatus::Crawling));
        assert_eq!(SavedJobStatus::from_repr(2), Some(SavedJobStatus::Crawled));
        assert_eq!(
            SavedJobStatus::from_repr(3),
            Some(SavedJobStatus::Analyzing)
        );
        assert_eq!(SavedJobStatus::from_repr(4), Some(SavedJobStatus::Analyzed));
        assert_eq!(SavedJobStatus::from_repr(5), Some(SavedJobStatus::Failed));
        assert_eq!(SavedJobStatus::from_repr(6), Some(SavedJobStatus::Expired));
        assert_eq!(SavedJobStatus::from_repr(7), None);
    }

    #[test]
    fn test_saved_job_status_repr_roundtrip() {
        for code in 0u8..=6 {
            let status = SavedJobStatus::from_repr(code).unwrap();
            assert_eq!(status as u8, code);
        }
    }

    #[test]
    fn test_saved_job_status_from_i16() {
        assert_eq!(saved_job_status_from_i16(0), SavedJobStatus::PendingCrawl);
        assert_eq!(saved_job_status_from_i16(4), SavedJobStatus::Analyzed);
    }

    #[test]
    #[should_panic(expected = "invalid saved_job.status")]
    fn test_saved_job_status_from_i16_invalid() { saved_job_status_from_i16(99); }

    #[test]
    fn test_pipeline_stage_display() {
        assert_eq!(PipelineStage::Crawl.to_string(), "crawl");
        assert_eq!(PipelineStage::Analyze.to_string(), "analyze");
        assert_eq!(PipelineStage::Gc.to_string(), "gc");
    }

    #[test]
    fn test_pipeline_stage_from_repr() {
        assert_eq!(PipelineStage::from_repr(0), Some(PipelineStage::Crawl));
        assert_eq!(PipelineStage::from_repr(1), Some(PipelineStage::Analyze));
        assert_eq!(PipelineStage::from_repr(2), Some(PipelineStage::Gc));
        assert_eq!(PipelineStage::from_repr(3), None);
    }

    #[test]
    fn test_pipeline_event_kind_display() {
        assert_eq!(PipelineEventKind::Started.to_string(), "started");
        assert_eq!(PipelineEventKind::Completed.to_string(), "completed");
        assert_eq!(PipelineEventKind::Failed.to_string(), "failed");
        assert_eq!(PipelineEventKind::Info.to_string(), "info");
    }

    #[test]
    fn test_pipeline_event_kind_from_repr() {
        assert_eq!(
            PipelineEventKind::from_repr(0),
            Some(PipelineEventKind::Started)
        );
        assert_eq!(
            PipelineEventKind::from_repr(1),
            Some(PipelineEventKind::Completed)
        );
        assert_eq!(
            PipelineEventKind::from_repr(2),
            Some(PipelineEventKind::Failed)
        );
        assert_eq!(
            PipelineEventKind::from_repr(3),
            Some(PipelineEventKind::Info)
        );
        assert_eq!(PipelineEventKind::from_repr(4), None);
    }

    #[test]
    fn test_pipeline_stage_from_i16() {
        assert_eq!(pipeline_stage_from_i16(0), PipelineStage::Crawl);
        assert_eq!(pipeline_stage_from_i16(1), PipelineStage::Analyze);
        assert_eq!(pipeline_stage_from_i16(2), PipelineStage::Gc);
    }

    #[test]
    #[should_panic(expected = "invalid saved_job_event.stage")]
    fn test_pipeline_stage_from_i16_invalid() { pipeline_stage_from_i16(99); }

    #[test]
    fn test_pipeline_event_kind_from_i16() {
        assert_eq!(pipeline_event_kind_from_i16(0), PipelineEventKind::Started);
        assert_eq!(pipeline_event_kind_from_i16(3), PipelineEventKind::Info);
    }

    #[test]
    #[should_panic(expected = "invalid saved_job_event.event_kind")]
    fn test_pipeline_event_kind_from_i16_invalid() { pipeline_event_kind_from_i16(99); }
}
