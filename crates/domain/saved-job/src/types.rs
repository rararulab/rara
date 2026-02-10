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

//! Domain types for saved job tracking.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, FromRepr};
use uuid::Uuid;

use job_domain_shared::convert::{chrono_opt_to_timestamp, chrono_to_timestamp, u8_from_i16};
use job_model::saved_job::SavedJob as StoreSavedJob;

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
    SavedJobStatus::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid saved_job.status: {value}"))
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(SavedJobStatus::from_repr(0), Some(SavedJobStatus::PendingCrawl));
        assert_eq!(SavedJobStatus::from_repr(1), Some(SavedJobStatus::Crawling));
        assert_eq!(SavedJobStatus::from_repr(2), Some(SavedJobStatus::Crawled));
        assert_eq!(SavedJobStatus::from_repr(3), Some(SavedJobStatus::Analyzing));
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
    fn test_saved_job_status_from_i16_invalid() {
        saved_job_status_from_i16(99);
    }
}
