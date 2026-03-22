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

//! GitHub HTTP client with authentication and retry.
//!
//! Reads `GITHUB_TOKEN` or `GH_TOKEN` from the environment for authenticated
//! requests (5 000 req/hour vs 60 unauthenticated). Retries on 429 and 5xx
//! with exponential backoff.

use snafu::ResultExt;

use crate::error::InstallSnafu;

/// Maximum retry attempts for API calls (including the first try).
const MAX_RETRIES: u32 = 3;

/// Base delay in milliseconds for exponential backoff.
const BASE_DELAY_MS: u64 = 1_500;

/// Maximum delay cap in milliseconds.
const MAX_DELAY_MS: u64 = 15_000;

/// User-Agent header value for all GitHub requests from this crate.
const USER_AGENT: &str = "rara-skills";

/// GitHub API client with optional token auth and automatic retry on
/// transient errors (429 / 5xx).
pub struct GitHubClient {
    client: reqwest::Client,
    token:  Option<String>,
}

impl Default for GitHubClient {
    fn default() -> Self { Self::new() }
}

impl GitHubClient {
    /// Create a new client. Reads `GITHUB_TOKEN` or `GH_TOKEN` from env.
    pub fn new() -> Self {
        let token = std::env::var("GITHUB_TOKEN")
            .or_else(|_| std::env::var("GH_TOKEN"))
            .ok();
        if token.is_none() {
            tracing::debug!(
                "no GITHUB_TOKEN or GH_TOKEN set — using unauthenticated GitHub API (60 req/hour \
                 limit)"
            );
        }
        Self {
            client: reqwest::Client::new(),
            token,
        }
    }

    /// Issue a GET request with auth header and retry on 429 / 5xx.
    ///
    /// `context` is a human-readable label used in error messages (e.g.
    /// "GitHub tarball download").
    pub async fn get(
        &self,
        url: &str,
        context: &str,
    ) -> Result<reqwest::Response, crate::error::SkillError> {
        let mut next_delay_ms: Option<u64> = None;

        for attempt in 0..MAX_RETRIES {
            if let Some(delay_ms) = next_delay_ms.take() {
                tracing::debug!(attempt, delay_ms, context, "retrying GitHub request");
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }

            let mut req = self
                .client
                .get(url)
                .header("User-Agent", USER_AGENT)
                .header("Accept", "application/vnd.github.v3+json");
            if let Some(ref token) = self.token {
                req = req.header("Authorization", format!("Bearer {token}"));
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp);
                    }
                    if status.as_u16() == 429 || status.is_server_error() {
                        if attempt + 1 < MAX_RETRIES {
                            let backoff = BASE_DELAY_MS
                                .saturating_mul(1u64 << (attempt + 1).min(5))
                                .min(MAX_DELAY_MS);
                            let delay = resp
                                .headers()
                                .get("retry-after")
                                .and_then(|v| v.to_str().ok())
                                .and_then(|v| v.parse::<u64>().ok())
                                .map(|secs| (secs * 1000).min(MAX_DELAY_MS))
                                .unwrap_or(backoff);
                            next_delay_ms = Some(delay);
                            continue;
                        }
                        return InstallSnafu {
                            message: format!(
                                "{context} returned {status} after {MAX_RETRIES} attempts"
                            ),
                        }
                        .fail();
                    }
                    // Non-retriable HTTP error — return HttpStatus so callers
                    // can match on the status code (e.g. 404 fallback logic).
                    return Err(crate::error::SkillError::HttpStatus {
                        status: status.as_u16(),
                        url:    url.to_string(),
                    });
                }
                Err(e) => {
                    if attempt + 1 >= MAX_RETRIES {
                        return Err(e).context(crate::error::RequestSnafu);
                    }
                    let backoff = BASE_DELAY_MS
                        .saturating_mul(1u64 << (attempt + 1).min(5))
                        .min(MAX_DELAY_MS);
                    next_delay_ms = Some(backoff);
                    tracing::warn!(attempt, context, error = %e, "GitHub request failed, will retry");
                }
            }
        }
        unreachable!("retry loop exhausted without returning")
    }
}
