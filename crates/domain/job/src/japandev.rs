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

//! [`JapanDevDriver`] — job source driver for japan-dev.com.
//!
//! Fetches job listings from the JapanDev Meilisearch API and converts
//! them to [`RawJob`] records for the discovery pipeline.

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    error::SourceError,
    types::{DiscoveryCriteria, RawJob},
};

/// Source name constant for the JapanDev driver.
pub const JAPANDEV_SOURCE_NAME: &str = "japandev";

/// Configuration for the JapanDev Meilisearch API.
#[derive(Debug, Clone)]
pub struct JapanDevConfig {
    /// Meilisearch base URL.
    pub base_url:      String,
    /// Bearer token for the API.
    pub api_key:       String,
    /// Default result limit per query.
    pub default_limit: u32,
}

impl Default for JapanDevConfig {
    fn default() -> Self {
        Self {
            base_url:      "https://meili.japan-dev.com".to_owned(),
            api_key:       "3838486cea4344beaef2c4c5979be249fc5736ea4aab99fab193b5e7f540ffae"
                .to_owned(),
            default_limit: 60,
        }
    }
}

/// Job source driver for japan-dev.com via the Meilisearch multi-search API.
#[derive(Debug, Clone)]
pub struct JapanDevDriver {
    client: reqwest::Client,
    config: JapanDevConfig,
}

impl JapanDevDriver {
    /// Create a new `JapanDevDriver` with the given configuration.
    #[must_use]
    pub fn new(config: JapanDevConfig) -> Self {
        let client = reqwest::Client::new();
        Self { client, config }
    }

    /// Fetch raw job listings from JapanDev matching the given criteria.
    pub async fn fetch_jobs(
        &self,
        criteria: &DiscoveryCriteria,
    ) -> Result<Vec<RawJob>, SourceError> {
        let q = criteria.keywords.join(" ");
        if q.is_empty() {
            return Ok(Vec::new());
        }

        let limit = criteria.max_results.unwrap_or(self.config.default_limit);
        let url = format!("{}/multi-search", self.config.base_url);

        let body = serde_json::json!({
            "queries": [{
                "indexUid": "Job_production",
                "q": q,
                "limit": limit + 1,
                "offset": 0
            }]
        });

        debug!(url = %url, query = %q, limit, "JapanDev search request");

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    SourceError::Retryable {
                        source_name: JAPANDEV_SOURCE_NAME.to_owned(),
                        message:     format!("request timed out: {e}"),
                    }
                } else {
                    SourceError::Retryable {
                        source_name: JAPANDEV_SOURCE_NAME.to_owned(),
                        message:     format!("HTTP request failed: {e}"),
                    }
                }
            })?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SourceError::AuthError {
                source_name: JAPANDEV_SOURCE_NAME.to_owned(),
                message:     format!("authentication failed (HTTP {status})"),
            });
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SourceError::RateLimited {
                source_name:      JAPANDEV_SOURCE_NAME.to_owned(),
                retry_after_secs: 60,
            });
        }
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(SourceError::Retryable {
                source_name: JAPANDEV_SOURCE_NAME.to_owned(),
                message:     format!("HTTP {status}: {body_text}"),
            });
        }

        let search_response: MultiSearchResponse =
            response.json().await.map_err(|e| SourceError::NonRetryable {
                source_name: JAPANDEV_SOURCE_NAME.to_owned(),
                message:     format!("failed to parse response JSON: {e}"),
            })?;

        let hits: Vec<JapanDevHit> = search_response
            .results
            .into_iter()
            .flat_map(|r| r.hits)
            .collect();

        debug!(hit_count = hits.len(), "JapanDev search returned hits");

        let raw_jobs = hits.into_iter().map(RawJob::from).collect();
        Ok(raw_jobs)
    }
}

// ===========================================================================
// Meilisearch response types (private wire format)
// ===========================================================================

#[derive(Debug, Deserialize)]
struct MultiSearchResponse {
    results: Vec<SearchResult>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    hits: Vec<JapanDevHit>,
}

/// A single job hit from the JapanDev Meilisearch index.
///
/// Deserialize + Serialize: Serialize is needed to store the full hit
/// as `raw_data` in [`RawJob`].
#[derive(Debug, Clone, Deserialize, Serialize)]
struct JapanDevHit {
    id:    String,
    title: String,

