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

use std::fmt;

use crate::top::types::{AgentInfo, ApprovalRequest, AuditEvent, ProcessStats, SystemStats};

#[derive(Debug)]
pub enum ClientError {
    Http(reqwest::Error),
    Deserialize {
        url:    String,
        source: reqwest::Error,
    },
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Http(e) => write!(f, "HTTP error: {e}"),
            ClientError::Deserialize { url, source } => {
                write!(f, "failed to deserialize response from {url}: {source}")
            }
        }
    }
}

impl std::error::Error for ClientError {}

pub struct KernelClient {
    base_url: String,
    client:   reqwest::Client,
}

impl KernelClient {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");
        Self { base_url, client }
    }

    pub async fn stats(&self) -> Result<SystemStats, ClientError> {
        let url = format!("{}/api/v1/kernel/stats", self.base_url);
        self.get_json(&url).await
    }

    pub async fn processes(&self) -> Result<Vec<ProcessStats>, ClientError> {
        let url = format!("{}/api/v1/kernel/processes", self.base_url);
        self.get_json(&url).await
    }

    pub async fn agents(&self) -> Result<Vec<AgentInfo>, ClientError> {
        let url = format!("{}/api/v1/agents", self.base_url);
        self.get_json(&url).await
    }

    pub async fn approvals(&self) -> Result<Vec<ApprovalRequest>, ClientError> {
        let url = format!("{}/api/v1/kernel/approvals", self.base_url);
        self.get_json(&url).await
    }

    pub async fn audit(&self, limit: usize) -> Result<Vec<AuditEvent>, ClientError> {
        let url = format!("{}/api/v1/kernel/audit?limit={limit}", self.base_url);
        self.get_json(&url).await
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, ClientError> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(ClientError::Http)?;
        resp.json::<T>()
            .await
            .map_err(|source| ClientError::Deserialize {
                url: url.to_owned(),
                source,
            })
    }
}
