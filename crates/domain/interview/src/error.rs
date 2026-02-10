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

//! Error types for the interview domain.

use snafu::Snafu;
use uuid::Uuid;

/// Errors that can occur in the interview domain.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum InterviewError {
    /// The requested interview plan was not found (or has been soft-deleted).
    #[snafu(display("interview plan not found: {id}"))]
    NotFound {
        /// The id that was looked up.
        id: Uuid,
    },

    /// A storage/infrastructure error occurred.
    #[snafu(display("repository error: {message}"))]
    RepositoryError {
        /// Description of the underlying storage failure.
        message: String,
    },

    /// Validation of input data failed.
    #[snafu(display("validation error: {reason}"))]
    ValidationError {
        /// What was invalid.
        reason: String,
    },

    /// AI prep-material generation failed.
    #[snafu(display("prep generation failed: {message}"))]
    PrepGenerationFailed {
        /// Description of the generation failure.
        message: String,
    },

    /// The requested status transition is not allowed.
    #[snafu(display("invalid status transition from {from} to {to}"))]
    InvalidStatusTransition {
        /// Current status (display form).
        from: String,
        /// Requested status (display form).
        to:   String,
    },
}

impl axum::response::IntoResponse for InterviewError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            InterviewError::NotFound { .. } => axum::http::StatusCode::NOT_FOUND,
            InterviewError::ValidationError { .. } => axum::http::StatusCode::BAD_REQUEST,
            InterviewError::InvalidStatusTransition { .. } => axum::http::StatusCode::CONFLICT,
            InterviewError::RepositoryError { .. } => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            InterviewError::PrepGenerationFailed { .. } => {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let body = serde_json::json!({
            "error": { "status": status.as_u16(), "message": self.to_string() }
        });
        (status, axum::Json(body)).into_response()
    }
}
