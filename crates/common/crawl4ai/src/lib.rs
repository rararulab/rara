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
//! converting web pages to markdown.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu, ensure};
use tracing::instrument;
use validator::Validate;

/// Errors produced by [`Crawl4AiClient`].
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Crawl4AiError {
    #[snafu(display("invalid crawl URL: {url}: {source}"))]
    InvalidUrl {
        url:    String,
        source: validator::ValidationErrors,
    },

    #[snafu(display("crawl request failed for URL {url}: {source}"))]
    HttpRequest {
        url:    String,
        source: reqwest::Error,
    },

    #[snafu(display("crawl returned HTTP {status} for URL {url}: {body}"))]
    HttpStatus {
        url:    String,
        status: reqwest::StatusCode,
        body:   String,
    },

    #[snafu(display("failed to parse crawl response for URL {url}: {source}"))]
    ParseResponse {
        url:    String,
        source: reqwest::Error,
    },

    #[snafu(display("crawl remote error for URL {url}: {message}"))]
    RemoteError { url: String, message: String },

    #[snafu(display("crawl returned empty markdown for URL {url}"))]
    EmptyMarkdown { url: String },
}

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
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build reqwest client for Crawl4AI");
        Self::with_client(base_url, client)
    }

    /// Create a client with a caller-provided reqwest client.
    pub fn with_client(base_url: &str, client: reqwest::Client) -> Self {
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// `crawl_md` a single URL and return extracted markdown content.
    #[instrument(skip(self), fields(url = %url))]
    pub async fn crawl_md(&self, url: &str) -> Result<String, Crawl4AiError> {
        #[derive(Debug, Serialize, Validate)]
        struct MdRequest {
            #[validate(url)]
            url: String,
        }

        #[derive(Debug, Deserialize)]
        struct MdResponse {
            #[allow(dead_code)]
            url:           String,
            success:       bool,
            markdown:      Option<String>,
            error_message: Option<String>,
        }

        let url = url.to_owned();
        let body = MdRequest { url: url.clone() };
        body.validate()
            .context(InvalidUrlSnafu { url: url.clone() })?;

        let endpoint = format!("{}/md", self.base_url);

        let resp = self
            .client
            .post(&endpoint)
            .json(&body)
            .send()
            .await
            .context(HttpRequestSnafu { url: url.clone() })?;

        let status = resp.status();
        ensure!(
            status.is_success(),
            HttpStatusSnafu {
                url: url.clone(),
                status,
                body: String::new()
            }
        );

        let md_resp: MdResponse = resp
            .json()
            .await
            .context(ParseResponseSnafu { url: url.clone() })?;

        let message = md_resp
            .error_message
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or("unknown crawl error")
            .to_owned();
        ensure!(
            md_resp.success,
            RemoteSnafu {
                url: url.clone(),
                message
            }
        );

        md_resp
            .markdown
            .filter(|md| !md.trim().is_empty())
            .ok_or(Crawl4AiError::EmptyMarkdown { url })
    }
}