    #[serde(default)]
    company_name: Option<String>,
    #[serde(default)]
    location:     Option<String>,
    #[serde(default)]
    slug:         Option<String>,

    #[serde(default)]
    salary_min: Option<i64>,
    #[serde(default)]
    salary_max: Option<i64>,

    #[serde(default)]
    skill_names: Vec<String>,

    #[serde(default)]
    published_at: Option<String>,

    // Description fields.
    #[serde(default)]
    details:      Option<String>,
    #[serde(default)]
    intro:        Option<String>,
    #[serde(default)]
    requirements: Option<String>,
    #[serde(default)]
    benefits:     Option<String>,

    // Metadata fields (stored in raw_data).
    #[serde(default)]
    japanese_level:     Option<String>,
    #[serde(default)]
    english_level:      Option<String>,
    #[serde(default)]
    remote_level:       Option<String>,
    #[serde(default)]
    seniority_level:    Option<String>,
    #[serde(default)]
    employment_type:    Option<String>,
    #[serde(default)]
    candidate_location: Option<String>,
    #[serde(default)]
    sponsors_visas:     Option<bool>,

    // Company sub-object.
    #[serde(default)]
    company: Option<JapanDevCompany>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct JapanDevCompany {
    #[serde(default)]
    name:              Option<String>,
    #[serde(default)]
    is_verified:       Option<bool>,
    #[serde(default)]
    slug:              Option<String>,
    #[serde(default)]
    logo_url:          Option<String>,
    #[serde(default)]
    short_description: Option<String>,
}

// ===========================================================================
// JapanDevHit -> RawJob
// ===========================================================================

impl From<JapanDevHit> for RawJob {
    fn from(hit: JapanDevHit) -> Self {
        let source_job_id = hit.id.clone();

        // Prefer company_name; fall back to company.name.
        let company = hit
            .company_name
            .clone()
            .or_else(|| hit.company.as_ref().and_then(|c| c.name.clone()));

        // Construct URL from slug.
        let url = hit
            .slug
            .as_ref()
            .map(|slug| format!("https://japan-dev.com/jobs/{slug}"));

        // Combine description parts.
        let description = build_description(
            hit.intro.as_deref(),
            hit.details.as_deref(),
            hit.requirements.as_deref(),
            hit.benefits.as_deref(),
        );

        // Convert salary from i64 (JPY) to i32.
        #[allow(clippy::cast_possible_truncation)]
        let salary_min = hit.salary_min.map(|v| v as i32);
        #[allow(clippy::cast_possible_truncation)]
        let salary_max = hit.salary_max.map(|v| v as i32);

        // Parse published_at into a jiff Timestamp.
        let posted_at = hit
            .published_at
            .as_deref()
            .and_then(parse_published_at);

        // Store the full hit as raw_data.
        let raw_data = serde_json::to_value(&hit).ok();

        Self {
            source_job_id,
            source_name: JAPANDEV_SOURCE_NAME.to_owned(),
            title: Some(hit.title),
            company,
            location: hit.location,
            description,
            url,
            salary_min,
            salary_max,
            salary_currency: Some("JPY".to_owned()),
            tags: hit.skill_names,
            raw_data,
            posted_at,
        }
    }
}

/// Combine the description sections into a single string.
fn build_description(
    intro: Option<&str>,
    details: Option<&str>,
    requirements: Option<&str>,
    benefits: Option<&str>,
) -> Option<String> {
    let mut sections = Vec::new();

    if let Some(text) = intro.filter(|s| !s.trim().is_empty()) {
        sections.push(format!("## Introduction\n\n{text}"));
    }
    if let Some(text) = details.filter(|s| !s.trim().is_empty()) {
        sections.push(format!("## Details\n\n{text}"));
    }
    if let Some(text) = requirements.filter(|s| !s.trim().is_empty()) {
        sections.push(format!("## Requirements\n\n{text}"));
    }
    if let Some(text) = benefits.filter(|s| !s.trim().is_empty()) {
        sections.push(format!("## Benefits\n\n{text}"));
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

/// Parse an ISO 8601 datetime string from the JapanDev API.
fn parse_published_at(date_str: &str) -> Option<jiff::Timestamp> {
    // Try full ISO 8601 datetime first (e.g. "2026-01-15T00:00:00.000Z").
    if let Ok(ts) = date_str.parse::<jiff::Timestamp>() {
        return Some(ts);
    }
    // Fall back to date-only (e.g. "2026-01-15") -> midnight UTC.
    let date: jiff::civil::Date = date_str.parse().ok()?;
    let zdt = date.at(0, 0, 0, 0).in_tz("UTC").ok()?;
    Some(zdt.timestamp())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit() -> JapanDevHit {
        JapanDevHit {
            id:                 "3313".to_owned(),
            title:              "Senior Rust Engineer".to_owned(),
            company_name:       Some("Example Corp".to_owned()),
            location:           Some("Tokyo".to_owned()),
            slug:               Some("example-corp-senior-rust-engineer".to_owned()),
            salary_min:         Some(8_000_000),
            salary_max:         Some(12_000_000),
            skill_names:        vec!["Rust".to_owned(), "Kubernetes".to_owned()],
            published_at:       Some("2026-01-15T00:00:00.000Z".to_owned()),
            details:            Some("Work on our backend.".to_owned()),
            intro:              Some("Join our team!".to_owned()),
            requirements:       Some("5+ years Rust.".to_owned()),
            benefits:           Some("Remote friendly.".to_owned()),
            japanese_level:     Some("Business".to_owned()),
            english_level:      Some("Fluent".to_owned()),
            remote_level:       Some("Full remote".to_owned()),
            seniority_level:    Some("Senior".to_owned()),
            employment_type:    Some("Full-time".to_owned()),
            candidate_location: Some("Japan".to_owned()),
            sponsors_visas:     Some(true),
            company:            Some(JapanDevCompany {
                name:              Some("Example Corp".to_owned()),
                is_verified:       Some(true),
                slug:              Some("example-corp".to_owned()),
                logo_url:          Some("https://example.com/logo.png".to_owned()),
                short_description: Some("A great company".to_owned()),
            }),
        }
    }

    #[test]
    fn hit_to_raw_job_maps_basic_fields() {
        let hit = make_hit();
        let raw: RawJob = hit.into();

        assert_eq!(raw.source_job_id, "3313");
        assert_eq!(raw.source_name, JAPANDEV_SOURCE_NAME);
        assert_eq!(raw.title.as_deref(), Some("Senior Rust Engineer"));
        assert_eq!(raw.company.as_deref(), Some("Example Corp"));
        assert_eq!(raw.location.as_deref(), Some("Tokyo"));
    }

    #[test]
    fn hit_to_raw_job_constructs_url_from_slug() {
        let hit = make_hit();
        let raw: RawJob = hit.into();

        assert_eq!(
            raw.url.as_deref(),
            Some("https://japan-dev.com/jobs/example-corp-senior-rust-engineer")
        );
    }

    #[test]
    fn hit_to_raw_job_salary_and_currency() {
        let hit = make_hit();
        let raw: RawJob = hit.into();

        assert_eq!(raw.salary_min, Some(8_000_000));
        assert_eq!(raw.salary_max, Some(12_000_000));
        assert_eq!(raw.salary_currency.as_deref(), Some("JPY"));
    }

    #[test]
    fn hit_to_raw_job_parses_published_at() {
        let hit = make_hit();
        let raw: RawJob = hit.into();

        let posted = raw.posted_at.expect("posted_at should be set");
        assert_eq!(posted.to_string(), "2026-01-15T00:00:00Z");
    }

    #[test]
    fn hit_to_raw_job_maps_tags() {
        let hit = make_hit();
        let raw: RawJob = hit.into();

        assert_eq!(raw.tags, vec!["Rust", "Kubernetes"]);
    }

    #[test]
    fn hit_to_raw_job_combines_description_sections() {
        let hit = make_hit();
        let raw: RawJob = hit.into();

        let desc = raw.description.expect("description should be set");
        assert!(desc.contains("## Introduction"));
        assert!(desc.contains("Join our team!"));
        assert!(desc.contains("## Details"));
        assert!(desc.contains("Work on our backend."));
        assert!(desc.contains("## Requirements"));
        assert!(desc.contains("5+ years Rust."));
        assert!(desc.contains("## Benefits"));
        assert!(desc.contains("Remote friendly."));
    }

    #[test]
    fn hit_to_raw_job_stores_raw_data() {
        let hit = make_hit();
        let raw: RawJob = hit.into();

        let data = raw.raw_data.expect("raw_data should be set");
        assert_eq!(data.get("id").and_then(|v| v.as_str()), Some("3313"));
        assert_eq!(
            data.get("japanese_level").and_then(|v| v.as_str()),
            Some("Business")
        );
        assert_eq!(
            data.get("sponsors_visas").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn hit_to_raw_job_falls_back_to_company_sub_object() {
        let mut hit = make_hit();
        hit.company_name = None; // Clear top-level company_name
        let raw: RawJob = hit.into();

        assert_eq!(raw.company.as_deref(), Some("Example Corp"));
    }

    #[test]
    fn hit_to_raw_job_handles_missing_optional_fields() {
        let hit = JapanDevHit {
            id:                 "999".to_owned(),
            title:              "Backend Dev".to_owned(),
            company_name:       None,
            location:           None,
            slug:               None,
            salary_min:         None,
            salary_max:         None,
            skill_names:        vec![],
            published_at:       None,
            details:            None,
            intro:              None,
            requirements:       None,
            benefits:           None,
            japanese_level:     None,
            english_level:      None,
            remote_level:       None,
            seniority_level:    None,
            employment_type:    None,
            candidate_location: None,
            sponsors_visas:     None,
            company:            None,
        };
        let raw: RawJob = hit.into();

        assert_eq!(raw.source_job_id, "999");
        assert_eq!(raw.title.as_deref(), Some("Backend Dev"));
        assert!(raw.company.is_none());
        assert!(raw.location.is_none());
        assert!(raw.url.is_none());
        assert!(raw.salary_min.is_none());
        assert!(raw.salary_max.is_none());
        assert_eq!(raw.salary_currency.as_deref(), Some("JPY"));
        assert!(raw.tags.is_empty());
        assert!(raw.posted_at.is_none());
        assert!(raw.description.is_none());
    }

    #[test]
    fn build_description_combines_sections() {
        let desc = build_description(
            Some("Intro text"),
            Some("Details text"),
            Some("Requirements text"),
            Some("Benefits text"),
        );
        let desc = desc.expect("should produce a description");
        assert!(desc.contains("## Introduction\n\nIntro text"));
        assert!(desc.contains("## Details\n\nDetails text"));
        assert!(desc.contains("## Requirements\n\nRequirements text"));
        assert!(desc.contains("## Benefits\n\nBenefits text"));
    }

    #[test]
    fn build_description_skips_empty_sections() {
        let desc = build_description(None, Some("Details only"), Some("   "), None);
        let desc = desc.expect("should produce a description");
        assert!(!desc.contains("Introduction"));
        assert!(desc.contains("## Details\n\nDetails only"));
        assert!(!desc.contains("Requirements"));
        assert!(!desc.contains("Benefits"));
    }

    #[test]
    fn build_description_returns_none_when_all_empty() {
        assert!(build_description(None, None, None, None).is_none());
        assert!(build_description(Some(""), Some("  "), None, None).is_none());
    }

    #[test]
    fn parse_published_at_iso_datetime() {
        let ts = parse_published_at("2026-01-15T00:00:00.000Z").expect("should parse");
        assert_eq!(ts.to_string(), "2026-01-15T00:00:00Z");
    }

    #[test]
    fn parse_published_at_date_only() {
        let ts = parse_published_at("2026-01-15").expect("should parse");
        assert_eq!(ts.to_string(), "2026-01-15T00:00:00Z");
    }

    #[test]
    fn parse_published_at_invalid() {
        assert!(parse_published_at("not-a-date").is_none());
        assert!(parse_published_at("").is_none());
    }

    #[test]
    fn default_config_values() {
        let cfg = JapanDevConfig::default();
        assert_eq!(cfg.base_url, "https://meili.japan-dev.com");
        assert!(!cfg.api_key.is_empty());
        assert_eq!(cfg.default_limit, 60);
    }
}
