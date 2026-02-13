//! Error types for the sessions domain.

use snafu::Snafu;

/// Errors that can occur in the sessions domain.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SessionError {
    /// The requested session was not found.
    #[snafu(display("session not found: {key}"))]
    NotFound { key: String },

    /// A session with this key already exists.
    #[snafu(display("session already exists: {key}"))]
    AlreadyExists { key: String },

    /// A storage/infrastructure error occurred.
    #[snafu(display("repository error: {source}"))]
    Repository { source: sqlx::Error },

    /// The session key is malformed.
    #[snafu(display("invalid session key: {message}"))]
    InvalidKey { message: String },

    /// The fork point is out of range.
    #[snafu(display("invalid fork point: seq {seq} is out of range for session {key}"))]
    InvalidForkPoint { key: String, seq: i64 },
}

impl From<sqlx::Error> for SessionError {
    fn from(source: sqlx::Error) -> Self {
        Self::Repository { source }
    }
}
