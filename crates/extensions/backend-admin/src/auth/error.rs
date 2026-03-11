use axum::{Json, response::IntoResponse};
use serde_json::json;
use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum AuthError {
    #[snafu(display("Invalid request: {message}"))]
    InvalidRequest { message: String },

    #[snafu(display("Invalid credentials"))]
    InvalidCredentials,

    #[snafu(display("Account locked, please try again in 15 minutes"))]
    AccountLocked,

    #[snafu(display("Email not verified"))]
    EmailNotVerified,

    #[snafu(display("Authentication is not configured"))]
    NotConfigured,

    #[snafu(display("Authentication failed: {message}"))]
    Internal { message: String },
}

impl IntoResponse for AuthError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            Self::InvalidRequest { .. } => axum::http::StatusCode::BAD_REQUEST,
            Self::InvalidCredentials => axum::http::StatusCode::UNAUTHORIZED,
            Self::AccountLocked => axum::http::StatusCode::TOO_MANY_REQUESTS,
            Self::EmailNotVerified => axum::http::StatusCode::FORBIDDEN,
            Self::NotConfigured | Self::Internal { .. } => {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let message = self.to_string();
        if status.is_server_error() {
            tracing::error!(http_status = status.as_u16(), error = %message, "auth request error");
        }
        let body = json!({
            "error": {
                "status": status.as_u16(),
                "message": message,
            }
        });
        (status, Json(body)).into_response()
    }
}
