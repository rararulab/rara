use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum McpAdminError {
    #[snafu(display("server not found: {name}"))]
    ServerNotFound { name: String },
    #[snafu(display("server already exists: {name}"))]
    ServerAlreadyExists { name: String },
    #[snafu(display("server not connected: {name}"))]
    ServerNotConnected { name: String },
    #[snafu(display("mcp error: {message}"))]
    McpError { message: String },
    #[snafu(display("registry error: {message}"))]
    RegistryError { message: String },
}

impl IntoResponse for McpAdminError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            Self::ServerNotFound { .. } => (StatusCode::NOT_FOUND, self.to_string()),
            Self::ServerAlreadyExists { .. } => (StatusCode::CONFLICT, self.to_string()),
            Self::ServerNotConnected { .. } => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::McpError { .. } | Self::RegistryError { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        };
        (status, axum::Json(json!({ "error": msg }))).into_response()
    }
}
