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

use snafu::Snafu;

/// Errors that a job source driver can produce.
///
/// The variants carry enough information for callers to decide whether
/// to retry, back off, or give up.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SourceError {
    /// A transient failure that can be retried.
    #[snafu(display("Retryable error from source '{source_name}': {message}"))]
    Retryable {
        source_name: String,
        message:     String,
    },

    /// A permanent failure that should not be retried.
    #[snafu(display("Non-retryable error from source '{source_name}': {message}"))]
    NonRetryable {
        source_name: String,
        message:     String,
    },

    /// The source has rate-limited us.
    #[snafu(display("Rate limited by source '{source_name}', retry after {retry_after_secs}s"))]
    RateLimited {
        source_name:      String,
        retry_after_secs: u64,
    },

    /// Authentication / authorization failure.
    #[snafu(display("Auth error for source '{source_name}': {message}"))]
    AuthError {
        source_name: String,
        message:     String,
    },

    /// The raw data could not be normalized into a valid
    /// [`NormalizedJob`].
    #[snafu(display(
        "Normalization failed for job '{source_job_id}' from '{source_name}': {message}"
    ))]
    NormalizationFailed {
        source_name:   String,
        source_job_id: String,
        message:       String,
    },
}

impl axum::response::IntoResponse for SourceError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            SourceError::Retryable { .. } => axum::http::StatusCode::SERVICE_UNAVAILABLE,
            SourceError::NonRetryable { .. } => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            SourceError::RateLimited { .. } => axum::http::StatusCode::TOO_MANY_REQUESTS,
            SourceError::AuthError { .. } => axum::http::StatusCode::UNAUTHORIZED,
            SourceError::NormalizationFailed { .. } => {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let message = self.to_string();
        if status.is_server_error() {
            tracing::error!(http_status = status.as_u16(), error = %message, "source request error");
        }
        let body = serde_json::json!({
            "error": { "status": status.as_u16(), "message": message }
        });
        (status, axum::Json(body)).into_response()
    }
}
