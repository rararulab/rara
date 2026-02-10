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

//! HTTP client for the [Crawl4AI](https://github.com/unclecode/crawl4ai) service.
//!
//! Crawl4AI runs as a local Docker container and exposes a REST API for
//! converting web pages to clean markdown.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::error::SavedJobError;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct CrawlRequest {
    urls:               Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    word_count_threshold: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CrawlResponse {
    success: bool,
    results: Vec<CrawlResult>,
}

#[derive(Debug, Deserialize)]
struct CrawlResult {
    #[allow(dead_code)]
    url:           String,
    success:       bool,
    markdown:      Option<String>,
    error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// HTTP client for the Crawl4AI service.
#[derive(Clone)]
pub struct Crawl4AiClient {
    client:   reqwest::Client,
    base_url: String,
}

impl Crawl4AiClient {
    /// Create a new client pointing to the given Crawl4AI base URL.
    ///
    /// Defaults to `http://localhost:11235` if not specified.
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build reqwest client for Crawl4AI");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// Crawl a single URL and return the extracted markdown content.
    #[instrument(skip(self), fields(url = %url))]
    pub async fn crawl(&self, url: &str) -> Result<String, SavedJobError> {
        let endpoint = format!("{}/crawl", self.base_url);

        let body = CrawlRequest {
            urls:                 vec![url.to_owned()],
            word_count_threshold: Some(10),
        };

        let resp = self
            .client
            .post(&endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| SavedJobError::CrawlError {
                url:     url.to_owned(),
                message: format!("HTTP request failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(SavedJobError::CrawlError {
                url:     url.to_owned(),
                message: format!("Crawl4AI returned HTTP {status}: {body_text}"),
            });
        }

        let crawl_resp: CrawlResponse =
            resp.json().await.map_err(|e| SavedJobError::CrawlError {
                url:     url.to_owned(),
                message: format!("failed to parse response: {e}"),
            })?;

        if !crawl_resp.success || crawl_resp.results.is_empty() {
            return Err(SavedJobError::CrawlError {
                url:     url.to_owned(),
                message: "Crawl4AI returned no results".to_owned(),
            });
        }

        let result = &crawl_resp.results[0];
        if !result.success {
            return Err(SavedJobError::CrawlError {
                url:     url.to_owned(),
                message: result
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "unknown crawl error".to_owned()),
            });
        }

        result
            .markdown
            .clone()
            .filter(|md| !md.trim().is_empty())
            .ok_or_else(|| SavedJobError::CrawlError {
                url:     url.to_owned(),
                message: "crawl returned empty markdown".to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_trims_trailing_slash() {
        let client = Crawl4AiClient::new("http://localhost:11235/");
        assert_eq!(client.base_url, "http://localhost:11235");
    }
}
