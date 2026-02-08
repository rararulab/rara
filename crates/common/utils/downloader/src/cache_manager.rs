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

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use sha2::{Digest, Sha256};
use snafu::ResultExt;
use tokio::{fs, io::AsyncReadExt};

use crate::{
    error::{DownloadError, FileReadSnafu, FileWriteSnafu},
    types::CacheMetadata,
};

/// Manages file caching operations
pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    pub const fn new(cache_dir: PathBuf) -> Self { Self { cache_dir } }

    /// Check if a file exists in cache and is valid
    pub async fn get(&self, url: &str) -> Result<Option<CachedFile>, DownloadError> {
        let cache_entry = CacheEntry::new(&self.cache_dir, url);

        if !cache_entry.exists().await {
            return Ok(None);
        }

        let metadata = cache_entry.read_metadata().await?;

        // Verify URL matches (avoid hash collisions)
        if metadata.url != url {
            return Ok(None);
        }

        // Verify file integrity - recompute hash and check against stored metadata
        let actual_sha256 = Self::compute_sha256(&cache_entry.content_path()).await?;
        if actual_sha256 != metadata.sha256 {
            // Cache is corrupted, remove it
            cache_entry.remove().await;
            return Ok(None);
        }

        Ok(Some(CachedFile {
            path: cache_entry.content_path(),
            metadata,
        }))
    }

    /// Store a file in cache with computed SHA256 using hard link
    pub async fn store(
        &self,
        url: &str,
        file_path: &Path,
        sha256: &str,
    ) -> Result<(), DownloadError> {
        let cache_entry = CacheEntry::new(&self.cache_dir, url);
        cache_entry.create_dir().await?;

        // Use hard link to save disk space and time
        // Falls back to copy if hard link fails (e.g., cross-filesystem)
        let content_path = cache_entry.content_path();
        if fs::hard_link(file_path, &content_path).await.is_err() {
            fs::copy(file_path, &content_path)
                .await
                .context(FileWriteSnafu)?;
        }

        // Get file size
        let file_size = fs::metadata(&content_path)
            .await
            .context(FileReadSnafu)?
            .len();

        // Write metadata
        let metadata = CacheMetadata {
            url: url.to_string(),
            sha256: sha256.to_string(),
            file_size,
            downloaded_at: Timestamp::now().as_second(),
        };

        cache_entry.write_metadata(&metadata).await?;

        Ok(())
    }

    /// Restore a cached file to the output path using hard link
    pub async fn restore(
        &self,
        cached: &CachedFile,
        output_path: &Path,
    ) -> Result<(), DownloadError> {
        // Create parent directory if needed
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).await.context(FileWriteSnafu)?;
        }

        // Use hard link to save disk space and time
        // Falls back to copy if hard link fails (e.g., cross-filesystem)
        if fs::hard_link(&cached.path, output_path).await.is_err() {
            fs::copy(&cached.path, output_path)
                .await
                .context(FileWriteSnafu)?;
        }

        Ok(())
    }

    /// Compute SHA256 hash of a file with optimized buffered reading
    async fn compute_sha256(path: &Path) -> Result<String, DownloadError> {
        let mut file = fs::File::open(path).await.context(FileReadSnafu)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 512 * 1024]; // 512KB buffer

        loop {
            let n = file.read(&mut buffer).await.context(FileReadSnafu)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }

        Ok(format!("{:x}", hasher.finalize()))
    }
}

/// Represents a cached file entry
struct CacheEntry {
    cache_path: PathBuf,
}

impl CacheEntry {
    fn new(cache_dir: &Path, url: &str) -> Self {
        let url_hash = hash_url(url);
        let cache_path = cache_dir.join(&url_hash);
        Self { cache_path }
    }

    fn content_path(&self) -> PathBuf { self.cache_path.join("content") }

    fn metadata_path(&self) -> PathBuf { self.cache_path.join("metadata.json") }

    async fn exists(&self) -> bool {
        tokio::fs::try_exists(self.content_path())
            .await
            .unwrap_or(false)
            && tokio::fs::try_exists(self.metadata_path())
                .await
                .unwrap_or(false)
    }

    async fn create_dir(&self) -> Result<(), DownloadError> {
        fs::create_dir_all(&self.cache_path)
            .await
            .context(FileWriteSnafu)
    }

    async fn remove(&self) { let _ = fs::remove_dir_all(&self.cache_path).await; }

    async fn read_metadata(&self) -> Result<CacheMetadata, DownloadError> {
        let metadata_str = fs::read_to_string(self.metadata_path())
            .await
            .context(FileReadSnafu)?;
        serde_json::from_str(&metadata_str)
            .ok()
            .ok_or(DownloadError::StateCorrupted)
    }

    async fn write_metadata(&self, metadata: &CacheMetadata) -> Result<(), DownloadError> {
        let metadata_str = serde_json::to_string_pretty(metadata)
            .ok()
            .ok_or(DownloadError::StateCorrupted)?;
        fs::write(self.metadata_path(), metadata_str)
            .await
            .context(FileWriteSnafu)
    }
}

/// A cached file with its metadata
pub struct CachedFile {
    pub path:     PathBuf,
    pub metadata: CacheMetadata,
}

/// Hash a URL to create a cache file prefix
fn hash_url(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    format!("{:x}", hasher.finalize())
}
