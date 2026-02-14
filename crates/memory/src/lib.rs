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

//! Local memory index service for markdown-backed agent memory.
//!
//! # Overview
//! The memory subsystem provides:
//! - Incremental indexing of Markdown files.
//! - Hybrid retrieval (keyword + vector) with optional Chroma acceleration.
//! - Pluggable persistence backends (SQLite or PostgreSQL).
//! - Agent-facing tools (`memory_search`, `memory_get`) built on top of
//!   [`MemoryManager`].
//!
//! # Runtime Behavior
//! - Storage backend is selected by runtime settings (`agent.memory`).
//! - Vector retrieval can be disabled at runtime (`embeddings_enabled = false`).
//! - Chroma is optional; when unavailable, retrieval falls back to local vector
//!   search without failing the user request.

pub mod embedder;
pub mod chroma;
pub mod manager;
pub mod reranking;
pub mod search;
pub mod store;
pub mod store_pg;
pub mod store_sqlite;

pub use embedder::{Embedder, HashEmbedder};
pub use manager::{ChunkDetail, MemoryManager, MemoryResult, SyncStats};
pub use reranking::rerank_results;
pub use search::{hybrid_search, keyword_only_search};
pub use store::{ChunkInput, IndexedFileMeta, MemoryStore};
pub use store_pg::PgMemoryStore;
pub use store_sqlite::SqliteMemoryStore;
pub use chroma::ChromaClient;
