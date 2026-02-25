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

//! [`JapanDevDriver`] -- job source driver for japan-dev.com.
//!
//! Fetches job listings from the JapanDev Meilisearch API, then
//! concurrently scrapes each job's detail page to extract the full
//! description from the Nuxt SSR payload.

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::{
    error::SourceError,
    types::{DiscoveryCriteria, RawJob},
};

/// Source name constant for the JapanDev driver.
pub const JAPANDEV_SOURCE_NAME: &str = "japandev";

const JAPANDEV_SITE_URL: &str = "https://japan-dev.com";

/// Configuration for the JapanDev Meilisearch API.
#[derive(Debug, Clone)]
pub struct JapanDevConfig {
    /// Meilisearch base URL.
    pub base_url:      String,
    /// Bearer token for the API (public search key).
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
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()
            .unwrap_or_default();
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
            response
                .json()
                .await
                .map_err(|e| SourceError::NonRetryable {
                    source_name: JAPANDEV_SOURCE_NAME.to_owned(),
                    message:     format!("failed to parse response JSON: {e}"),
                })?;

        let hits: Vec<JapanDevHit> = search_response
            .results
            .into_iter()
            .flat_map(|r| r.hits)
            .collect();

        debug!(hit_count = hits.len(), "JapanDev search returned hits");

        let mut raw_jobs: Vec<RawJob> = hits.into_iter().map(RawJob::from).collect();

        // Concurrently fetch descriptions from detail pages.
        self.fill_descriptions(&mut raw_jobs).await;

        Ok(raw_jobs)
    }

    /// Fetch each job's detail page concurrently and fill in missing
    /// descriptions from the Nuxt SSR `__NUXT_DATA__` payload.
    async fn fill_descriptions(&self, jobs: &mut [RawJob]) {
        let futures: Vec<_> = jobs
            .iter()
            .map(|job| {
                let client = self.client.clone();
                let url = job.url.clone();
                async move {
                    let Some(url) = url else {
                        return None;
                    };
                    match fetch_description(&client, &url).await {
                        Ok(desc) => desc,
                        Err(e) => {
                            warn!(url, error = %e, "failed to fetch JapanDev job description");
                            None
                        }
                    }
                }
            })
            .collect();

        let descriptions = futures::future::join_all(futures).await;

        for (job, desc) in jobs.iter_mut().zip(descriptions) {
            if job.description.is_none() {
                job.description = desc;
            }
        }
    }
}

/// Fetch a JapanDev job detail page and extract the description from
/// the `__NUXT_DATA__` SSR payload.
async fn fetch_description(
    client: &reqwest::Client,
    url: &str,
) -> Result<Option<String>, String> {
    let resp = client
        .get(url)
        .header("Accept", "text/html")
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let html = resp.text().await.map_err(|e| format!("body error: {e}"))?;
    Ok(extract_description_from_nuxt_data(&html))
}

/// Parse the `__NUXT_DATA__` script tag and find the longest HTML
/// string that looks like a job description.
fn extract_description_from_nuxt_data(html: &str) -> Option<String> {
    // Find: <script ... id="__NUXT_DATA__">...</script>
    let marker = "id=\"__NUXT_DATA__\">";
    let start = html.find(marker)? + marker.len();
    let end = html[start..].find("</script>")? + start;
    let json_str = &html[start..end];

    // The payload is a flat JSON array; descriptions are long HTML strings.
    let data: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;

    // Find the longest string that looks like HTML job content.
    data.iter()
        .filter_map(|v| v.as_str())
        .filter(|s| s.len() > 200 && s.contains("<p>"))
        .max_by_key(|s| s.len())
        .map(|s| strip_html_tags(s))
}

