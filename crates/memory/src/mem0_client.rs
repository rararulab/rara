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

//! REST client for the [mem0](https://github.com/mem0ai/mem0) service.
//!
//! mem0 is the **state layer** of the memory system. It provides:
//!
//! - **Automatic fact extraction** — send a conversation and mem0 uses an LLM
//!   to extract structured facts (e.g. "user prefers Rust", "user lives in
//!   Shanghai").
//! - **Auto-deduplication** — when new facts overlap with existing ones, mem0
//!   compares via vector similarity + LLM judgment and emits ADD / UPDATE /
//!   DELETE / NOOP events.
//! - **Semantic search** — retrieve facts by meaning rather than exact keywords.
//!
//! ## API Reference
//!
//! | Method          | HTTP                         | Purpose                              |
//! |-----------------|------------------------------|--------------------------------------|
//! | `add_memories`  | `POST /v1/memories/`         | Extract facts from a conversation    |
//! | `search`        | `POST /v1/memories/search/`  | Semantic similarity search           |
//! | `get`           | `GET /v1/memories/{id}/`     | Retrieve a single memory by ID       |
//! | `delete`        | `DELETE /v1/memories/{id}/`   | Delete a single memory               |
//!
//! ## Deployment
//!
//! mem0 is deployed as a Docker container (`mem0/mem0-api-server`) and uses
//! ChromaDB as its vector storage backend. See `deploy/docker-compose/` and
//! `deploy/helm/rara-infra/` for deployment configurations.

use serde::{Deserialize, Serialize};

use crate::error::{HttpSnafu, Mem0Snafu, MemoryResult};
use snafu::ResultExt;

/// Client for the mem0 REST API.
///
/// Wraps a [`reqwest::Client`] and stores the base URL. All requests are
/// unauthenticated (the self-hosted mem0 server does not require auth by
/// default). If auth is needed in the future, add an API key field here.
pub struct Mem0Client {
    /// Shared HTTP client (connection pooling, keep-alive).
    client: reqwest::Client,
    /// Base URL without trailing slash, e.g. `http://localhost:8080`.
    base_url: String,
}

impl Mem0Client {
    /// Create a new mem0 client pointing at the given base URL.
    ///
    /// The URL should include the scheme and host, e.g. `http://localhost:8080`.
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// Add memories by sending a conversation to mem0.
    ///
    /// mem0 automatically extracts structured facts from the messages using
    /// its built-in LLM pipeline. Each extracted fact generates a
    /// [`Mem0Event`] with one of: `ADD`, `UPDATE`, `DELETE`, or `NOOP`.
    ///
    /// `POST /v1/memories/`
    pub async fn add_memories(
        &self,
        messages: Vec<Mem0Message>,
        user_id: &str,
    ) -> MemoryResult<Vec<Mem0Event>> {
        let url = format!("{}/v1/memories/", self.base_url);
        let body = serde_json::json!({
            "messages": messages,
            "user_id": user_id,
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
            return Mem0Snafu {
                message: format!("POST /v1/memories/ returned {status}: {text}"),
            }
            .fail();
        }

        let result: Mem0AddResponse = resp.json().await.context(HttpSnafu)?;
        Ok(result.results)
    }

    /// Search memories by semantic similarity.
    ///
    /// `POST /v1/memories/search/`
    pub async fn search(
        &self,
        query: &str,
        user_id: &str,
        top_k: usize,
    ) -> MemoryResult<Vec<Mem0Memory>> {
        let url = format!("{}/v1/memories/search/", self.base_url);
        let body = serde_json::json!({
            "query": query,
            "user_id": user_id,
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
            return Mem0Snafu {
                message: format!("POST /v1/memories/search/ returned {status}: {text}"),
            }
            .fail();
        }

        let results: Vec<Mem0Memory> = resp.json().await.context(HttpSnafu)?;
        Ok(results)
    }

    /// Get a single memory by ID.
    ///
    /// `GET /v1/memories/{id}/`
    pub async fn get(&self, id: &str) -> MemoryResult<Mem0Memory> {
        let url = format!("{}/v1/memories/{id}/", self.base_url);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Mem0Snafu {
                message: format!("GET /v1/memories/{id}/ returned {status}: {text}"),
            }
            .fail();
        }

        let memory: Mem0Memory = resp.json().await.context(HttpSnafu)?;
        Ok(memory)
    }

    /// Delete a single memory by ID.
    ///
    /// `DELETE /v1/memories/{id}/`
    pub async fn delete(&self, id: &str) -> MemoryResult<()> {
        let url = format!("{}/v1/memories/{id}/", self.base_url);

        let resp = self
            .client
            .delete(&url)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Mem0Snafu {
                message: format!("DELETE /v1/memories/{id}/ returned {status}: {text}"),
            }
            .fail();
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single message in a conversation sent to mem0.
///
/// Follows the OpenAI chat message format (`role` + `content`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mem0Message {
    /// Message role: `"user"`, `"assistant"`, or `"system"`.
    pub role: String,
    /// The message text content.
    pub content: String,
}

/// An event returned from the `add_memories` endpoint.
///
/// Each event describes what mem0 did with an extracted fact:
/// - `"ADD"` — a new fact was stored.
/// - `"UPDATE"` — an existing fact was refined with new information.
/// - `"DELETE"` — a fact was removed (contradicted by new info).
/// - `"NOOP"` — the fact already existed unchanged.
#[derive(Debug, Clone, Deserialize)]
pub struct Mem0Event {
    /// The memory ID (UUID) that was created or affected.
    pub id: String,
    /// Event type: `"ADD"`, `"UPDATE"`, `"DELETE"`, or `"NOOP"`.
    pub event: String,
    /// The fact text that was processed.
    pub data: Mem0EventData,
}

/// Data payload within a [`Mem0Event`].
#[derive(Debug, Clone, Deserialize)]
pub struct Mem0EventData {
    /// The extracted or updated fact text.
    pub memory: String,
}

/// A stored memory record from mem0.
///
/// Returned by both `search` (with `score`) and `get` (without `score`).
#[derive(Debug, Clone, Deserialize)]
pub struct Mem0Memory {
    /// Unique memory identifier (UUID).
    pub id: String,
    /// The stored fact text (e.g. `"User prefers Rust for backend work"`).
    pub memory: String,
    /// The user this memory belongs to.
    pub user_id: Option<String>,
    /// Semantic similarity score (0.0–1.0). Present only in search results.
    pub score: Option<f64>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
}

/// Internal response wrapper for `POST /v1/memories/`.
#[derive(Debug, Deserialize)]
struct Mem0AddResponse {
    results: Vec<Mem0Event>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MemoryError;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn add_memories_success() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "results": [{
                "id": "abc",
                "event": "ADD",
                "data": {"memory": "user likes rust"}
            }]
        });

