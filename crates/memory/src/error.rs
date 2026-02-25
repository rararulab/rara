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

//! Unified error types for the memory layer.
//!
//! Each backend has its own error variant so callers can distinguish which
//! service failed. The [`Http`](MemoryError::Http) variant captures low-level
//! transport errors (DNS, timeout, TLS) that are common across all backends.

use snafu::prelude::*;

/// Errors produced by the memory layer.
///
/// Variant names correspond 1:1 to the backend that originated the error,
/// making it easy to triage issues in logs and monitoring.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum MemoryError {
    /// A non-2xx response or deserialization failure from the mem0 API.
    #[snafu(display("mem0 error: {message}"))]
    Mem0 { message: String },

    /// A non-2xx response or deserialization failure from the Memos API.
    #[snafu(display("memos error: {message}"))]
    Memos { message: String },

    /// A non-2xx response or deserialization failure from the Hindsight API.
    #[snafu(display("hindsight error: {message}"))]
    Hindsight { message: String },

    /// A transport-level error (DNS, timeout, connection refused, TLS).
    #[snafu(display("HTTP request failed: {source}"))]
    Http { source: reqwest::Error },

    /// Catch-all for errors that don't fit the above categories.
    #[snafu(display("{message}"))]
    Other { message: String },
}

/// Convenience alias used throughout this crate.
pub type MemoryResult<T> = Result<T, MemoryError>;
