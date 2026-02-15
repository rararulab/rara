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

//! Storage abstraction for local memory indexing.

use crate::manager::{ChunkDetail, MemoryResult};

/// File-level index metadata.
#[derive(Debug, Clone)]
pub struct IndexedFileMeta {
    /// Workspace-relative path of the source markdown file.
    pub path: String,
    /// SHA-256 digest of file bytes.
    pub hash: String,
    /// Last modification timestamp (unix seconds).
    pub mtime: i64,
    /// File size in bytes.
    pub size: i64,
}

/// Input payload for one memory chunk.
#[derive(Debug, Clone)]
pub struct ChunkInput {
    /// Stable per-file chunk sequence number.
    pub chunk_index: i64,
    /// Raw text content of this chunk.
    pub content:     String,
}

/// Persistence contract for memory index backends.
pub trait MemoryStore: Send + Sync {
    /// Ensure all required tables/indexes exist.
    fn ensure_schema(&self) -> MemoryResult<()>;

    /// List currently indexed files with metadata used for incremental sync.
    fn list_files(&self) -> MemoryResult<Vec<IndexedFileMeta>>;

    /// Replace all chunks for a file with the given payload.
    fn upsert_file_chunks(
        &self,
        path: &str,
        hash: &str,
        mtime: i64,
        size: i64,
        chunks: &[ChunkInput],
    ) -> MemoryResult<()>;

    /// Delete file entries (and cascaded chunks) for the given paths.
    fn delete_files(&self, paths: &[String]) -> MemoryResult<()>;

    /// Execute keyword-only retrieval.
    fn keyword_search(&self, query: &str, limit: usize) -> MemoryResult<Vec<MemorySearchRow>>;

    /// Fetch full chunk payload by identifier.
    fn get_chunk(&self, chunk_id: i64) -> MemoryResult<Option<ChunkDetail>>;
}

/// Low-level search row returned by store backend.
#[derive(Debug, Clone)]
pub struct MemorySearchRow {
    /// Unique chunk identifier.
    pub chunk_id:    i64,
    /// Source path of the chunk.
    pub path:        String,
    /// Per-file chunk index.
    pub chunk_index: i64,
    /// Chunk text content.
    pub content:     String,
    /// Backend-native keyword score.
    pub score:       f64,
}
