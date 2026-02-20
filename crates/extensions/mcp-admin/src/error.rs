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

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum McpAdminError {
    #[snafu(display("server not found: {name}"))]
    ServerNotFound { name: String },
    #[snafu(display("server already exists: {name}"))]
    ServerAlreadyExists { name: String },
    #[snafu(display("server not connected: {name}"))]
    ServerNotConnected { name: String },
    #[snafu(display("mcp error: {message}"))]
    McpError { message: String },
    #[snafu(display("registry error: {message}"))]
    RegistryError { message: String },
}

impl IntoResponse for McpAdminError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            Self::ServerNotFound { .. } => (StatusCode::NOT_FOUND, self.to_string()),
            Self::ServerAlreadyExists { .. } => (StatusCode::CONFLICT, self.to_string()),
            Self::ServerNotConnected { .. } => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::McpError { .. } | Self::RegistryError { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        };
        (status, axum::Json(json!({ "error": msg }))).into_response()
    }
}
