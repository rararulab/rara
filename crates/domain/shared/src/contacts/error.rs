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

//! Error types for contacts.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use snafu::Snafu;
use uuid::Uuid;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum ContactError {
    #[snafu(display("contact not found: {id}"))]
    NotFound { id: Uuid },

    #[snafu(display("contact with username '{username}' already exists"))]
    DuplicateUsername { username: String },

    #[snafu(display("repository error: {source}"))]
    Repository { source: sqlx::Error },

    #[snafu(display("validation error: {message}"))]
    Validation { message: String },
}

impl IntoResponse for ContactError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            ContactError::NotFound { .. } => (StatusCode::NOT_FOUND, self.to_string()),
            ContactError::DuplicateUsername { .. } => (StatusCode::CONFLICT, self.to_string()),
            ContactError::Repository { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            ContactError::Validation { .. } => (StatusCode::BAD_REQUEST, self.to_string()),
        };
        (status, msg).into_response()
    }
}
