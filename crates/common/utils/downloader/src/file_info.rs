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

use snafu::{ResultExt, ensure};

use crate::error::{DownloadError, HttpSnafu, NetworkSnafu};

/// Information about a file from HEAD request
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub size:           u64,
    pub supports_range: bool,
    /// Optional SHA256 checksum from server (extracted from headers like
    /// X-Checksum-SHA256, Digest, etc.)
    pub checksum:       Option<String>,
}

/// Fetches file information from server
pub struct FileInfoFetcher {
    client: reqwest::Client,
}

impl FileInfoFetcher {
    pub const fn new(client: reqwest::Client) -> Self { Self { client } }

    /// Get file info from server using HEAD request
    pub async fn fetch(&self, url: &str) -> Result<FileInfo, DownloadError> {
        let response = self.client.head(url).send().await.context(NetworkSnafu)?;

        ensure!(
            response.status().is_success(),
            HttpSnafu {
                status: response.status().as_u16(),
                url,
            }
        );

        // Get Content-Length
        let size = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .ok_or(DownloadError::FileSizeUnknown)?;

        // Check Accept-Ranges header
        let supports_range = response
            .headers()
            .get(reqwest::header::ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("bytes"));

        // Try to extract checksum from various headers
        let checksum = Self::extract_checksum(response.headers());

        Ok(FileInfo {
            size,
            supports_range,
            checksum,
        })
    }

    /// Extract SHA256 checksum from HTTP headers
    /// Tries common header names: X-Checksum-SHA256, X-Amz-Meta-Sha256, Digest
    fn extract_checksum(headers: &reqwest::header::HeaderMap) -> Option<String> {
        // Try X-Checksum-SHA256 (common custom header)
        if let Some(value) = headers.get("x-checksum-sha256")
            && let Ok(s) = value.to_str()
        {
            return Some(s.to_lowercase());
        }

        // Try X-Amz-Meta-Sha256 (AWS S3)
        if let Some(value) = headers.get("x-amz-meta-sha256")
            && let Ok(s) = value.to_str()
        {
            return Some(s.to_lowercase());
        }

        // Try Digest header (RFC 5843 - standard base64, RFC 3230 - legacy hex)
        if let Some(value) = headers.get(reqwest::header::HeaderName::from_static("digest"))
            && let Ok(s) = value.to_str()
        {
            // Format: "SHA-256=base64hash" or "sha-256=hexhash"
            if let Some(hash) = s
                .strip_prefix("SHA-256=")
                .or_else(|| s.strip_prefix("sha-256="))
            {
                // If it looks like hex (64 chars), use it directly
                if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some(hash.to_lowercase());
                }

                // Try Base64 decoding (RFC 5843 standard)
                use base64::Engine;
                if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(hash) {
                    // SHA-256 should be exactly 32 bytes
                    if decoded.len() == 32 {
                        return Some(decoded.iter().fold(
                            String::with_capacity(64),
                            |mut acc, b| {
                                use std::fmt::Write;
                                let _ = write!(acc, "{b:02x}");
                                acc
                            },
                        ));
                    }
                }
            }
        }

        None
    }
}
