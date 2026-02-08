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

use std::{path::PathBuf, time::Duration};

use bon::Builder;
use rsketch_base::readable_size::ReadableSize;
use smart_default::SmartDefault;

/// Configuration for chunk calculation
#[derive(Debug, Clone, SmartDefault)]
pub struct ChunkingConfig {
    /// Minimum size of each chunk (default: 5MB)
    #[default(ReadableSize::mb(5))]
    pub min_chunk_size:        ReadableSize,
    /// Maximum number of chunks (default: 16)
    #[default = 16]
    pub max_chunks:            usize,
    /// Files smaller than this are downloaded without chunking (default: 16MB)
    #[default(ReadableSize::mb(16))]
    pub small_file_threshold:  ReadableSize,
    /// Files between small and medium thresholds use 2-4 chunks (default:
    /// 128MB)
    #[default(ReadableSize::mb(128))]
    pub medium_file_threshold: ReadableSize,
}

impl ChunkingConfig {
    /// Calculate the number of chunks for a file of the given size
    #[must_use]
    pub fn calculate_chunks(&self, file_size: u64) -> usize {
        let small_threshold = self.small_file_threshold.as_bytes();
        let medium_threshold = self.medium_file_threshold.as_bytes();
        let min_chunk = self.min_chunk_size.as_bytes().max(1);
        let max_chunks = self.max_chunks.max(1) as u64;

        if file_size < small_threshold {
            // Small file: no chunking
            1
        } else if file_size < medium_threshold {
            // Medium file: 2-4 chunks
            let chunks = file_size / min_chunk;
            chunks.clamp(2, 4) as usize
        } else {
            // Large file: based on size, capped at max_chunks
            let chunks = (file_size / min_chunk).max(1);
            #[allow(clippy::cast_possible_truncation)]
            let result = chunks.min(max_chunks) as usize;
            result
        }
    }
}

/// Configuration for the downloader
#[derive(Debug, Clone, SmartDefault, Builder)]
pub struct DownloaderConfig {
    /// Chunking configuration
    #[default(ChunkingConfig::default())]
    pub chunking: ChunkingConfig,

    /// Directory to store cached downloads (default: user cache dir /
    /// downloader)
    #[default(dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("downloader"))]
    pub cache_dir: PathBuf,

    /// Directory for temporary files during download
    #[default(std::env::temp_dir().join("downloader"))]
    pub temp_dir: PathBuf,

    /// Timeout for HTTP requests
    #[default(Duration::from_secs(30))]
    pub timeout: Duration,

    /// Maximum number of retries per chunk
    #[default = 3]
    pub max_retries: usize,

    /// Custom User-Agent header
    pub user_agent: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_calculation_small_file() {
        let config = ChunkingConfig::default();
        // 10MB - should be 1 chunk (below 16MB threshold)
        assert_eq!(config.calculate_chunks(10 * 1024 * 1024), 1);
    }

    #[test]
    fn test_chunk_calculation_medium_file() {
        let config = ChunkingConfig::default();
        // 50MB = 52,428,800 bytes / 5MB min_chunk_size = 10, clamped to max 4
        let chunks = config.calculate_chunks(50 * 1024 * 1024);
        assert_eq!(chunks, 4);
    }

    #[test]
    fn test_chunk_calculation_large_file() {
        let config = ChunkingConfig::default();
        // 500MB - should be capped at 16 chunks
        let chunks = config.calculate_chunks(500 * 1024 * 1024);
        assert!(chunks <= 16);
        assert!(chunks > 4);
    }
}
