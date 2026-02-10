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

//! Error types for the saved-job domain.

use snafu::Snafu;
use uuid::Uuid;

/// Errors that can occur in the saved-job domain.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SavedJobError {
    /// The requested saved job was not found.
    #[snafu(display("saved job not found: {id}"))]
    NotFound { id: Uuid },

    /// A saved job with this URL already exists.
    #[snafu(display("duplicate URL: {url}"))]
    DuplicateUrl { url: String },

    /// The request data failed validation.
    #[snafu(display("validation error: {message}"))]
    ValidationError { message: String },

    /// A storage/infrastructure error occurred.
    #[snafu(display("repository error: {message}"))]
    RepositoryError { message: String },
}

impl axum::response::IntoResponse for SavedJobError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            SavedJobError::NotFound { .. } => axum::http::StatusCode::NOT_FOUND,
            SavedJobError::DuplicateUrl { .. } => axum::http::StatusCode::CONFLICT,
            SavedJobError::ValidationError { .. } => axum::http::StatusCode::BAD_REQUEST,
            SavedJobError::RepositoryError { .. } => {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let body = serde_json::json!({
            "error": { "status": status.as_u16(), "message": self.to_string() }
        });
        (status, axum::Json(body)).into_response()
    }
}
