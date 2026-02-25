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
