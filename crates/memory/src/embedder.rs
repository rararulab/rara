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
//!
//! The `Embedder` trait is retained for potential future use with external
//! embedding providers. Server-side embeddings are now handled by Chroma's
//! built-in model (all-MiniLM-L6-v2).

use crate::manager::MemoryResult;

/// Embedding provider abstraction.
///
/// Currently unused — Chroma handles embeddings server-side. Retained as a
/// placeholder for future external embedding providers.
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
