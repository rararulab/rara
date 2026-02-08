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

mod cache_manager;
mod chunk_downloader;
mod chunk_merger;
mod config;
mod downloader;
mod error;
mod file_info;
mod parallel_manager;
mod single_downloader;
pub(crate) mod state_manager;
mod types;

pub use config::{ChunkingConfig, DownloaderConfig};
pub use downloader::Downloader;
pub use error::DownloadError;
pub use file_info::FileInfo;
pub use state_manager::calculate_chunk_boundaries;
pub use types::{ChunkState, ChunkStatus, DownloadRequest, DownloadResult, DownloadState};
