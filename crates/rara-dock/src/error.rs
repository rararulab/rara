// Copyright 2025 Rararulab
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

use snafu::Snafu;

/// Errors produced by Dock operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum DockError {
    #[snafu(display("Failed to read dock data at {path}: {source}"))]
    Read {
        path:   String,
        source: std::io::Error,
    },

    #[snafu(display("Failed to write dock data at {path}: {source}"))]
    Write {
        path:   String,
        source: std::io::Error,
    },

    #[snafu(display("Failed to create directory {path}: {source}"))]
    CreateDir {
        path:   String,
        source: std::io::Error,
    },

    #[snafu(display("Failed to list sessions in {path}: {source}"))]
    ListSessions {
        path:   String,
        source: std::io::Error,
    },

    #[snafu(display("Failed to serialize dock data: {source}"))]
    Serialize { source: serde_json::Error },

    #[snafu(display("Failed to deserialize dock data: {source}"))]
    Deserialize { source: serde_json::Error },

    #[snafu(display("Invalid session ID: {id}"))]
    InvalidSessionId { id: String },

    #[snafu(display("Session already exists: {id}"))]
    SessionAlreadyExists { id: String },

    #[snafu(display("Kernel error: {message}"))]
    Kernel { message: String },
}
