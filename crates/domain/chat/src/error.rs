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

//! Error types for the chat domain service.
//!
//! [`ChatError`] wraps errors from the sessions layer and the agent runner,
//! and implements [`axum::response::IntoResponse`] so that handlers can
//! return `Result<T, ChatError>` directly.

use snafu::Snafu;

/// Errors that can occur during chat domain operations.
///
/// Each variant maps to an appropriate HTTP status code via the
/// [`IntoResponse`](axum::response::IntoResponse) implementation.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum ChatError {
    /// The requested session was not found.
    #[snafu(display("session not found: {key}"))]
    SessionNotFound { key: String },

    /// Invalid request data.
    #[snafu(display("invalid request: {message}"))]
    InvalidRequest { message: String },

    /// The LLM agent failed.
    #[snafu(display("agent error: {message}"))]
    AgentError { message: String },

    /// A session storage error occurred.
    #[snafu(display("session error: {message}"))]
    SessionError { message: String },
}

/// Convert a sessions-layer error into a chat-domain error.
///
/// [`SessionError::NotFound`](rara_sessions::error::SessionError::NotFound)
/// maps to [`ChatError::SessionNotFound`]; all other variants become a
/// generic [`ChatError::SessionError`].
impl From<rara_sessions::error::SessionError> for ChatError {
    fn from(err: rara_sessions::error::SessionError) -> Self {
        match err {
            rara_sessions::error::SessionError::NotFound { key } => Self::SessionNotFound { key },
            other => Self::SessionError {
                message: other.to_string(),
            },
        }
    }
}

/// Convert an agent-runner error into a [`ChatError::AgentError`].
impl From<rara_agents::err::Error> for ChatError {
    fn from(err: rara_agents::err::Error) -> Self {
        Self::AgentError {
            message: err.to_string(),
        }
    }
}

/// Maps [`ChatError`] variants to HTTP status codes:
///
/// | Variant            | Status              |
/// |--------------------|---------------------|
/// | `SessionNotFound`  | `404 Not Found`     |
/// | `InvalidRequest`   | `400 Bad Request`   |
/// | `AgentError`       | `502 Bad Gateway`   |
/// | `SessionError`     | `500 Internal`      |
///
/// Server errors (`5xx`) are logged at the `error` level.
impl axum::response::IntoResponse for ChatError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            Self::SessionNotFound { .. } => axum::http::StatusCode::NOT_FOUND,
            Self::InvalidRequest { .. } => axum::http::StatusCode::BAD_REQUEST,
            Self::AgentError { .. } => axum::http::StatusCode::BAD_GATEWAY,
            Self::SessionError { .. } => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = self.to_string();
        if status.is_server_error() {
            tracing::error!(http_status = status.as_u16(), error = %message, "chat request error");
        }
        let body = serde_json::json!({
            "error": { "status": status.as_u16(), "message": message }
        });
        (status, axum::Json(body)).into_response()
    }
}
