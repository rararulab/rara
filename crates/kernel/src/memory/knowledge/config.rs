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

//! Configuration for the Knowledge Layer.

use bon::Builder;
use serde::Deserialize;

/// Configuration for the Knowledge Layer.
///
/// Loaded from `memory.knowledge` section in config.yaml.
/// All fields are required — no hardcoded defaults.
#[derive(Debug, Clone, Builder, Deserialize)]
pub struct KnowledgeConfig {
    /// Whether the knowledge layer is active.
    pub enabled: bool,
    /// OpenAI embedding model name (e.g. "text-embedding-3-small").
    pub embedding_model: String,
    /// Embedding vector dimensions (e.g. 1536).
    pub embedding_dimensions: usize,
    /// Number of top-k results from vector search.
    pub search_top_k: usize,
    /// Cosine similarity threshold for deduplication (0.0–1.0).
    pub similarity_threshold: f32,
    /// LLM model name for memory extraction (e.g. "haiku").
    pub extractor_model: String,
}