/// Minimal HTML tag stripper -- converts HTML to plain text.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut prev_was_block = false;

    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                // Peek ahead for block-level tags to insert newlines.
                // We'll handle this after closing '>'.
            }
            '>' => {
                in_tag = false;
                // Check if we just closed a block tag.
                if prev_was_block {
                    if !result.ends_with('\n') {
                        result.push('\n');
                    }
                    prev_was_block = false;
                }
            }
            _ if in_tag => {
                // Check for block-level tag names.
                if matches!(c, 'p' | 'P' | 'h' | 'H' | 'l' | 'L') {
                    prev_was_block = true;
                }
            }
            _ => result.push(c),
        }
    }

    // Decode common HTML entities.
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
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
    sponsors_visas:     Option<String>,

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

        // Construct detail URL: /jobs/{company_slug}/{job_slug}
        let company_slug = hit.company.as_ref().and_then(|c| c.slug.as_deref());
        let url = match (company_slug, hit.slug.as_deref()) {
            (Some(cs), Some(js)) => {
                Some(format!("{JAPANDEV_SITE_URL}/jobs/{cs}/{js}"))
            }
            (_, Some(js)) => Some(format!("{JAPANDEV_SITE_URL}/jobs/{js}")),
            _ => None,
        };

        // Convert salary from i64 (JPY) to i32.
        #[allow(clippy::cast_possible_truncation)]
        let salary_min = hit.salary_min.map(|v| v as i32);
        #[allow(clippy::cast_possible_truncation)]
        let salary_max = hit.salary_max.map(|v| v as i32);

        // Parse published_at into a jiff Timestamp.
        let posted_at = hit.published_at.as_deref().and_then(parse_published_at);

        // Store the full hit as raw_data.
        let raw_data = serde_json::to_value(&hit).ok();

        Self {
            source_job_id,
            source_name: JAPANDEV_SOURCE_NAME.to_owned(),
            title: Some(hit.title),
            company,
            location: hit.location,
            description: None, // Filled later by fill_descriptions()
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

/// Parse an ISO 8601 datetime string from the JapanDev API.
fn parse_published_at(date_str: &str) -> Option<jiff::Timestamp> {
    if let Ok(ts) = date_str.parse::<jiff::Timestamp>() {
        return Some(ts);
    }
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

    #[test]
    fn strip_html_tags_basic() {
        let html = "<p><strong>About</strong></p><p>We build things.</p>";
        let text = strip_html_tags(html);
        assert!(text.contains("About"));
        assert!(text.contains("We build things."));
        assert!(!text.contains('<'));
    }

    #[test]
    fn extract_description_from_nuxt_payload() {
        let html = r#"<html><script type="application/json" data-nuxt-data="nuxt-app" id="__NUXT_DATA__">["short","another","\u003cp\u003e\u003cstrong\u003eAbout the role\u003c/strong\u003e\u003c/p\u003e\u003cp\u003eWe are looking for a senior engineer to join our team and help build the next generation of our platform. You will work on distributed systems, APIs, and cloud infrastructure. Requirements include 5+ years of experience with backend development and strong communication skills.\u003c/p\u003e"]</script></html>"#;
        let desc = extract_description_from_nuxt_data(html);
        assert!(desc.is_some());
        let text = desc.unwrap();
        assert!(text.contains("About the role"));
        assert!(text.contains("senior engineer"));
        assert!(!text.contains("<p>"));
    }

    #[tokio::test]
    async fn fetch_jobs_from_real_api() {
        common_telemetry::logging::init_default_ut_logging();

        let driver = JapanDevDriver::new(JapanDevConfig::default());
        let criteria = DiscoveryCriteria::builder()
            .keywords(["rust"])
            .max_results(3u32)
            .build();
        let jobs = driver.fetch_jobs(&criteria).await.unwrap();

        println!("fetched {} jobs from JapanDev", jobs.len());
        for job in &jobs {
            let has_desc = job.description.is_some();
            println!(
                "  [{}] {} @ {} (has_desc={})",
                job.source_name,
                job.title.as_deref().unwrap_or("?"),
                job.company.as_deref().unwrap_or("?"),
                has_desc,
            );
            if let Some(desc) = &job.description {
                println!("    desc preview: {}...", &desc[..desc.len().min(120)]);
            }
        }

        assert!(!jobs.is_empty(), "expected at least one job from JapanDev");
        assert!(jobs.iter().all(|j| j.source_name == JAPANDEV_SOURCE_NAME));
    }
}
