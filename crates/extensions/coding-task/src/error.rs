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

//! Error types for the coding-task extension.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use snafu::Snafu;
use uuid::Uuid;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum CodingTaskError {
    #[snafu(display("coding task not found: {id}"))]
    NotFound { id: Uuid },

    #[snafu(display("repository error: {message}"))]
    Repository { message: String },

    #[snafu(display("workspace error: {message}"))]
    Workspace { message: String },

    #[snafu(display("invalid status transition from {from} to {to}"))]
    InvalidTransition { from: String, to: String },

    #[snafu(display("agent execution error: {message}"))]
    Execution { message: String },
}

impl IntoResponse for CodingTaskError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::InvalidTransition { .. } => StatusCode::CONFLICT,
            Self::Repository { .. }
            | Self::Workspace { .. }
            | Self::Execution { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = serde_json::json!({ "error": self.to_string() });
        (status, axum::Json(body)).into_response()
    }
}
