use snafu::prelude::*;

/// Errors produced by [`TtsService`](crate::TtsService).
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum TtsError {
    /// Input text exceeds the configured maximum length.
    #[snafu(display("text exceeds max length {max}: got {actual}"))]
    TextTooLong { max: usize, actual: usize },

    /// The HTTP request to the TTS server failed at the transport level.
    #[snafu(display("TTS HTTP request failed: {source}"))]
    Http { source: reqwest::Error },

    /// The TTS server returned a non-2xx status code.
    #[snafu(display("TTS server returned {status}: {body}"))]
    Server { status: u16, body: String },
}

/// Convenience alias used throughout this crate.
pub type Result<T> = std::result::Result<T, TtsError>;
