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

//! Memory index service for markdown-backed agent memory.
//!
//! # Overview
//! - Incremental indexing of Markdown files into PostgreSQL.
//! - Hybrid retrieval (Chroma vector + PG keyword) with token-overlap reranking.
//! - Server-side embeddings via Chroma (all-MiniLM-L6-v2).
//!
//! # Infrastructure
//! - **PostgreSQL** — file metadata + chunk content + full-text index.
//! - **Chroma** — vector embeddings + nearest-neighbor search.
//! - Schema managed via migrations in `crates/rara-model/migrations/`.

pub mod chroma;
pub mod manager;
pub mod reranking;
pub mod store;
pub mod store_pg;

pub use chroma::ChromaClient;
pub use manager::{ChunkDetail, MemoryManager, MemoryResult, SyncStats};
pub use store::{ChunkInput, IndexedFileMeta};
pub use store_pg::PgMemoryStore;
