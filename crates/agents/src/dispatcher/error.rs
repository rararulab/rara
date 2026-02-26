use axum::{http::StatusCode, response::IntoResponse};
use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum DispatcherError {
    #[snafu(display("dispatcher channel closed"))]
    ChannelClosed,
    #[snafu(display("task not found: {task_id}"))]
    TaskNotFound { task_id: String },
    #[snafu(display("agent execution failed: {message}"))]
    AgentError { message: String },
}

impl IntoResponse for DispatcherError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            DispatcherError::ChannelClosed => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            DispatcherError::TaskNotFound { .. } => (StatusCode::NOT_FOUND, self.to_string()),
            DispatcherError::AgentError { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        };
        (status, msg).into_response()
    }
}
