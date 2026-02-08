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

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("IO error"), context(false))]
    Io {
        source: std::io::Error,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },

    #[snafu(display("Channel send error"))]
    ChannelSend {
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("Channel receive error"))]
    ChannelRecv {
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("Corrupted message at sequence {sequence}"))]
    CorruptedMessage {
        sequence: u64,
        #[snafu(implicit)]
        loc:      snafu::Location,
    },

    #[snafu(display("Invalid file path: {}", path.display()))]
    InvalidPath {
        path: PathBuf,
        #[snafu(implicit)]
        loc:  snafu::Location,
    },

    #[snafu(display("File rolling failed: {message}"))]
    RollFileFailed {
        message: String,
        #[snafu(implicit)]
        loc:     snafu::Location,
    },

    #[snafu(display("Mmap operation failed"))]
    MmapFailed {
        source: mmap_io::MmapIoError,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },

    #[snafu(display("Index error: {message}"))]
    IndexError {
        message: String,
        #[snafu(implicit)]
        loc:     snafu::Location,
    },

    #[snafu(display("{message}"))]
    Internal {
        message: String,
        #[snafu(implicit)]
        loc:     snafu::Location,
    },

    #[snafu(display("Manifest corrupted: {reason}"))]
    ManifestCorrupted {
        reason: String,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },

    #[snafu(display("Manifest version {version} not supported"))]
    UnsupportedManifestVersion {
        version: u32,
        #[snafu(implicit)]
        loc:     snafu::Location,
    },
}
