// Copyright 2026 Crrow
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

//! Typed HTTP client from bot runtime to main service.
//!
//! This client intentionally reuses domain request/response models for
//! discover API to avoid payload drift between bot and main service.

use job_domain_job_source::types::DiscoveryCriteria;
pub use job_domain_job_source::types::DiscoveryJobResponse;
use reqwest::StatusCode;
use serde::Serialize;
use snafu::{ResultExt, Snafu};

/// Error model for bot -> main-service HTTP calls.
#[derive(Debug, Snafu)]
pub enum MainServiceHttpError {
    #[snafu(display("request failed: {source}"))]
    Request { source: reqwest::Error },
    #[snafu(display("main service returned status {status}: {body}"))]
    HttpStatus { status: StatusCode, body: String },
}

/// Main service HTTP client used by bot runtime.
#[derive(Clone)]
pub struct MainServiceHttpClient {
    base_url: String,
    client:   reqwest::Client,
}

impl MainServiceHttpClient {
    /// Create a client with normalized base URL.
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            client:   reqwest::Client::new(),
        }
    }

    /// Call main service discovery API.
    ///
    /// Maps directly to `POST /api/v1/jobs/discover`.
    pub async fn discover_jobs(
        &self,
        keywords: Vec<String>,
        location: Option<String>,
        max_results: u32,
    ) -> Result<Vec<DiscoveryJobResponse>, MainServiceHttpError> {
        let url = format!("{}/api/v1/jobs/discover", self.base_url);
        let req = DiscoveryCriteria {
            keywords,
            location,
            job_type: None,
            max_results: Some(max_results),
            posted_after: None,
            sites: Vec::new(),
        };

        let resp = self
            .client
            .post(url)
            .json(&req)
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        let jobs = resp
            .json::<Vec<DiscoveryJobResponse>>()
            .await
            .context(RequestSnafu)?;
        Ok(jobs)
    }

    /// Submit a raw JD text to main service for parse-and-save flow.
    ///
    /// Maps to bot internal endpoint:
    /// `POST /api/v1/internal/bot/jd-parse`.
    pub async fn submit_jd_parse(&self, text: &str) -> Result<(), MainServiceHttpError> {
        let url = format!("{}/api/v1/internal/bot/jd-parse", self.base_url);
        let resp = self
            .client
            .post(url)
            .json(&JdParseRequest {
                text: text.to_owned(),
            })
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
struct JdParseRequest {
    /// Raw JD text from telegram message.
    text: String,
}
