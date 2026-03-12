// Copyright 2025 Rararulab
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

//! ClawHub marketplace client - search and install skills from clawhub.ai.
//!
//! ClawHub is a public skill registry with vector search, versioning, and
//! moderation. This client wraps the v1 REST API.
//!
//! API reference: <https://clawhub.ai/api/v1/>
//! - Search:   `GET /api/v1/search?q=...&limit=20`
//! - Browse:   `GET /api/v1/skills?limit=20&sort=trending`
//! - Detail:   `GET /api/v1/skills/{slug}`
//! - Download: `GET /api/v1/download?slug=...`

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Maximum retry attempts for API calls (including the first try).
const MAX_RETRIES: u32 = 3;

/// Base delay in milliseconds for exponential backoff.
const BASE_DELAY_MS: u64 = 1_500;

/// Maximum delay cap in milliseconds.
const MAX_DELAY_MS: u64 = 15_000;

/// Default ClawHub API base URL.
const DEFAULT_BASE_URL: &str = "https://clawhub.ai/api/v1";

/// Client for the ClawHub marketplace (clawhub.ai).
pub struct ClawhubClient {
    base_url: String,
    client:   reqwest::Client,
}

impl ClawhubClient {
    /// Create a new ClawHub client with default settings.
    pub fn new() -> Self {
        Self::with_url(DEFAULT_BASE_URL)
    }

    /// Create a ClawHub client with a custom API URL.
    pub fn with_url(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Issue a GET request with automatic retry on 429 and 5xx.
    async fn get_with_retry(
        &self,
        url: &str,
        context: &str,
    ) -> Result<reqwest::Response, crate::error::SkillError> {
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let base = BASE_DELAY_MS.saturating_mul(1u64 << attempt.min(5));
                let delay_ms = base.min(MAX_DELAY_MS);
                debug!(attempt, delay_ms, context, "retrying ClawHub request");
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }

            let result = self
                .client
                .get(url)
                .header("User-Agent", "rara-clawhub/0.1")
                .send()
                .await;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp);
                    }
                    if status.as_u16() == 429 || status.is_server_error() {
                        if attempt + 1 < MAX_RETRIES {
                            // Respect Retry-After header when present.
                            if let Some(ra) = resp
                                .headers()
                                .get("retry-after")
                                .and_then(|v| v.to_str().ok())
                                .and_then(|v| v.parse::<u64>().ok())
                            {
                                let capped = (ra * 1000).min(MAX_DELAY_MS);
                                tokio::time::sleep(std::time::Duration::from_millis(capped))
                                    .await;
                            }
                            continue;
                        }
                        return Err(crate::error::SkillError::InvalidInput {
                            message: format!(
                                "{context} returned {status} after {MAX_RETRIES} attempts"
                            ),
                        });
                    }
                    return Err(crate::error::SkillError::InvalidInput {
                        message: format!("{context} returned {status}"),
                    });
                }
                Err(e) => {
                    if attempt + 1 >= MAX_RETRIES {
                        return Err(crate::error::SkillError::InvalidInput {
                            message: format!(
                                "{context} failed after {MAX_RETRIES} attempts: {e}"
                            ),
                        });
                    }
                    warn!(attempt, context, error = %e, "ClawHub request failed, will retry");
                }
            }
        }
        unreachable!()
    }
}

// -- Search: GET /api/v1/search?q=...&limit=N --------------------------------

/// A skill entry from the search endpoint.
///
/// Search results use `results` (not `items`) and are flatter than browse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubSearchEntry {
    pub slug:         String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary:      String,
    #[serde(default)]
    pub version:      Option<String>,
    #[serde(default)]
    pub score:        f64,
    /// Unix ms timestamp.
    #[serde(default)]
    pub updated_at:   Option<i64>,
}

/// Response from `GET /api/v1/search`.
#[derive(Debug, Clone, Deserialize)]
pub struct ClawhubSearchResponse {
    pub results: Vec<ClawhubSearchEntry>,
}

// -- Browse: GET /api/v1/skills?limit=N&sort=... -----------------------------

/// Stats nested inside browse entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubStats {
    #[serde(default)]
    pub downloads:         u64,
    #[serde(default)]
    pub installs_all_time: u64,
    #[serde(default)]
    pub installs_current:  u64,
    #[serde(default)]
    pub stars:             u64,
}

