//! Typed error types for the STT driver.

use snafu::prelude::*;

/// Errors returned by
/// [`SttService::transcribe`](crate::SttService::transcribe).
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SttError {
    /// HTTP-level failure (timeout, connection refused, etc.).
    #[snafu(display("STT HTTP request failed: {source}"))]
    Http { source: reqwest::Error },

    /// The STT server returned a non-2xx status code.
    #[snafu(display("STT server returned {status}: {body}"))]
    ServerError { status: u16, body: String },

    /// Failed to deserialize the JSON response body.
    #[snafu(display("failed to parse STT response: {source}"))]
    Parse { source: reqwest::Error },

    /// The server returned a valid response but the `text` field was empty.
    #[snafu(display("STT response contained no text"))]
    EmptyResponse,
}

/// Convenience alias used throughout this crate.
pub type Result<T> = std::result::Result<T, SttError>;

impl SttError {
    /// Whether this error is likely transient and worth retrying
    /// (network error, timeout, 429 rate-limit, or 5xx server error).
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Http { source } => source.is_timeout() || source.is_connect(),
            Self::ServerError { status, .. } => *status == 429 || *status >= 500,
            Self::Parse { .. } | Self::EmptyResponse => false,
        }
    }
}
