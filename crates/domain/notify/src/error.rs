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

//! Error types for the notification domain.

use serde::{Deserialize, Serialize};
use snafu::Snafu;
use uuid::Uuid;

#[derive(Debug, Clone, Snafu, Serialize, Deserialize)]
pub enum NotifyError {
    #[snafu(display("notification not found: {id}"))]
    NotFound { id: Uuid },

    #[snafu(display("send failed on {channel}: {message}"))]
    SendFailed { channel: String, message: String },

    #[snafu(display("repository error: {message}"))]
    RepositoryError { message: String },

    #[snafu(display("validation error: {message}"))]
    ValidationError { message: String },

    #[snafu(display("retry exhausted for {id} after {attempts} attempts"))]
    RetryExhausted { id: Uuid, attempts: i32 },
}

impl axum::response::IntoResponse for NotifyError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            NotifyError::NotFound { .. } => axum::http::StatusCode::NOT_FOUND,
            NotifyError::ValidationError { .. } => axum::http::StatusCode::BAD_REQUEST,
            NotifyError::RetryExhausted { .. } => axum::http::StatusCode::CONFLICT,
            NotifyError::SendFailed { .. } => axum::http::StatusCode::BAD_GATEWAY,
            NotifyError::RepositoryError { .. } => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = serde_json::json!({
            "error": { "status": status.as_u16(), "message": self.to_string() }
        });
        (status, axum::Json(body)).into_response()
    }
}
