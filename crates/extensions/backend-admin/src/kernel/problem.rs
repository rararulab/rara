use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// RFC 9457 Problem Details response.
///
/// Used for all error responses from kernel HTTP endpoints.
/// Content-Type is set to `application/problem+json`.
#[derive(Debug, Serialize)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub problem_type: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ProblemDetails {
    pub fn not_found(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            problem_type: "https://rara.dev/problems/not-found".to_string(),
            title: title.into(),
            status: 404,
            detail: Some(detail.into()),
        }
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self {
            problem_type: "https://rara.dev/problems/internal-error".to_string(),
            title: "Internal Server Error".to_string(),
            status: 500,
            detail: Some(detail.into()),
        }
    }
}

impl IntoResponse for ProblemDetails {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(self)).into_response();
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            "application/problem+json"
                .parse()
                .expect("valid header value"),
        );
        response
    }
}
