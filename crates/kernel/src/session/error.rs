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

//! Session-specific error types.

use snafu::Snafu;

/// Errors that can occur during session persistence operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SessionError {
    /// The requested session was not found.
    #[snafu(display("session not found: {key}"))]
    NotFound { key: String },

    /// A session with this key already exists.
    #[snafu(display("session already exists: {key}"))]
    AlreadyExists { key: String },

    /// A storage/infrastructure error occurred.
    #[snafu(display("repository error: {source}"))]
    Repository { source: sqlx::Error },

    /// The session key is malformed.
    #[snafu(display("invalid session key: {message}"))]
    InvalidKey { message: String },

    /// The fork point is out of range.
    #[snafu(display("invalid fork point: seq {seq} is out of range for session {key}"))]
    InvalidForkPoint { key: String, seq: i64 },

    /// A file I/O error occurred while reading/writing message JSONL files.
    #[snafu(display("message file I/O error: {source}"))]
    FileIo { source: std::io::Error },

    /// A JSON serialization/deserialization error occurred.
    #[snafu(display("json error: {source}"))]
    Json { source: serde_json::Error },
}

impl From<sqlx::Error> for SessionError {
    fn from(source: sqlx::Error) -> Self { Self::Repository { source } }
}
