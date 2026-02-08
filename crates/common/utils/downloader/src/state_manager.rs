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

#![allow(clippy::unused_self)]
#![allow(clippy::unnecessary_wraps)]

use std::path::PathBuf;

use jiff::Timestamp;
use sha2::{Digest, Sha256};
use snafu::ResultExt;
use tokio::fs;

use crate::{
    error::{DownloadError, FileReadSnafu, FileWriteSnafu},
    types::{ChunkState, ChunkStatus, DownloadState},
};

/// Manages download state persistence for resume support
#[derive(Clone)]
pub struct StateManager {
    temp_dir: PathBuf,
}

impl StateManager {
    pub const fn new(temp_dir: PathBuf) -> Self { Self { temp_dir } }

    /// Create initial download state
    pub fn create_state(
        &self,
        url: &str,
        server_checksum: Option<String>,
        file_size: u64,
        boundaries: Vec<(u64, u64)>,
    ) -> DownloadState {
        let url_hash = hash_url(url);
        let num_chunks = boundaries.len();
        let chunk_size = if num_chunks > 0 {
            file_size / num_chunks as u64
        } else {
            file_size
        };

        let chunks: Vec<ChunkState> = boundaries
            .into_iter()
            .enumerate()
            .map(|(index, (start, end))| ChunkState {
                index,
                start,
                end,
                status: ChunkStatus::Pending,
                temp_file: self.temp_dir.join(format!("{url_hash}.part{index}")),
                retry_count: 0,
            })
            .collect();

        let now = Timestamp::now().as_second();

        DownloadState {
            url: url.to_string(),
            server_checksum,
            file_size,
            total_chunks: num_chunks,
            chunk_size,
            chunks,
            created_at: now,
            updated_at: now,
        }
    }

    /// Load download state from disk
    pub async fn load(&self, url: &str) -> Result<Option<DownloadState>, DownloadError> {
        let state_path = self.state_path(url);

        if !tokio::fs::try_exists(&state_path).await.unwrap_or(false) {
            return Ok(None);
        }

        let state_str = fs::read_to_string(&state_path)
            .await
            .context(FileReadSnafu)?;
        let state: DownloadState = serde_json::from_str(&state_str)
            .ok()
            .ok_or(DownloadError::StateCorrupted)?;

        Ok(Some(state))
    }

    /// Save download state to disk
    pub async fn save(&self, state: &DownloadState) -> Result<(), DownloadError> {
        let state_path = self.state_path(&state.url);

        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent).await.context(FileWriteSnafu)?;
        }

        let state_str = serde_json::to_string_pretty(state)
            .ok()
            .ok_or(DownloadError::StateCorrupted)?;
        fs::write(&state_path, state_str)
            .await
            .context(FileWriteSnafu)?;

        Ok(())
    }

    /// Validate that state is compatible with current download attempt
    pub fn validate(
        &self,
        state: &DownloadState,
        url: &str,
        server_checksum: Option<&str>,
        file_size: u64,
    ) -> bool {
        state.url == url
            && state.server_checksum.as_deref() == server_checksum
            && state.file_size == file_size
    }

    /// Clean up state file, temporary chunks, and lock file
    pub async fn cleanup(&self, url: &str) -> Result<(), DownloadError> {
        let state_path = self.state_path(url);
        let _ = fs::remove_file(&state_path).await;

        // Remove lock file
        let lock_path = self.lock_path(url);
        let _ = fs::remove_file(&lock_path).await;

        // Remove temporary chunk files
        let url_hash = hash_url(url);
        let pattern = format!("{url_hash}.part");

        if let Ok(mut entries) = fs::read_dir(&self.temp_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(name) = entry.file_name().to_str()
                    && name.starts_with(&pattern)
                {
                    let _ = fs::remove_file(entry.path()).await;
                }
            }
        }

        Ok(())
    }

    /// Get path to state file for a URL
    fn state_path(&self, url: &str) -> PathBuf {
        let url_hash = hash_url(url);
        self.temp_dir.join(format!("{url_hash}.state.json"))
    }

    /// Get path to lock file for a URL
    pub(crate) fn lock_path(&self, url: &str) -> PathBuf {
        let url_hash = hash_url(url);
        self.temp_dir.join(format!("{url_hash}.lock"))
    }
}

/// Hash a URL to create a state file prefix
fn hash_url(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Calculate chunk boundaries for parallel download
#[must_use]
pub fn calculate_chunk_boundaries(file_size: u64, num_chunks: usize) -> Vec<(u64, u64)> {
    // Guard against zero chunks or zero file size
    if num_chunks == 0 || file_size == 0 {
        return Vec::new();
    }

    // Clamp num_chunks to not exceed file_size to avoid zero-sized chunks
    #[allow(clippy::cast_possible_truncation)]
    let num_chunks = if (num_chunks as u64) > file_size {
        file_size as usize
    } else {
        num_chunks
    };
    let chunk_size = file_size / num_chunks as u64;
    let mut boundaries = Vec::with_capacity(num_chunks);

    for i in 0..num_chunks {
        let start = i as u64 * chunk_size;
        let end = if i == num_chunks - 1 {
            file_size - 1 // Last chunk goes to end
        } else {
            (i as u64 + 1) * chunk_size - 1
        };
        boundaries.push((start, end));
    }

    boundaries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_chunk_boundaries() {
        let boundaries = calculate_chunk_boundaries(1000, 4);
        assert_eq!(boundaries.len(), 4);
        assert_eq!(boundaries[0], (0, 249));
        assert_eq!(boundaries[1], (250, 499));
        assert_eq!(boundaries[2], (500, 749));
        assert_eq!(boundaries[3], (750, 999));
    }

    #[test]
    fn test_calculate_chunk_boundaries_single() {
        let boundaries = calculate_chunk_boundaries(1000, 1);
        assert_eq!(boundaries.len(), 1);
        assert_eq!(boundaries[0], (0, 999));
    }

    #[test]
    fn test_hash_url() {
        let hash1 = hash_url("https://example.com/file.zip");
        let hash2 = hash_url("https://example.com/file.zip");
        let hash3 = hash_url("https://example.com/other.zip");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 64); // SHA256 hex string length
    }
}
