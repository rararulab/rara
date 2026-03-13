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

//! Embedding service — OpenAI embedding API + usearch vector index.
//!
//! Handles embedding generation via the OpenAI API and provides in-memory
//! approximate nearest-neighbor search via usearch for semantic retrieval.

use std::{path::PathBuf, sync::Mutex};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use super::config::KnowledgeConfig;

/// Manages embedding generation (OpenAI-compatible API) and vector search
/// (usearch).
pub struct EmbeddingService {
    client:          Client,
    config:          KnowledgeConfig,
    index:           Mutex<Index>,
    index_path:      PathBuf,
    api_key:         String,
    embedding_model: String,
    /// Base URL of the OpenAI-compatible provider (e.g. `https://openrouter.ai/api/v1`).
    base_url:        String,
}

#[derive(Serialize)]
struct EmbeddingRequest {
    model:      String,
    input:      Vec<String>,
    dimensions: usize,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl EmbeddingService {
    /// Create a new EmbeddingService, loading or creating the usearch index.
    pub fn new(
        config: KnowledgeConfig,
        api_key: String,
        embedding_model: String,
        base_url: String,
    ) -> anyhow::Result<Self> {
        let index_path = rara_paths::data_dir().join("knowledge/memory.usearch");
        if let Some(parent) = index_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let options = IndexOptions {
            dimensions: config.embedding_dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            ..Default::default()
        };
        let index = Index::new(&options)
            .map_err(|e| anyhow::anyhow!("failed to create usearch index: {e}"))?;

        // Load existing index from disk if available.
        if index_path.exists() {
            index
                .load(index_path.to_str().unwrap_or_default())
                .map_err(|e| anyhow::anyhow!("failed to load usearch index: {e}"))?;
            info!(
                size = index.size(),
                capacity = index.capacity(),
                "loaded usearch index from disk"
            );
        } else {
            // Reserve initial capacity.
            index
                .reserve(1024)
                .map_err(|e| anyhow::anyhow!("failed to reserve usearch capacity: {e}"))?;
        }

        Ok(Self {
            client: Client::new(),
            config,
            index: Mutex::new(index),
            index_path,
            api_key,
            embedding_model,
            base_url,
        })
    }

    /// Call the OpenAI embeddings API to generate embeddings for one or more
    /// texts.
    pub async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = EmbeddingRequest {
            model:      self.embedding_model.clone(),
            input:      texts.to_vec(),
            dimensions: self.config.embedding_dimensions,
        };

        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json::<EmbeddingResponse>()
            .await?;

        let embeddings: Vec<Vec<f32>> = response.data.into_iter().map(|d| d.embedding).collect();

        // Ensure ordering matches input ordering (OpenAI returns in order, but
        // let's be explicit).
        if embeddings.len() != texts.len() {
            anyhow::bail!(
                "embedding response count mismatch: expected {}, got {}",
                texts.len(),
                embeddings.len()
            );
        }

        debug!(count = embeddings.len(), "generated embeddings");
        Ok(embeddings)
    }

    /// Add a vector to the usearch index with the given key (memory_item id).
    pub fn add_to_index(&self, key: u64, vector: &[f32]) -> anyhow::Result<()> {
        let index = self
            .index
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;

        // Grow capacity if needed.
        if index.size() >= index.capacity() {
            let new_cap = index.capacity() * 2;
            index
                .reserve(new_cap)
                .map_err(|e| anyhow::anyhow!("failed to reserve capacity: {e}"))?;
        }

        index
            .add(key, vector)
            .map_err(|e| anyhow::anyhow!("failed to add vector: {e}"))?;
        Ok(())
    }

    /// Search the usearch index for the nearest neighbors of the given query
    /// vector.
    ///
    /// Returns `(key, distance)` pairs sorted by increasing distance.
    pub fn search(&self, query: &[f32], top_k: usize) -> anyhow::Result<Vec<(u64, f32)>> {
        let index = self
            .index
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;

        if index.size() == 0 {
            return Ok(Vec::new());
        }

        let matches = index
            .search(query, top_k)
            .map_err(|e| anyhow::anyhow!("search failed: {e}"))?;

        Ok(matches.keys.into_iter().zip(matches.distances).collect())
    }

    /// Persist the usearch index to disk.
    pub fn save_index(&self) -> anyhow::Result<()> {
        let index = self
            .index
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;

        index
            .save(self.index_path.to_str().unwrap_or_default())
            .map_err(|e| anyhow::anyhow!("failed to save usearch index: {e}"))?;

        info!(size = index.size(), "saved usearch index to disk");
        Ok(())
    }

    /// Rebuild the in-memory index from embedding blobs stored in the database.
    ///
    /// Each `(id, blob)` pair contains a memory-item id and its raw f32
    /// embedding bytes (little-endian).
    pub fn rebuild_index(&self, items: &[(i64, Vec<u8>)]) -> anyhow::Result<()> {
        let index = self
            .index
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;

        index
            .reset()
            .map_err(|e| anyhow::anyhow!("failed to reset index: {e}"))?;

        if items.is_empty() {
            return Ok(());
        }

        index
            .reserve(items.len().max(1024))
            .map_err(|e| anyhow::anyhow!("failed to reserve capacity: {e}"))?;

        for (id, blob) in items {
            let floats = blob_to_f32s(blob);
            if floats.len() != self.config.embedding_dimensions {
                warn!(
                    id,
                    expected = self.config.embedding_dimensions,
                    got = floats.len(),
                    "skipping embedding with wrong dimension count"
                );
                continue;
            }
            index
                .add(*id as u64, &floats)
                .map_err(|e| anyhow::anyhow!("failed to add vector {id}: {e}"))?;
        }

        info!(count = items.len(), "rebuilt usearch index from database");
        Ok(())
    }
}

/// Convert a `Vec<f32>` embedding to a raw byte blob (little-endian).
pub fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Convert a raw byte blob back to `Vec<f32>` (little-endian).
pub fn blob_to_f32s(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().expect("chunk is 4 bytes");
            f32::from_le_bytes(arr)
        })
        .collect()
}
