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

//! Core types for the memory index system.

use serde::{Deserialize, Serialize};

/// A markdown document indexed in the memory store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDocument {
    /// File path relative to the data directory.
    pub id:         String,
    /// First H1 heading or filename.
    pub title:      String,
    /// Full markdown content.
    pub content:    String,
    /// Semantic chunks derived from the document.
    pub chunks:     Vec<MemoryChunk>,
    /// SHA-256 hash of the file content.
    pub hash:       String,
    /// Unix timestamp of when the document was last indexed.
    pub updated_at: i64,
}

/// A chunk of a document, split by headings or paragraph boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChunk {
    /// Chunk identifier: `"{doc_id}#{chunk_index}"`.
    pub chunk_id:    String,
    /// Parent document identifier.
    pub doc_id:      String,
    /// The chunk's text content.
    pub content:     String,
    /// The section heading this chunk belongs to, if any.
    pub heading:     Option<String>,
    /// Zero-based index of this chunk within the document.
    pub chunk_index: u32,
}

/// A single search result from the FTS index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Parent document identifier.
    pub doc_id:   String,
    /// Chunk identifier that matched.
    pub chunk_id: String,
    /// Section heading of the matched chunk, if any.
    pub heading:  Option<String>,
    /// FTS-highlighted snippet of the match.
    pub snippet:  String,
    /// BM25 relevance rank (lower is more relevant).
    pub rank:     f64,
}

/// Statistics returned after a sync operation.
#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    /// Number of newly added documents.
    pub added:     usize,
    /// Number of documents whose content changed.
    pub updated:   usize,
    /// Number of documents removed (file deleted).
    pub deleted:   usize,
    /// Number of documents that were unchanged.
    pub unchanged: usize,
}
