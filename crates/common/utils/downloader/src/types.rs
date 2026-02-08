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

use std::path::PathBuf;

use bon::Builder;
use jiff::Span;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

/// A request to download a file
#[derive(Debug, Clone, Builder)]
pub struct DownloadRequest {
    /// URL to download from
    pub url:         String,
    /// Path where the downloaded file should be saved
    pub output_path: PathBuf,
}

/// Result of a successful download
#[derive(Debug, Clone)]
pub struct DownloadResult {
    /// Path where the file was saved
    pub path:       PathBuf,
    /// Size of the downloaded file in bytes
    pub size:       u64,
    /// SHA256 hash of the file (lowercase hex)
    pub sha256:     String,
    /// Whether the file was served from cache
    pub from_cache: bool,
    /// Total duration of the download operation
    pub duration:   Span,
}

/// Status of a chunk download
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum ChunkStatus {
    /// Chunk has not been downloaded yet
    Pending,
    /// Chunk has been successfully downloaded
    Completed,
    /// Chunk download failed
    Failed,
}

/// State of a single chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkState {
    /// Index of this chunk (0-based)
    pub index:       usize,
    /// Start byte position (inclusive)
    pub start:       u64,
    /// End byte position (inclusive)
    pub end:         u64,
    /// Current status of the chunk
    pub status:      ChunkStatus,
    /// Path to the temporary file for this chunk
    pub temp_file:   PathBuf,
    /// Number of retry attempts made
    #[serde(default)]
    pub retry_count: usize,
}

/// Persistent download state for resume support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadState {
    /// URL being downloaded
    pub url:             String,
    /// Optional SHA256 checksum from server (if provided in HTTP headers)
    pub server_checksum: Option<String>,
    /// Total file size in bytes
    pub file_size:       u64,
    /// Total number of chunks
    pub total_chunks:    usize,
    /// Size of each chunk (last chunk may be smaller)
    pub chunk_size:      u64,
    /// State of each chunk
    pub chunks:          Vec<ChunkState>,
    /// Unix timestamp when download was created
    pub created_at:      i64,
    /// Unix timestamp when state was last updated
    pub updated_at:      i64,
}

/// Metadata stored in cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    /// Original URL
    pub url:           String,
    /// SHA256 hash of the cached file
    pub sha256:        String,
    /// Size of the cached file
    pub file_size:     u64,
    /// Unix timestamp when file was cached
    pub downloaded_at: i64,
}
