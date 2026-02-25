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

//! REST client for the Hindsight 4-network memory service.
//!
//! Hindsight organises memories into four networks:
//! - **world** — factual knowledge about the external world
//! - **experience** — episodic memories of past interactions
//! - **opinion** — beliefs and preferences
//! - **observation** — raw sensory / perceptual records
//!
//! NOTE: The exact REST endpoints below are based on the Hindsight design
//! docs and may need adjustment once the service is deployed. Each method
//! includes a comment indicating the assumed path so that verification is
//! straightforward.

use serde::Deserialize;

use crate::error::{HindsightSnafu, HttpSnafu, MemoryResult};
use snafu::ResultExt;

/// Client for the Hindsight memory service.
pub struct HindsightClient {
    client: reqwest::Client,
    base_url: String,
    bank_id: String,
}

impl HindsightClient {
    /// Create a new Hindsight client.
    ///
    /// - `base_url`: scheme + host, e.g. `http://localhost:8100`.
    /// - `bank_id`: the memory bank identifier to operate on.
    pub fn new(base_url: String, bank_id: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            bank_id,
        }
    }

    /// Store content into the four-network memory.
    ///
    /// Assumed endpoint: `POST /api/v1/banks/{bank_id}/retain`
    pub async fn retain(&self, content: &str) -> MemoryResult<()> {
        let url = format!(
            "{}/api/v1/banks/{}/retain",
            self.base_url, self.bank_id
        );
        let body = serde_json::json!({
            "content": content,
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return HindsightSnafu {
                message: format!("POST retain returned {status}: {text}"),
            }
            .fail();
        }

        Ok(())
    }

    /// Hybrid recall across all four networks (semantic + keyword + graph + temporal).
    ///
    /// Assumed endpoint: `POST /api/v1/banks/{bank_id}/recall`
    pub async fn recall(
        &self,
        query: &str,
        top_k: usize,
    ) -> MemoryResult<Vec<HindsightMemory>> {
        let url = format!(
            "{}/api/v1/banks/{}/recall",
            self.base_url, self.bank_id
        );
        let body = serde_json::json!({
            "query": query,
            "top_k": top_k,
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return HindsightSnafu {
                message: format!("POST recall returned {status}: {text}"),
            }
            .fail();
        }

        let results: Vec<HindsightMemory> = resp.json().await.context(HttpSnafu)?;
        Ok(results)
    }

    /// Deep reasoning / reflection over the memory bank.
    ///
    /// Assumed endpoint: `POST /api/v1/banks/{bank_id}/reflect`
    pub async fn reflect(&self, query: &str) -> MemoryResult<String> {
        let url = format!(
            "{}/api/v1/banks/{}/reflect",
            self.base_url, self.bank_id
        );
        let body = serde_json::json!({
            "query": query,
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return HindsightSnafu {
                message: format!("POST reflect returned {status}: {text}"),
            }
            .fail();
        }

        let result: ReflectResponse = resp.json().await.context(HttpSnafu)?;
        Ok(result.response)
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A memory record returned from Hindsight recall.
#[derive(Debug, Clone, Deserialize)]
pub struct HindsightMemory {
    pub id: String,
    pub content: String,
    /// Network the memory belongs to: `"world"`, `"experience"`, `"opinion"`,
    /// or `"observation"`.
    pub network: String,
    pub score: f64,
}

/// Internal response wrapper for the reflect endpoint.
#[derive(Debug, Deserialize)]
struct ReflectResponse {
    response: String,
}
