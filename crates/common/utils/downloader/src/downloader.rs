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
#![allow(clippy::needless_pass_by_value)]

use std::{io::ErrorKind, sync::Arc};

use fd_lock::RwLock;
use jiff::Timestamp;
use snafu::ResultExt;
use tokio::{fs, sync::Mutex};

use crate::{
    cache_manager::CacheManager,
    chunk_merger::ChunkMerger,
    config::DownloaderConfig,
    error::{DownloadError, DownloadInProgressSnafu, FileWriteSnafu, Sha256MismatchSnafu},
    file_info::{FileInfo, FileInfoFetcher},
    parallel_manager::ParallelDownloadManager,
    single_downloader::SingleThreadDownloader,
    state_manager::{StateManager, calculate_chunk_boundaries},
    types::{DownloadRequest, DownloadResult, DownloadState},
};

/// HTTP Downloader with OOP design - clean and maintainable architecture
///
/// Components:
/// - `CacheManager`: Handles file caching
/// - `StateManager`: Manages download state persistence for resume
/// - `FileInfoFetcher`: Fetches file metadata from server
/// - `SingleThreadDownloader`: Downloads small files in single thread
/// - `ParallelDownloadManager`: Coordinates multi-threaded chunk downloads
/// - `ChunkMerger`: Merges downloaded chunks into final file
pub struct Downloader {
    config:    DownloaderConfig,
    cache:     CacheManager,
    state:     StateManager,
    file_info: FileInfoFetcher,
    single:    SingleThreadDownloader,
    parallel:  ParallelDownloadManager,
}

