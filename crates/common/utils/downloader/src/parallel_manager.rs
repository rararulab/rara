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

use std::sync::Arc;

use jiff::Timestamp;
use tokio::sync::Mutex;

use crate::{
    chunk_downloader::ChunkDownloader,
    error::DownloadError,
    state_manager::StateManager,
    types::{ChunkState, ChunkStatus, DownloadState},
};

/// Result type for chunk download operations
type ChunkResult = Result<usize, (usize, DownloadError)>;

/// Manages parallel downloading of chunks across multiple tokio tasks
pub struct ParallelDownloadManager {
    client:      reqwest::Client,
    max_retries: usize,
}

impl ParallelDownloadManager {
    pub const fn new(client: reqwest::Client, max_retries: usize) -> Self {
        Self {
            client,
            max_retries,
        }
    }

    /// Download all pending chunks in parallel
    pub async fn download_all(
        &self,
        state: &Arc<Mutex<DownloadState>>,
        state_manager: StateManager,
    ) -> Result<(), DownloadError> {
        let (url, pending_chunks) = self.get_pending_chunks(state).await;

        if pending_chunks.is_empty() {
            return Ok(());
        }

        let handles = self.spawn_workers(url, pending_chunks, Arc::clone(state), state_manager);
        self.collect_results(handles).await?;

        Ok(())
    }

    /// Get list of chunks that need to be downloaded
    async fn get_pending_chunks(
        &self,
        state: &Arc<Mutex<DownloadState>>,
    ) -> (String, Vec<ChunkState>) {
        let s = state.lock().await;
        let url = s.url.clone();
        let chunks = s
            .chunks
            .iter()
            .filter(|c| c.status != ChunkStatus::Completed)
            .cloned()
            .collect();
        (url, chunks)
    }

    /// Spawn worker tasks for each chunk
    fn spawn_workers(
        &self,
        url: String,
        chunks: Vec<ChunkState>,
        state: Arc<Mutex<DownloadState>>,
        state_manager: StateManager,
    ) -> Vec<tokio::task::JoinHandle<ChunkResult>> {
        chunks
            .into_iter()
            .map(|chunk| {
                let downloader =
                    ChunkDownloader::new(self.client.clone(), url.clone(), self.max_retries);
                let state = Arc::clone(&state);
                let state_manager = state_manager.clone();

                tokio::spawn(async move {
                    Self::download_chunk_worker(downloader, chunk, state, state_manager).await
                })
            })
            .collect()
    }

    /// Worker function that runs in each task
    async fn download_chunk_worker(
        downloader: ChunkDownloader,
        chunk: ChunkState,
        state: Arc<Mutex<DownloadState>>,
        state_manager: StateManager,
    ) -> ChunkResult {
        let index = chunk.index;

        match downloader.download(&chunk).await {
            Ok(()) => {
                Self::mark_completed(&state, index).await;
                // Best-effort save - don't fail the download if state persistence fails
                let s = state.lock().await;
                let _ = state_manager.save(&s).await;
                Ok(index)
            }
            Err(e) => {
                Self::mark_failed(&state, index).await;
                // Best-effort save - don't fail the download if state persistence fails
                let s = state.lock().await;
                let _ = state_manager.save(&s).await;
                Err((index, e))
            }
        }
    }

    /// Mark a chunk as completed in the shared state
    async fn mark_completed(state: &Arc<Mutex<DownloadState>>, index: usize) {
        let mut s = state.lock().await;
        s.chunks[index].status = ChunkStatus::Completed;
        s.updated_at = Timestamp::now().as_second();
    }

    /// Mark a chunk as failed in the shared state
    async fn mark_failed(state: &Arc<Mutex<DownloadState>>, index: usize) {
        let mut s = state.lock().await;
        s.chunks[index].status = ChunkStatus::Failed;
        s.chunks[index].retry_count += 1;
    }

    /// Collect results from all worker tasks
    async fn collect_results(
        &self,
        handles: Vec<tokio::task::JoinHandle<ChunkResult>>,
    ) -> Result<(), DownloadError> {
        let mut errors = Vec::new();

        for handle in handles {
            match handle.await {
                Ok(Ok(_index)) => {
                    // Chunk completed successfully
                }
                Ok(Err((index, e))) => {
                    errors.push((index, e));
                }
                Err(join_error) => {
                    // Task was cancelled or panicked
                    if join_error.is_panic() {
                        return Err(DownloadError::TaskPanic {
                            reason: join_error.to_string(),
                        });
                    }
                    // Task was cancelled - treat as error
                    return Err(DownloadError::TaskCancelled);
                }
            }
        }

        if !errors.is_empty() {
            let (_index, first_error) = errors.into_iter().next().unwrap();
            return Err(first_error);
        }

        Ok(())
    }
}
