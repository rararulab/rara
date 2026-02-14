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

//! Local memory index service with SQLite FTS5 full-text search.
//!
//! Indexes markdown files from a data directory into a SQLite database with
//! FTS5 for fast keyword search. Provides incremental sync (by content hash),
//! heading-based chunking, and BM25-ranked search results.
//!
//! # Architecture
//!
//! ```text
//! Markdown files (data dir)
//!   |
//!   v
//! MemoryManager.sync()         <- incremental by content hash
//!   |
//!   v
//! SqliteMemoryStore            <- SQLite FTS5 + metadata
//!   |
//!   v
//! Agent tools: memory_search, memory_get
//! ```

mod chunker;
pub mod error;
pub mod manager;
pub mod store;
pub mod types;
