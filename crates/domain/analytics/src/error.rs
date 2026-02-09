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

//! Error types for the analytics domain.

use snafu::Snafu;

/// Errors that can occur in the analytics domain.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum AnalyticsError {
    /// The requested snapshot was not found.
    #[snafu(display("snapshot not found: {id}"))]
    NotFound { id: uuid::Uuid },

    /// A repository/storage error occurred.
    #[snafu(display("repository error: {message}"))]
    Repository { message: String },

    /// A snapshot for this period+date combination already exists.
    #[snafu(display("duplicate snapshot for period {period} on date {date}"))]
    DuplicateSnapshot { period: String, date: String },
}

impl axum::response::IntoResponse for AnalyticsError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            AnalyticsError::NotFound { .. } => axum::http::StatusCode::NOT_FOUND,
            AnalyticsError::Repository { .. } => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            AnalyticsError::DuplicateSnapshot { .. } => axum::http::StatusCode::CONFLICT,
        };
        let body = serde_json::json!({
            "error": { "status": status.as_u16(), "message": self.to_string() }
        });
        (status, axum::Json(body)).into_response()
    }
}
