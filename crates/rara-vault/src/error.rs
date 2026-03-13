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

/// Errors produced by Vault operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum VaultError {
    #[snafu(display("Vault authentication failed: {source}"))]
    Auth { source: reqwest::Error },

    #[snafu(display("Vault connection error: {source}"))]
    Connection { source: reqwest::Error },

    #[snafu(display("Failed to read auth credential file {path}: {source}"))]
    CredentialFile {
        path:   String,
        source: std::io::Error,
    },

    #[snafu(display("Vault API error: {status} - {message}"))]
    Api { status: u16, message: String },

    #[snafu(display("Secret not found: {path}"))]
    NotFound { path: String },

    #[snafu(display("Failed to deserialize Vault response: {source}"))]
    Deserialize { source: serde_json::Error },

    #[snafu(display("Token expired and renewal failed"))]
    TokenExpired,
}
