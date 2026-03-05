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

use snafu::prelude::*;

/// Errors produced by the local tape subsystem.
///
/// These errors are intentionally local to `tape` so file-format failures and
/// cache/state issues do not get flattened into the broader memory crate error
/// taxonomy.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(super)))]
pub enum TapError {
    /// Filesystem or OS-level failure while reading, writing, renaming, or
    /// syncing tape files.
    #[snafu(display("tape I/O error: {source}"))]
    Io { source: std::io::Error },

    /// Failure while serializing a tape entry or derived JSON payload.
    #[snafu(display("tape JSON encode error: {source}"))]
    JsonEncode { source: serde_json::Error },

    /// Failure while decoding persisted JSONL content back into structured
    /// tape entries.
    #[snafu(display("tape JSON decode error: {source}"))]
    JsonDecode { source: serde_json::Error },

    /// Failure while decoding the URL-encoded tape name stored in a filename.
    #[snafu(display("tape URL decode error: {source}"))]
    UrlDecode { source: std::string::FromUtf8Error },

    /// Internal invariant failure in cache or worker lifecycle management.
    #[snafu(display("tape state error: {message}"))]
    State { message: String },
}

/// Convenience result alias used by all tape modules.
pub type TapResult<T> = Result<T, TapError>;
