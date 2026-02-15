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

use chromadb::{
    client::{ChromaAuthMethod, ChromaClientOptions, ChromaClient as SdkClient, ChromaTokenHeader},
    collection::{ChromaCollection, CollectionEntries, QueryOptions},
};
use serde_json::Map;

use crate::manager::{MemoryError, MemoryResult};

#[derive(Debug, Clone)]
pub struct ChromaClient {
    base_url:        String,
    collection_name: String,
    api_key:         Option<String>,
}

/// Chunk payload sent to Chroma upsert endpoints.
#[derive(Debug, Clone)]
pub struct ChromaChunk {
    pub id:         String,
    pub document:   String,
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
        let collection_name = collection
            .and_then(|v| {
                let t = v.trim().to_owned();
                if t.is_empty() { None } else { Some(t) }
            })
            .unwrap_or_else(|| "job-memory".to_owned());

        let api_key = api_key.and_then(|v| {
            let t = v.trim().to_owned();
            if t.is_empty() { None } else { Some(t) }
        });

        Some(Self {
            base_url,
            collection_name,
            api_key,
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

    /// Build SDK client options from stored config.
    fn build_options(&self) -> ChromaClientOptions {
        let auth = self
            .api_key
            .clone()
            .map_or(ChromaAuthMethod::None, |token| ChromaAuthMethod::TokenAuth {
                token,
                header: ChromaTokenHeader::Authorization,
            });

        ChromaClientOptions {
            url: Some(self.base_url.clone()),
            auth,
            ..Default::default()
        }
    }

    /// Create the SDK client and get-or-create the target collection.
    async fn get_collection(&self) -> MemoryResult<(SdkClient, ChromaCollection)> {
        let client = SdkClient::new(self.build_options())
            .await
            .map_err(|e| MemoryError::Other {
                message: format!("failed to create chroma client: {e}"),
            })?;

        let collection = client
            .get_or_create_collection(&self.collection_name, None)
            .await
            .map_err(|e| MemoryError::Other {
                message: format!("failed to get/create collection '{}': {e}", self.collection_name),
            })?;

        Ok((client, collection))
    }

    /// Insert or update chunks in the configured collection.
    pub async fn upsert_chunks(&self, chunks: &[ChromaChunk]) -> MemoryResult<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let (_client, collection) = self.get_collection().await?;

        let ids: Vec<&str> = chunks.iter().map(|c| c.id.as_str()).collect();
        let documents: Vec<&str> = chunks.iter().map(|c| c.document.as_str()).collect();
        let metadatas: Vec<Map<String, serde_json::Value>> = chunks
            .iter()
            .map(|c| {
                let mut map = Map::new();
                map.insert("path".to_owned(), serde_json::Value::String(c.path.clone()));
                map.insert(
                    "chunk_index".to_owned(),
                    serde_json::Value::Number(serde_json::Number::from(c.chunk_index)),
                );
                map
            })
            .collect();

        let entries = CollectionEntries {
            ids,
            embeddings: None, // Chroma auto-embeds via built-in model
            documents: Some(documents),
            metadatas: Some(metadatas),
        };

        collection
            .upsert(entries, None)
            .await
            .map_err(|e| MemoryError::Other {
                message: format!("chroma upsert failed: {e}"),
            })?;

        Ok(())
    }

    /// Execute a nearest-neighbor query in Chroma using server-side embeddings.
    ///
    /// Chroma embeds the query text using its built-in model (all-MiniLM-L6-v2).
    /// Distances are converted to a normalized score (`1 - distance`) for
    /// fusion with keyword retrieval.
    pub async fn query(&self, query_text: &str, n_results: usize) -> MemoryResult<Vec<ChromaHit>> {
        let (_client, collection) = self.get_collection().await?;

        let query_options = QueryOptions {
            query_texts: Some(vec![query_text]),
            query_embeddings: None, // Chroma auto-embeds the query
            n_results: Some(n_results),
            include: Some(vec!["metadatas", "documents", "distances"]),
            ..Default::default()
        };

        let result = collection
            .query(query_options, None)
            .await
            .map_err(|e| MemoryError::Other {
                message: format!("chroma query failed: {e}"),
            })?;

        let ids = result
            .ids
            .first()
            .cloned()
            .unwrap_or_default();
        let docs: Vec<String> = result
            .documents
            .as_ref()
            .and_then(|d| d.first())
            .cloned()
            .unwrap_or_default();
        let metadatas: Vec<Option<Map<String, serde_json::Value>>> = result
            .metadatas
            .as_ref()
            .and_then(|m| m.first())
            .cloned()
            .unwrap_or_default();
        let distances: Vec<f32> = result
            .distances
            .as_ref()
            .and_then(|d| d.first())
            .cloned()
            .unwrap_or_default();

        let mut hits = Vec::new();
        for i in 0..ids.len() {
            let id = &ids[i];
            if id.is_empty() {
                continue;
            }

            let metadata = metadatas.get(i).and_then(|m| m.as_ref());
            let path = metadata
                .and_then(|m| m.get("path"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let chunk_index = metadata
                .and_then(|m| m.get("chunk_index"))
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default();

            let document = docs.get(i).cloned().unwrap_or_default();

            let distance = f64::from(*distances.get(i).unwrap_or(&1.0));
            let score = 1.0 - distance.max(0.0);

            hits.push(ChromaHit {
                id: id.clone(),
                path,
                chunk_index,
                document,
                score,
            });
        }

        Ok(hits)
    }
}
