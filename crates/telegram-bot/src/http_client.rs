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

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};

#[derive(Debug, Snafu)]
pub enum MainServiceHttpError {
    #[snafu(display("request failed: {source}"))]
    Request { source: reqwest::Error },
    #[snafu(display("main service returned status {status}: {body}"))]
    HttpStatus { status: StatusCode, body: String },
}

#[derive(Clone)]
pub struct MainServiceHttpClient {
    base_url: String,
    client:   reqwest::Client,
}

impl MainServiceHttpClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            client:   reqwest::Client::new(),
        }
    }

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
struct DiscoveryCriteria {
    keywords:     Vec<String>,
    location:     Option<String>,
    job_type:     Option<String>,
    max_results:  Option<u32>,
    posted_after: Option<String>,
    sites:        Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct JdParseRequest {
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryJobResponse {
    pub title:           String,
    pub company:         String,
    pub location:        Option<String>,
    pub url:             Option<String>,
    pub salary_min:      Option<i32>,
    pub salary_max:      Option<i32>,
    pub salary_currency: Option<String>,
}
