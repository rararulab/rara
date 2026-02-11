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

//! Error types for the application domain.

use job_domain_shared::id::ApplicationId;
use snafu::Snafu;

use crate::types::ApplicationStatus;

/// Errors that can occur in the application domain.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum ApplicationError {
    /// The requested status transition is not allowed by the state machine.
    #[snafu(display("invalid transition from {from} to {to}: transition not allowed"))]
    InvalidTransition {
        from: ApplicationStatus,
        to:   ApplicationStatus,
    },

    /// The requested application was not found (or has been soft-deleted).
    #[snafu(display("application not found: {id}"))]
    NotFound { id: ApplicationId },

    /// An application for this job already exists.
    #[snafu(display("duplicate application for job: {message}"))]
    DuplicateApplication { message: String },

    /// A storage/infrastructure error occurred.
    #[snafu(display("repository error: {message}"))]
    RepositoryError { message: String },

    /// The request data failed validation.
    #[snafu(display("validation error: {message}"))]
    ValidationError { message: String },
}

impl axum::response::IntoResponse for ApplicationError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            ApplicationError::NotFound { .. } => axum::http::StatusCode::NOT_FOUND,
            ApplicationError::ValidationError { .. } => axum::http::StatusCode::BAD_REQUEST,
            ApplicationError::InvalidTransition { .. } => axum::http::StatusCode::CONFLICT,
            ApplicationError::DuplicateApplication { .. } => axum::http::StatusCode::CONFLICT,
            ApplicationError::RepositoryError { .. } => {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let message = self.to_string();
        if status.is_server_error() {
            tracing::error!(http_status = status.as_u16(), error = %message, "application request error");
        }
        let body = serde_json::json!({
            "error": { "status": status.as_u16(), "message": message }
        });
        (status, axum::Json(body)).into_response()
    }
}
