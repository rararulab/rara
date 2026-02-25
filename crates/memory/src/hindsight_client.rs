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

//! REST client for the [Hindsight](https://github.com/vectorize-io/hindsight)
//! 4-network memory service.
//!
//! Hindsight is the **learning layer** of the memory system. It organises
//! memories into four semantic networks:
//!
//! | Network         | Content                                            |
//! |-----------------|----------------------------------------------------|
//! | **world**       | Factual knowledge about the external world          |
//! | **experience**  | Episodic memories of past interactions (biography)  |
//! | **opinion**     | Beliefs, preferences, and confidence scores         |
//! | **observation** | Entity summaries and raw perceptual records         |
//!
//! ## Three Operations
//!
//! 1. **retain** — store content into the four-network model. Hindsight
//!    automatically distributes the information across the appropriate networks.
//! 2. **recall** — hybrid retrieval combining semantic search, BM25 keyword
//!    matching, graph traversal, and temporal decay.
//! 3. **reflect** — personality-conditioned deep reasoning that synthesizes
//!    an answer from information across all four networks.
//!
//! ## Scoping
//!
//! All operations are scoped to a **bank** (identified by `bank_id`). Each
//! bank is an isolated memory store — different agents or users can have
//! separate banks.
//!
//! ## Deployment
//!
//! Hindsight is deployed as `ghcr.io/vectorize-io/hindsight:latest` with a
//! dedicated pgvector-enabled PostgreSQL instance.
//!
//! ## Endpoint Stability
//!
//! **NOTE**: The REST endpoints below are based on the Hindsight design docs
//! and may need adjustment once the service is deployed. Each method includes
//! a comment indicating the assumed path so that verification is
//! straightforward.

use serde::Deserialize;

use crate::error::{HindsightSnafu, HttpSnafu, MemoryResult};
use snafu::ResultExt;

/// Client for the Hindsight memory service.
///
/// Each client is bound to a specific memory bank. All retain/recall/reflect
/// operations are scoped to that bank.
pub struct HindsightClient {
    /// Shared HTTP client (connection pooling, keep-alive).
    client: reqwest::Client,
    /// Base URL without trailing slash, e.g. `http://localhost:8888`.
    base_url: String,
    /// Memory bank identifier. Isolates memory stores for different
    /// agents or users.
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
    /// Hindsight automatically analyses the content and distributes it
    /// across the appropriate networks (world, experience, opinion,
    /// observation).
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

    /// Hybrid recall across all four networks.
    ///
    /// Combines multiple retrieval strategies: semantic similarity, BM25
    /// keyword matching, knowledge graph traversal, and temporal decay.
    /// Returns up to `top_k` results sorted by relevance.
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
    /// Unlike [`recall`](Self::recall) which returns raw memory fragments,
    /// reflect asks Hindsight to synthesize an answer by reasoning across
    /// all four networks with personality conditioning. Returns a free-form
    /// text response.
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
    /// Unique record identifier within the bank.
    pub id: String,
    /// The memory content text.
    pub content: String,
    /// Which of the four networks this memory belongs to:
    /// `"world"`, `"experience"`, `"opinion"`, or `"observation"`.
    pub network: String,
    /// Relevance score from the hybrid retrieval pipeline (higher is better).
    pub score: f64,
}

/// Internal response wrapper for the reflect endpoint.
#[derive(Debug, Deserialize)]
struct ReflectResponse {
    /// The synthesized reasoning output.
    response: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MemoryError;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn retain_success() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/retain"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = HindsightClient::new(server.uri(), "test-bank".into());
        client.retain("some content").await.unwrap();
    }

    #[tokio::test]
    async fn retain_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/retain"))
            .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
            .mount(&server)
            .await;

        let client = HindsightClient::new(server.uri(), "test-bank".into());
        let err = client.retain("content").await.unwrap_err();
        assert!(matches!(err, MemoryError::Hindsight { .. }));
    }

    #[tokio::test]
    async fn retain_uses_bank_id_in_path() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/my-custom-bank/retain"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = HindsightClient::new(server.uri(), "my-custom-bank".into());
        client.retain("content").await.unwrap();
    }

    #[tokio::test]
    async fn recall_success() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!([{
            "id": "h1",
            "content": "memory",
            "network": "world",
            "score": 0.8
        }]);

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let client = HindsightClient::new(server.uri(), "test-bank".into());
        let results = client.recall("query", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "h1");
        assert_eq!(results[0].content, "memory");
        assert_eq!(results[0].network, "world");
        assert!((results[0].score - 0.8).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn recall_empty() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&serde_json::json!([])))
            .mount(&server)
            .await;

        let client = HindsightClient::new(server.uri(), "test-bank".into());
        let results = client.recall("nothing", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn recall_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
            .mount(&server)
            .await;

        let client = HindsightClient::new(server.uri(), "test-bank".into());
        let err = client.recall("query", 10).await.unwrap_err();
        assert!(matches!(err, MemoryError::Hindsight { .. }));
    }

    #[tokio::test]
    async fn reflect_success() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "response": "synthesized answer"
        });

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/reflect"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let client = HindsightClient::new(server.uri(), "test-bank".into());
        let answer = client.reflect("deep question").await.unwrap();
        assert_eq!(answer, "synthesized answer");
    }

    #[tokio::test]
    async fn reflect_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/reflect"))
            .respond_with(ResponseTemplate::new(500).set_body_string("error"))
            .mount(&server)
            .await;

        let client = HindsightClient::new(server.uri(), "test-bank".into());
        let err = client.reflect("query").await.unwrap_err();
        assert!(matches!(err, MemoryError::Hindsight { .. }));
    }
}
