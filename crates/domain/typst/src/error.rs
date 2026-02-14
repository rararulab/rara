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
    #[snafu(display("file not found: {path} in project {project_id}"))]
    FileNotFound { project_id: Uuid, path: String },

    /// The requested render was not found.
    #[snafu(display("render not found: {id}"))]
    RenderNotFound { id: Uuid },

    /// A file with the same path already exists in the project.
    #[snafu(display("file already exists: {path} in project {project_id}"))]
    FileAlreadyExists { project_id: Uuid, path: String },

    /// Invalid request data.
    #[snafu(display("invalid request: {message}"))]
    InvalidRequest { message: String },

    /// Typst compilation failed.
    #[snafu(display("compilation error: {message}"))]
    CompilationError { message: String },

    /// Object storage (S3) operation failed.
    #[snafu(display("storage error: {message}"))]
    Storage { message: String },

    /// Database error.
    #[snafu(display("repository error: {message}"))]
    Repository { message: String },
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

            Self::FileAlreadyExists { .. } => axum::http::StatusCode::CONFLICT,

            Self::InvalidRequest { .. } => axum::http::StatusCode::BAD_REQUEST,

            Self::CompilationError { .. } => axum::http::StatusCode::UNPROCESSABLE_ENTITY,

            Self::Storage { .. } | Self::Repository { .. } => {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
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
