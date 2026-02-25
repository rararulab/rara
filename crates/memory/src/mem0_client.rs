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
//! mem0 provides automatic fact extraction from conversations and semantic
//! search over structured memories.

use serde::{Deserialize, Serialize};

use crate::error::{HttpSnafu, Mem0Snafu, MemoryResult};
use snafu::ResultExt;

/// Client for the mem0 REST API.
pub struct Mem0Client {
    client: reqwest::Client,
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
    /// mem0 automatically extracts structured facts from the messages.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mem0Message {
    pub role: String,
    pub content: String,
}

/// An event returned from the `add_memories` endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct Mem0Event {
    pub id: String,
    pub event: String,
    pub data: Mem0EventData,
}

/// Data payload within a [`Mem0Event`].
#[derive(Debug, Clone, Deserialize)]
pub struct Mem0EventData {
    pub memory: String,
}

/// A stored memory record from mem0.
#[derive(Debug, Clone, Deserialize)]
pub struct Mem0Memory {
    pub id: String,
    pub memory: String,
    pub user_id: Option<String>,
    pub score: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

/// Internal response wrapper for `POST /v1/memories/`.
#[derive(Debug, Deserialize)]
struct Mem0AddResponse {
    results: Vec<Mem0Event>,
}
