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

use std::time::Duration;

use snafu::Snafu;

/// Errors produced by the browser subsystem.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum BrowserError {
    /// Lightpanda binary was not found on the system.
    #[snafu(display(
        "lightpanda binary not found at '{path}' — install from https://github.com/nicholasgasior/lightpanda"
    ))]
    BinaryNotFound { path: String },

    /// Lightpanda process failed to become ready within the timeout.
    #[snafu(display("lightpanda failed to start within {timeout:?}"))]
    StartupTimeout { timeout: Duration },

    /// CDP WebSocket connection was lost and could not be re-established.
    #[snafu(display("CDP connection lost, restart failed"))]
    ConnectionLost,

    /// An element ref from a previous snapshot is no longer valid.
    #[snafu(display(
        "element ref '{ref_id}' not found — page may have changed, try browser-snapshot to refresh"
    ))]
    RefNotFound { ref_id: String },

    /// Page navigation did not complete within the allowed time.
    #[snafu(display("page load timeout for {url}"))]
    PageLoadTimeout { url: String },

    /// JavaScript evaluation returned an error.
    #[snafu(display("JS evaluation error: {message}"))]
    EvaluationError { message: String },

    /// No dialog is currently active to accept or dismiss.
    #[snafu(display("no active dialog to handle"))]
    NoActiveDialog,

    /// A CDP protocol-level error.
    #[snafu(display("CDP error: {message}"))]
    Cdp { message: String },

    /// No page is currently open in the browser.
    #[snafu(display("no active page — use browser-navigate first"))]
    NoActivePage,

    /// Tab index is out of range.
    #[snafu(display("tab index {index} out of range (have {count} tabs)"))]
    TabIndexOutOfRange { index: usize, count: usize },

    /// Lightpanda process exited unexpectedly.
    #[snafu(display("lightpanda process crashed: {message}"))]
    ProcessCrashed { message: String },

    /// `lightpanda fetch` subprocess failed.
    #[snafu(display("lightpanda fetch failed for {url}: {message}"))]
    FetchFailed { url: String, message: String },
}

/// Convenience alias for browser results.
pub type BrowserResult<T> = std::result::Result<T, BrowserError>;