        Mock::given(method("POST"))
            .and(path("/v1/memories/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        let msgs = vec![Mem0Message {
            role: "user".into(),
            content: "I like rust".into(),
        }];
        let events = client.add_memories(msgs, "u1").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "ADD");
        assert_eq!(events[0].id, "abc");
        assert_eq!(events[0].data.memory, "user likes rust");
    }

    #[tokio::test]
    async fn add_memories_server_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        let msgs = vec![Mem0Message {
            role: "user".into(),
            content: "hello".into(),
        }];
        let err = client.add_memories(msgs, "u1").await.unwrap_err();
        assert!(matches!(err, MemoryError::Mem0 { .. }));
    }

    #[tokio::test]
    async fn add_memories_sends_correct_body() {
        let server = MockServer::start().await;

        let expected_body = serde_json::json!({
            "messages": [{"role": "user", "content": "I like rust"}],
            "user_id": "u1",
        });

        Mock::given(method("POST"))
            .and(path("/v1/memories/"))
            .and(body_json(&expected_body))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(&serde_json::json!({"results": []})),
            )
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        let msgs = vec![Mem0Message {
            role: "user".into(),
            content: "I like rust".into(),
        }];
        let events = client.add_memories(msgs, "u1").await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn search_success() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!([{
            "id": "m1",
            "memory": "fact",
            "user_id": "u1",
            "score": 0.9,
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z"
        }]);

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        let results = client.search("rust", "u1", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "m1");
        assert_eq!(results[0].memory, "fact");
        assert!((results[0].score.unwrap() - 0.9).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn search_empty() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&serde_json::json!([])))
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        let results = client.search("nothing", "u1", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn search_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(ResponseTemplate::new(422).set_body_string("validation error"))
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        let err = client.search("q", "u1", 10).await.unwrap_err();
        assert!(matches!(err, MemoryError::Mem0 { .. }));
    }

    #[tokio::test]
    async fn get_success() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "m1",
            "memory": "user likes rust",
            "user_id": "u1",
            "score": null,
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z"
        });

        Mock::given(method("GET"))
            .and(path("/v1/memories/m1/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        let mem = client.get("m1").await.unwrap();
        assert_eq!(mem.id, "m1");
        assert_eq!(mem.memory, "user likes rust");
    }

    #[tokio::test]
    async fn get_not_found() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/memories/missing/"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        let err = client.get("missing").await.unwrap_err();
        assert!(matches!(err, MemoryError::Mem0 { .. }));
    }

    #[tokio::test]
    async fn delete_success() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/v1/memories/m1/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        client.delete("m1").await.unwrap();
    }

    #[tokio::test]
    async fn delete_not_found() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/v1/memories/missing/"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let client = Mem0Client::new(server.uri());
        let err = client.delete("missing").await.unwrap_err();
        assert!(matches!(err, MemoryError::Mem0 { .. }));
    }

    #[tokio::test]
    async fn trailing_slash_stripped() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&serde_json::json!([])))
            .mount(&server)
            .await;

        // Pass URI with trailing slash to verify it doesn't produce "//v1/..."
        let client = Mem0Client::new(format!("{}/", server.uri()));
        let results = client.search("test", "u1", 5).await.unwrap();
        assert!(results.is_empty());
    }
}
