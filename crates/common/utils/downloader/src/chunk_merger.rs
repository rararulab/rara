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

use std::sync::Arc;

use sha2::{Digest, Sha256};
use snafu::ResultExt;
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
    sync::Mutex,
};

use crate::{
    error::{ChunkMissingSnafu, DownloadError, FileReadSnafu, FileWriteSnafu},
    types::{ChunkStatus, DownloadRequest, DownloadState},
};

/// Handles merging downloaded chunks into a single file
pub struct ChunkMerger;

impl ChunkMerger {
    /// Merge all completed chunks into the final output file and compute SHA256
    pub async fn merge_and_verify(
        request: &DownloadRequest,
        state: &Arc<Mutex<DownloadState>>,
    ) -> Result<(u64, String), DownloadError> {
        let chunks = {
            let s = state.lock().await;
            s.chunks.clone()
        };

        // Verify all chunks are completed
        for chunk in &chunks {
            if chunk.status != ChunkStatus::Completed {
                return ChunkMissingSnafu { index: chunk.index }.fail();
            }
        }

        // Create parent directory if needed
        if let Some(parent) = request.output_path.parent() {
            fs::create_dir_all(parent).await.context(FileWriteSnafu)?;
        }

        // Merge chunks into final file with buffered I/O
        let output_file = File::create(&request.output_path)
            .await
            .context(FileWriteSnafu)?;
        let mut buffered_writer = BufWriter::with_capacity(512 * 1024, output_file); // 512KB buffer
        let mut hasher = Sha256::new();
        let mut total_size = 0u64;
        let mut buffer = vec![0u8; 512 * 1024]; // 512KB buffer - reused across chunks

        for chunk in &chunks {
            let chunk_file = File::open(&chunk.temp_file).await.context(FileReadSnafu)?;
            let mut reader = BufReader::with_capacity(512 * 1024, chunk_file); // 512KB buffer

            // Read and write with hashing
            loop {
                let n = reader.read(&mut buffer).await.context(FileReadSnafu)?;
                if n == 0 {
                    break;
                }
                buffered_writer
                    .write_all(&buffer[..n])
                    .await
                    .context(FileWriteSnafu)?;
                hasher.update(&buffer[..n]);
                total_size += n as u64;
            }

            // Remove temp file after merging
            let _ = fs::remove_file(&chunk.temp_file).await;
        }

        // Flush buffer and sync to disk
        buffered_writer.flush().await.context(FileWriteSnafu)?;
        buffered_writer
            .get_mut()
            .sync_all()
            .await
            .context(FileWriteSnafu)?;

        let sha256 = format!("{:x}", hasher.finalize());
        Ok((total_size, sha256))
    }
}
