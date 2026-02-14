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

//! Embedding providers for memory hybrid search.

use sha2::{Digest, Sha256};

use crate::manager::MemoryResult;

/// Embedding provider abstraction.
pub trait Embedder: Send + Sync {
    /// Provider identifier used in cache keys (for example `local`).
    fn provider(&self) -> &'static str;

    /// Model identifier used in cache keys.
    fn model(&self) -> &'static str;

    /// Fixed embedding dimension.
    fn dimension(&self) -> usize;

    /// Generate an embedding vector for the input text.
    fn embed(&self, text: &str) -> MemoryResult<Vec<f32>>;
}

/// Lightweight deterministic local embedder.
///
/// This is not semantic-SOTA, but provides stable vectors for local hybrid
/// retrieval and keeps the architecture ready for future external embedders.
#[derive(Debug, Clone)]
pub struct HashEmbedder {
    dim: usize,
}

impl HashEmbedder {
    /// Create a deterministic hash embedder with at least 32 dimensions.
    pub fn new(dim: usize) -> Self { Self { dim: dim.max(32) } }
}

impl Default for HashEmbedder {
    fn default() -> Self { Self::new(256) }
}

impl Embedder for HashEmbedder {
    fn provider(&self) -> &'static str { "local" }

    fn model(&self) -> &'static str { "hash-embedding-v1" }

    fn dimension(&self) -> usize { self.dim }

    fn embed(&self, text: &str) -> MemoryResult<Vec<f32>> {
        let mut vec = vec![0_f32; self.dim];
        for token in text.split_whitespace() {
            let hash = Sha256::digest(token.as_bytes());
            let idx = ((u16::from(hash[0]) << 8) | u16::from(hash[1])) as usize % self.dim;
            let sign = if hash[2] & 1 == 0 { 1.0_f32 } else { -1.0_f32 };
            vec[idx] += sign;
        }

        normalize_l2(&mut vec);
        Ok(vec)
    }
}

fn normalize_l2(values: &mut [f32]) {
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in values.iter_mut() {
            *value /= norm;
        }
    }
}
