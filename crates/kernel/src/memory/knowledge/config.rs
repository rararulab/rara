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

/// Static tuning parameters for the Knowledge Layer.
///
/// Model names live in runtime settings; this struct holds only the
/// numeric/threshold parameters populated by `boot()`.
#[derive(Debug, Clone, Builder)]
pub struct KnowledgeConfig {
    /// Embedding vector dimensions (e.g. 1536).
    pub embedding_dimensions: usize,
    /// Number of top-k results from vector search.
    pub search_top_k:         usize,
    /// Cosine similarity threshold for deduplication (0.0–1.0).
    pub similarity_threshold: f32,
}
