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

use futures::StreamExt;
use sha2::{Digest, Sha256};
use snafu::{ResultExt, ensure};
use tokio::{
    fs::{self, File},
    io::AsyncWriteExt,
};

use crate::{
    error::{DownloadError, FileWriteSnafu, HttpSnafu, NetworkSnafu},
    types::DownloadRequest,
};

/// Handles single-threaded file downloads (for small files or when range is not
/// supported)
pub struct SingleThreadDownloader {
    client: reqwest::Client,
}

impl SingleThreadDownloader {
    pub const fn new(client: reqwest::Client) -> Self { Self { client } }

    /// Download file in a single thread and return (size, sha256)
    pub async fn download(
        &self,
        request: &DownloadRequest,
    ) -> Result<(u64, String), DownloadError> {
        let response = self
            .client
            .get(&request.url)
            .send()
            .await
            .context(NetworkSnafu)?;

        ensure!(
            response.status().is_success(),
            HttpSnafu {
                status: response.status().as_u16(),
                url:    &request.url,
            }
        );

        // Create parent directory if needed
        if let Some(parent) = request.output_path.parent() {
            fs::create_dir_all(parent).await.context(FileWriteSnafu)?;
        }

        // Download to a temp file first, then rename
        let temp_path = request.output_path.with_extension("download");

        // Create cleanup guard to ensure temp file is removed on error
        let cleanup_guard = TempFileCleanupGuard::new(&temp_path);

        let mut file = File::create(&temp_path).await.context(FileWriteSnafu)?;
        let mut hasher = Sha256::new();
        let mut total_size = 0u64;
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context(NetworkSnafu)?;
            file.write_all(&chunk).await.context(FileWriteSnafu)?;
            hasher.update(&chunk);
            total_size += chunk.len() as u64;
        }

        // Sync to disk
        file.sync_all().await.context(FileWriteSnafu)?;

        // Rename temp file to final path
        fs::rename(&temp_path, &request.output_path)
            .await
            .context(FileWriteSnafu)?;

        // Disarm the cleanup guard since we successfully renamed
        cleanup_guard.disarm();

        let sha256 = format!("{:x}", hasher.finalize());
        Ok((total_size, sha256))
    }
}

/// Guard that ensures temporary files are cleaned up on drop
struct TempFileCleanupGuard {
    path:  std::path::PathBuf,
    armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl TempFileCleanupGuard {
    fn new(path: &std::path::Path) -> Self {
        Self {
            path:  path.to_path_buf(),
            armed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    fn disarm(&self) {
        self.armed
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for TempFileCleanupGuard {
    fn drop(&mut self) {
        if self.armed.load(std::sync::atomic::Ordering::Acquire) {
            // Best-effort cleanup - ignore errors
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