/// Version info nested inside browse entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubVersionInfo {
    #[serde(default)]
    pub version:    String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub changelog:  String,
}

/// A skill entry from the browse endpoint (`GET /api/v1/skills`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubBrowseEntry {
    pub slug:           String,
    #[serde(default)]
    pub display_name:   String,
    #[serde(default)]
    pub summary:        String,
    #[serde(default)]
    pub tags:           std::collections::HashMap<String, String>,
    #[serde(default)]
    pub stats:          ClawhubStats,
    #[serde(default)]
    pub created_at:     i64,
    #[serde(default)]
    pub updated_at:     i64,
    #[serde(default)]
    pub latest_version: Option<ClawhubVersionInfo>,
}

/// Paginated response from the browse endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubBrowseResponse {
    pub items:       Vec<ClawhubBrowseEntry>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

// -- Detail: GET /api/v1/skills/{slug} ---------------------------------------

/// Owner info from the skill detail endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubOwner {
    #[serde(default)]
    pub handle:       Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// The `skill` object nested inside the detail response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubSkillInfo {
    pub slug:         String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary:      String,
    #[serde(default)]
    pub stats:        ClawhubStats,
    #[serde(default)]
    pub updated_at:   i64,
}

/// Full detail response from `GET /api/v1/skills/{slug}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubSkillDetail {
    pub skill:          ClawhubSkillInfo,
    #[serde(default)]
    pub latest_version: Option<ClawhubVersionInfo>,
    #[serde(default)]
    pub owner:          Option<ClawhubOwner>,
}

// -- Sort enum ----------------------------------------------------------------

/// Sort order for browsing skills on ClawHub.
#[derive(Debug, Clone, Copy)]
pub enum ClawhubSort {
    Trending,
    Updated,
    Downloads,
    Stars,
}

impl ClawhubSort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trending => "trending",
            Self::Updated => "updated",
            Self::Downloads => "downloads",
            Self::Stars => "stars",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_response_deserializes() {
        let json = r#"{
            "results": [{
                "score": 3.71,
                "slug": "github",
                "displayName": "Github",
                "summary": "Interact with GitHub using the gh CLI.",
                "version": "1.0.0",
                "updatedAt": 1771777539580
            }]
        }"#;
        let resp: ClawhubSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].slug, "github");
        assert!(resp.results[0].score > 3.0);
    }

    #[test]
    fn browse_response_deserializes() {
        let json = r#"{
            "items": [{
                "slug": "sonoscli",
                "displayName": "Sonoscli",
                "summary": "Control Sonos speakers.",
                "tags": {"latest": "1.0.0"},
                "stats": {
                    "downloads": 19736,
                    "installsAllTime": 455,
                    "installsCurrent": 437,
                    "stars": 15
                },
                "createdAt": 1767545381030,
                "updatedAt": 1771777535889,
                "latestVersion": {
                    "version": "1.0.0",
                    "createdAt": 1767545381030,
                    "changelog": ""
                }
            }],
            "nextCursor": null
        }"#;
        let resp: ClawhubBrowseResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].slug, "sonoscli");
        assert_eq!(resp.items[0].stats.downloads, 19736);
        assert_eq!(resp.items[0].stats.stars, 15);
    }

    #[test]
    fn detail_response_deserializes() {
        let json = r#"{
            "skill": {
                "slug": "gifgrep",
                "displayName": "GifGrep",
                "summary": "Search GIFs.",
                "stats": { "downloads": 100, "stars": 5 },
                "createdAt": 0,
                "updatedAt": 0
            },
            "latestVersion": { "version": "1.2.3", "createdAt": 0, "changelog": "fix" },
            "owner": { "handle": "steipete", "displayName": "Peter" }
        }"#;
        let detail: ClawhubSkillDetail = serde_json::from_str(json).unwrap();
        assert_eq!(detail.skill.slug, "gifgrep");
        assert_eq!(detail.latest_version.unwrap().version, "1.2.3");
        assert_eq!(detail.owner.unwrap().handle.unwrap(), "steipete");
    }
}
