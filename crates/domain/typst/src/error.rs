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

//! Error types for the Typst domain service.

use snafu::Snafu;
use uuid::Uuid;

/// Errors that can occur during Typst domain operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum TypstError {
    /// The requested project was not found.
    #[snafu(display("project not found: {id}"))]
    ProjectNotFound { id: Uuid },

    /// The requested file was not found.
    #[snafu(display("file not found: {path}"))]
    FileNotFound { path: String },

    /// The requested render was not found.
    #[snafu(display("render not found: {id}"))]
    RenderNotFound { id: Uuid },

    /// Invalid request data.
    #[snafu(display("invalid request: {message}"))]
    InvalidRequest { message: String },

    /// Path traversal detected.
    #[snafu(display("path traversal detected: {path}"))]
    PathTraversal { path: String },

    /// Directory not found.
    #[snafu(display("directory not found: {path}"))]
    DirectoryNotFound { path: String },

    /// Path is not a directory.
    #[snafu(display("not a directory: {path}"))]
    NotADirectory { path: String },

    /// No .typ files found in directory.
    #[snafu(display("no .typ files found in: {path}"))]
    NoTypstFiles { path: String },

    /// File I/O error.
    #[snafu(display("file I/O error: {source}"))]
    FileIo { source: std::io::Error },

    /// Typst compilation failed.
    #[snafu(display("compilation error: {message}"))]
    CompilationError { message: String },

    /// Object storage (S3) operation failed.
    #[snafu(display("storage error: {message}"))]
    Storage { message: String },

    /// Database error.
    #[snafu(display("repository error: {message}"))]
    Repository { message: String },

    /// Git clone operation failed.
    #[snafu(display("git clone failed: {message}"))]
    GitCloneFailed { message: String },

    /// Invalid Git URL.
    #[snafu(display("invalid git URL: {url}"))]
    InvalidGitUrl { url: String },

    /// Project has no associated git URL.
    #[snafu(display("project has no git URL"))]
    NotGitProject,

    /// Repository exceeds the maximum allowed size.
    #[snafu(display("repository too large: {size} bytes"))]
    RepositoryTooLarge { size: u64 },

    /// Command execution failed.
    #[snafu(display("command execution failed: {message}"))]
    CommandFailed { message: String },
}

/// Map a `sqlx::Error` into [`TypstError::Repository`].
pub fn map_db_err(e: sqlx::Error) -> TypstError {
    TypstError::Repository {
        message: e.to_string(),
    }
}

/// Map an `opendal::Error` into [`TypstError::Storage`].
pub fn map_storage_err(e: opendal::Error) -> TypstError {
    TypstError::Storage {
        message: e.to_string(),
    }
}

impl axum::response::IntoResponse for TypstError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            Self::ProjectNotFound { .. }
            | Self::FileNotFound { .. }
            | Self::RenderNotFound { .. } => axum::http::StatusCode::NOT_FOUND,

            Self::InvalidRequest { .. }
            | Self::PathTraversal { .. }
            | Self::NotADirectory { .. }
            | Self::NoTypstFiles { .. } => axum::http::StatusCode::BAD_REQUEST,

            Self::DirectoryNotFound { .. } => axum::http::StatusCode::NOT_FOUND,

            Self::CompilationError { .. } => axum::http::StatusCode::UNPROCESSABLE_ENTITY,

            Self::InvalidGitUrl { .. } | Self::NotGitProject => axum::http::StatusCode::BAD_REQUEST,

            Self::RepositoryTooLarge { .. } => axum::http::StatusCode::UNPROCESSABLE_ENTITY,

            Self::GitCloneFailed { .. }
            | Self::Storage { .. }
            | Self::Repository { .. }
            | Self::FileIo { .. }
            | Self::CommandFailed { .. } => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = self.to_string();
        if status.is_server_error() {
            tracing::error!(http_status = status.as_u16(), error = %message, "typst request error");
        }
        let body = serde_json::json!({
            "error": { "status": status.as_u16(), "message": message }
        });
        (status, axum::Json(body)).into_response()
    }
}
