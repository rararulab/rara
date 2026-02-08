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

use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum DownloadError {
    #[snafu(display("Network error: {source}"))]
    Network { source: reqwest::Error },

    #[snafu(display("HTTP error {status} for URL: {url}"))]
    Http { status: u16, url: String },

    #[snafu(display("Server does not support Range requests"))]
    RangeNotSupported,

    #[snafu(display("File write error: {source}"))]
    FileWrite { source: std::io::Error },

    #[snafu(display("File read error: {source}"))]
    FileRead { source: std::io::Error },

    #[snafu(display("SHA256 mismatch: expected {expected}, got {actual}"))]
    Sha256Mismatch { expected: String, actual: String },

    #[snafu(display("Download state corrupted"))]
    StateCorrupted,

    #[snafu(display("Chunk {index} is missing"))]
    ChunkMissing { index: usize },

    #[snafu(display("Chunk {index} failed after {retries} retries: {message}"))]
    ChunkFailed {
        index:   usize,
        retries: usize,
        message: String,
    },

    #[snafu(display("Thread panicked: {message}"))]
    ThreadPanic { message: String },

    #[snafu(display("Task panicked: {reason}"))]
    TaskPanic { reason: String },

    #[snafu(display("Task was cancelled"))]
    TaskCancelled,

    #[snafu(display("State file error at {}: {message}", path.display()))]
    StateFile { path: PathBuf, message: String },

    #[snafu(display("Failed to get file size from server"))]
    FileSizeUnknown,

    #[snafu(display("Download already in progress for URL: {url}"))]
    DownloadInProgress { url: String },
}
