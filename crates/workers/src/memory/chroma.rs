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

//! Optional Chroma vector index integration.

use reqwest::StatusCode;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct ChromaClient {
    base_url:       String,
    collection:     String,
    api_key:        Option<String>,
    http:           reqwest::Client,
    collection_url: String,
}

/// Chunk payload sent to Chroma upsert endpoints.
#[derive(Debug, Clone)]
pub struct ChromaChunk {
    pub id:         String,
    pub document:   String,
    pub embedding:  Vec<f32>,
    pub path:       String,
    pub chunk_index: i64,
}

/// Query hit returned from Chroma.
#[derive(Debug, Clone)]
pub struct ChromaHit {
    pub id:          String,
    pub path:        String,
    pub chunk_index: i64,
    pub document:    String,
    pub score:       f64,
}

impl ChromaClient {
    /// Build a client from explicit values.
    ///
    /// Returns `None` when `base_url` is empty.
    pub fn new(base_url: String, collection: Option<String>, api_key: Option<String>) -> Option<Self> {
        let base_url = base_url.trim().trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            return None;
        }
        let collection = collection
            .and_then(|v| {
                let t = v.trim().to_owned();
                if t.is_empty() { None } else { Some(t) }
            })
            .unwrap_or_else(|| "job-memory".to_owned());

        let api_key = api_key.and_then(|v| {
            let t = v.trim().to_owned();
            if t.is_empty() { None } else { Some(t) }
        });

        let collection_url = format!("{base_url}/api/v1/collections/{collection}");

        Some(Self {
            base_url,
            collection,
            api_key,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            collection_url,
        })
    }

    /// Build a client from environment variables.
    ///
    /// - `MEMORY_CHROMA_URL`
    /// - `MEMORY_CHROMA_COLLECTION` (optional)
    /// - `MEMORY_CHROMA_API_KEY` (optional)
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("MEMORY_CHROMA_URL").ok()?;
        let collection = std::env::var("MEMORY_CHROMA_COLLECTION")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "job-memory".to_owned());
        let api_key = std::env::var("MEMORY_CHROMA_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty());
        Self::new(base_url, Some(collection), api_key)
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.api_key {
            req.bearer_auth(token)
        } else {
            req
        }
    }

    /// Ensure the target collection exists.
    ///
    /// The implementation accepts deployment differences where collection
    /// creation may return non-success for already-existing collections.
    pub async fn ensure_collection(&self) -> Result<(), String> {
        let body = json!({"name": self.collection});
        let create_url = format!("{}/api/v1/collections", self.base_url);

        let resp = self
            .authed(self.http.post(&create_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("create collection request failed: {e}"))?;

        if resp.status().is_success() || resp.status() == StatusCode::CONFLICT {
            return Ok(());
        }

        // Some deployments return 400 for existing collection. Try get to verify.
        let get_resp = self
            .authed(self.http.get(&self.collection_url))
            .send()
            .await
            .map_err(|e| format!("verify collection request failed: {e}"))?;

        if get_resp.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "ensure collection failed: status={} body={}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            ))
        }
    }

    /// Insert or update chunks in the configured collection.
    ///
    /// Tries `/upsert` first and falls back to `/add` for compatibility with
    /// older deployments.
    pub async fn upsert_chunks(&self, chunks: &[ChromaChunk]) -> Result<(), String> {
        if chunks.is_empty() {
            return Ok(());
        }

        self.ensure_collection().await?;

        let ids = chunks.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        let embeddings = chunks
            .iter()
            .map(|c| c.embedding.clone())
            .collect::<Vec<_>>();
        let documents = chunks
            .iter()
            .map(|c| c.document.clone())
            .collect::<Vec<_>>();
        let metadatas = chunks
            .iter()
            .map(|c| {
                json!({
                    "path": c.path,
                    "chunk_index": c.chunk_index,
                })
            })
            .collect::<Vec<_>>();

        let payload = json!({
            "ids": ids,
            "embeddings": embeddings,
            "documents": documents,
            "metadatas": metadatas,
        });

        let upsert_url = format!("{}/upsert", self.collection_url);
        let add_url = format!("{}/add", self.collection_url);

        let upsert_resp = self
            .authed(self.http.post(&upsert_url))
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("upsert request failed: {e}"))?;

        if upsert_resp.status().is_success() {
            return Ok(());
        }

        let add_resp = self
            .authed(self.http.post(&add_url))
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("add request failed: {e}"))?;

        if add_resp.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "upsert failed: status={} body={}",
                add_resp.status(),
                add_resp.text().await.unwrap_or_default()
            ))
        }
    }

    /// Execute a nearest-neighbor query in Chroma.
    ///
    /// Distances are converted to a normalized score (`1 - distance`) for
    /// fusion with keyword retrieval.
    pub async fn query(&self, embedding: &[f32], n_results: usize) -> Result<Vec<ChromaHit>, String> {
        self.ensure_collection().await?;

        let payload = json!({
            "query_embeddings": [embedding],
            "n_results": n_results,
            "include": ["metadatas", "documents", "distances"],
        });

        let query_url = format!("{}/query", self.collection_url);
        let resp = self
            .authed(self.http.post(&query_url))
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("query request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!(
                "query failed: status={} body={}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            ));
        }

        let body = resp
            .json::<serde_json::Value>()
            .await
            .map_err(|e| format!("query decode failed: {e}"))?;

        let ids = body
            .get("ids")
            .and_then(|v| v.get(0))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let docs = body
            .get("documents")
            .and_then(|v| v.get(0))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let metadatas = body
            .get("metadatas")
            .and_then(|v| v.get(0))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let distances = body
            .get("distances")
            .and_then(|v| v.get(0))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut hits = Vec::new();
        for i in 0..ids.len() {
            let id = ids
                .get(i)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if id.is_empty() {
                continue;
            }

            let metadata = metadatas
                .get(i)
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default();
            let path = metadata
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let chunk_index = metadata
                .get("chunk_index")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default();

            let document = docs
                .get(i)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();

            let distance = distances
                .get(i)
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(1.0);
            let score = 1.0 - distance.max(0.0);

            hits.push(ChromaHit {
                id,
                path,
                chunk_index,
                document,
                score,
            });
        }

        Ok(hits)
    }
}
