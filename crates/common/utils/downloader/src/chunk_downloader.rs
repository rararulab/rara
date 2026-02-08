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

use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use futures::StreamExt;
use snafu::{ResultExt, ensure};
use tokio::{
    fs::{self, File},
    io::{AsyncWriteExt, BufWriter},
};

use crate::{
    error::{DownloadError, FileWriteSnafu, HttpSnafu, NetworkSnafu},
    types::ChunkState,
};

/// Handles downloading a single chunk with retry logic using backon
pub struct ChunkDownloader {
    client:      reqwest::Client,
    url:         String,
    max_retries: usize,
}

impl ChunkDownloader {
    pub const fn new(client: reqwest::Client, url: String, max_retries: usize) -> Self {
        Self {
            client,
            url,
            max_retries,
        }
    }

    /// Download a chunk with automatic retry and exponential backoff using
    /// backon
    pub async fn download(&self, chunk: &ChunkState) -> Result<(), DownloadError> {
        // Use backon's exponential backoff with max_times
        let backoff = ExponentialBuilder::default()
            .with_max_times(self.max_retries)
            .with_min_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(8));

        (|| self.try_download(chunk))
            .retry(backoff)
            .when(|e| !Self::is_client_error(e))
            .await
    }

    /// Single download attempt for a chunk
    async fn try_download(&self, chunk: &ChunkState) -> Result<(), DownloadError> {
        let range_header = format!("bytes={}-{}", chunk.start, chunk.end);

        let response = self
            .client
            .get(&self.url)
            .header(reqwest::header::RANGE, range_header)
            .send()
            .await
            .context(NetworkSnafu)?;

        // For ranged downloads, reject 200 OK (server ignored Range header)
        let status = response.status();
        if status == reqwest::StatusCode::OK {
            // Server ignored Range header, would corrupt chunked download
            return Err(DownloadError::RangeNotSupported);
        }
        ensure!(
            status == reqwest::StatusCode::PARTIAL_CONTENT,
            HttpSnafu {
                status: status.as_u16(),
                url:    &self.url,
            }
        );

        // Create parent directory if needed
        if let Some(parent) = chunk.temp_file.parent() {
            fs::create_dir_all(parent).await.context(FileWriteSnafu)?;
        }

        // Write to temp file with buffered writer
        let file = File::create(&chunk.temp_file)
            .await
            .context(FileWriteSnafu)?;
        let mut writer = BufWriter::with_capacity(512 * 1024, file); // 512KB buffer
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk_data = chunk_result.context(NetworkSnafu)?;
            writer
                .write_all(&chunk_data)
                .await
                .context(FileWriteSnafu)?;
        }

        // Flush buffer and sync to disk
        writer.flush().await.context(FileWriteSnafu)?;
        writer.get_mut().sync_all().await.context(FileWriteSnafu)?;

        Ok(())
    }

    /// Check if error is a client error (4xx) that shouldn't be retried
    const fn is_client_error(error: &DownloadError) -> bool {
        matches!(error, DownloadError::Http { status, .. } if *status >= 400 && *status < 500)
    }
}