impl Downloader {
    /// Create a new downloader with the given configuration
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client fails to build (should never happen with valid
    /// config)
    #[must_use]
    pub fn new(config: DownloaderConfig) -> Self {
        let client = Self::build_client(&config);
        let cache = CacheManager::new(config.cache_dir.clone());
        let state = StateManager::new(config.temp_dir.clone());
        let file_info = FileInfoFetcher::new(client.clone());
        let single = SingleThreadDownloader::new(client.clone());
        let parallel = ParallelDownloadManager::new(client, config.max_retries);

        Self {
            config,
            cache,
            state,
            file_info,
            single,
            parallel,
        }
    }

    /// Download a file from the given URL
    ///
    /// This method will:
    /// 1. Check cache if available
    /// 2. Acquire lock to prevent concurrent downloads of same URL
    /// 3. Resume from saved state if available and valid
    /// 4. Fetch file info from server (including optional checksum)
    /// 5. Choose download strategy (single or parallel)
    /// 6. Download and compute SHA256
    /// 7. Verify against server checksum if provided
    /// 8. Store in cache
    /// 9. Release lock
    pub async fn download(
        &self,
        request: DownloadRequest,
    ) -> Result<DownloadResult, DownloadError> {
        let start_time = Timestamp::now();

        // Try cache first (before acquiring lock)
        if let Some(cached) = self.cache.get(&request.url).await? {
            return self.serve_from_cache(cached, &request, start_time).await;
        }

        // Acquire lock to prevent concurrent downloads of the same URL
        // The lock is automatically released when _lock_guard is dropped
        let lock_path = self.state.lock_path(&request.url);

        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).await.context(FileWriteSnafu)?;
        }

        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&lock_path)
            .await
            .context(FileWriteSnafu)?;
        let file = file.into_std().await;
        let mut lock = RwLock::new(file);
        let _lock_guard = match lock.try_write() {
            Ok(guard) => guard,
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                return DownloadInProgressSnafu {
                    url: request.url.clone(),
                }
                .fail();
            }
            Err(err) => return Err(DownloadError::FileWrite { source: err }),
        };

        // Resume if a valid state exists, otherwise start a fresh download.
        if let Some(saved_state) = self.state.load(&request.url).await?
            && let Some(result) = self.resume_inner(&request, saved_state, start_time).await?
        {
            return Ok(result);
        }

        // Perform download (lock will be released when function returns)
        self.download_inner(&request, start_time).await
    }

    /// Inner download logic (after lock is acquired)
    async fn download_inner(
        &self,
        request: &DownloadRequest,
        start_time: Timestamp,
    ) -> Result<DownloadResult, DownloadError> {
        // Check cache again (another task might have completed it while we waited for
        // lock)
        if let Some(cached) = self.cache.get(&request.url).await? {
            return self.serve_from_cache(cached, request, start_time).await;
        }

        // Get file info from server (includes optional checksum from headers)
        let file_info = self.file_info.fetch(&request.url).await?;

        // Decide download strategy
        let num_chunks = self.calculate_strategy(&file_info);

        let (size, sha256) = if num_chunks == 1 {
            self.single.download(request).await?
        } else {
            self.download_parallel(request, &file_info, num_chunks)
                .await?
        };

        // Verify integrity against server checksum if provided
        if let Some(ref server_checksum) = file_info.checksum {
            self.verify_sha256(&sha256, server_checksum)?;
        }

        // Store in cache with computed SHA256
        self.cache
            .store(&request.url, &request.output_path, &sha256)
            .await?;

        Ok(DownloadResult {
            path: request.output_path.clone(),
            size,
            sha256,
            from_cache: false,
            duration: start_time.until(Timestamp::now()).unwrap_or_default(),
        })
    }

    /// Inner resume logic (after lock is acquired)
    async fn resume_inner(
        &self,
        request: &DownloadRequest,
        saved_state: DownloadState,
        start_time: Timestamp,
    ) -> Result<Option<DownloadResult>, DownloadError> {
        // Get current file info from server
        let file_info = self.file_info.fetch(&request.url).await?;

        // Validate state matches current server state
        if !self.state.validate(
            &saved_state,
            &request.url,
            file_info.checksum.as_deref(),
            file_info.size,
        ) {
            // State is stale, clean up
            self.state.cleanup(&request.url).await?;
            return Ok(None);
        }

        // Continue download with existing state
        let state = Arc::new(Mutex::new(saved_state));
        self.parallel
            .download_all(&state, self.state.clone())
            .await?;

        // Save final state
        {
            let s = state.lock().await;
            self.state.save(&s).await?;
        }

        // Merge chunks and verify
        let (size, sha256) = ChunkMerger::merge_and_verify(request, &state).await?;

        // Clean up state and temp files
        self.state.cleanup(&request.url).await?;

        // Verify integrity against server checksum if provided
        if let Some(ref server_checksum) = file_info.checksum {
            self.verify_sha256(&sha256, server_checksum)?;
        }

        // Store in cache
        self.cache
            .store(&request.url, &request.output_path, &sha256)
            .await?;

        Ok(Some(DownloadResult {
            path: request.output_path.clone(),
            size,
            sha256,
            from_cache: false,
            duration: start_time.until(Timestamp::now()).unwrap_or_default(),
        }))
    }

    /// Clean up all temporary files and state for a URL
    pub async fn cleanup(&self, url: &str) -> Result<(), DownloadError> {
        self.state.cleanup(url).await
    }

    // Private helper methods
    fn build_client(config: &DownloaderConfig) -> reqwest::Client {
        let mut builder = reqwest::Client::builder().timeout(config.timeout);

        if let Some(ref ua) = config.user_agent {
            builder = builder.user_agent(ua);
        }

        builder.build().expect("Failed to build HTTP client")
    }

    fn calculate_strategy(&self, file_info: &crate::file_info::FileInfo) -> usize {
        if file_info.supports_range {
            self.config.chunking.calculate_chunks(file_info.size)
        } else {
            1
        }
    }

    async fn serve_from_cache(
        &self,
        cached: crate::cache_manager::CachedFile,
        request: &DownloadRequest,
        start_time: Timestamp,
    ) -> Result<DownloadResult, DownloadError> {
        // Restore from cache to output path (using hard link if possible)
        self.cache.restore(&cached, &request.output_path).await?;

        Ok(DownloadResult {
            path:       request.output_path.clone(),
            size:       cached.metadata.file_size,
            sha256:     cached.metadata.sha256,
            from_cache: true,
            duration:   start_time.until(Timestamp::now()).unwrap_or_default(),
        })
    }

    async fn download_parallel(
        &self,
        request: &DownloadRequest,
        file_info: &FileInfo,
        num_chunks: usize,
    ) -> Result<(u64, String), DownloadError> {
        // Create download state
        let boundaries = calculate_chunk_boundaries(file_info.size, num_chunks);
        let download_state = self.state.create_state(
            &request.url,
            file_info.checksum.clone(),
            file_info.size,
            boundaries,
        );

        let state = Arc::new(Mutex::new(download_state));

        // Save initial state
        {
            let s = state.lock().await;
            self.state.save(&s).await?;
        }

        // Download all chunks
        self.parallel
            .download_all(&state, self.state.clone())
            .await?;

        // Save final state
        {
            let s = state.lock().await;
            self.state.save(&s).await?;
        }

        // Merge and verify
        let result = ChunkMerger::merge_and_verify(request, &state).await?;

        // Clean up
        self.state.cleanup(&request.url).await?;

        Ok(result)
    }

    fn verify_sha256(&self, actual: &str, expected: &str) -> Result<(), DownloadError> {
        if !actual.eq_ignore_ascii_case(expected) {
            return Sha256MismatchSnafu {
                expected: expected.to_string(),
                actual:   actual.to_string(),
            }
            .fail();
        }
        Ok(())
    }
}
