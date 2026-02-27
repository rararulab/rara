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

use base::shared_string::SharedString;
use snafu::Snafu;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("LLM provider error: {message}"))]
    Provider {
        message: SharedString,
    },

    #[snafu(display("LLM provider not configured"))]
    ProviderNotConfigured {
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("context window exceeded"))]
    ContextWindow,
    #[snafu(display("retryable server error"))]
    RetryableServer,
    #[snafu(display("non-retryable error"))]
    NonRetryable,

    IO {
        source:   std::io::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("context error: {message}"))]
    Context {
        message: SharedString,
    },

    Other {
        message: SharedString,
    },
}

impl From<(&str, Option<u16>)> for Error {
    fn from((msg, status_code): (&str, Option<u16>)) -> Self {
        if matches!(status_code, Some(500 | 502 | 503 | 529)) {
            return Error::RetryableServer;
        }

        // Check if an error message indicates a context window overflow.
        fn is_context_window_error(msg: &str) -> bool {
            /// Error patterns that indicate the context window has been
            /// exceeded.
            const CONTEXT_WINDOW_PATTERNS: &[&str] = &[
                "context_length_exceeded",
                "max_tokens",
                "too many tokens",
                "request too large",
                "maximum context length",
                "context window",
                "token limit",
                "content_too_large",
                "request_too_large",
            ];

            let lower = msg.to_ascii_lowercase();
            CONTEXT_WINDOW_PATTERNS
                .iter()
                .any(|pattern| lower.contains(pattern))
        }

        // Check if an error looks like a transient provider failure that may
        // succeed on retry (5xx, overloaded, etc.).
        fn is_retryable_server_error(msg: &str) -> bool {
            /// Error patterns that indicate a transient server error worth
            /// retrying.
            const RETRYABLE_PATTERNS: &[&str] = &[
                "http 500",
                "http 502",
                "http 503",
                "http 529",
                "server_error",
                "internal server error",
                "overloaded",
                "bad gateway",
                "service unavailable",
                "the server had an error processing your request",
            ];

            let lower = msg.to_ascii_lowercase();
            RETRYABLE_PATTERNS
                .iter()
                .any(|pattern| lower.contains(pattern))
        }

        if is_context_window_error(msg) {
            Error::ContextWindow
        } else if is_retryable_server_error(msg) {
            Error::RetryableServer
        } else {
            Error::NonRetryable
        }
    }
}

/// Whether the error is eligible for model fallback.
///
/// Context window errors and missing API key errors are NOT eligible because
/// switching models would not resolve them.
pub fn is_fallback_eligible(err: &Error) -> bool {
    !matches!(
        err,
        Error::ContextWindow | Error::ProviderNotConfigured { .. }
    )
}

pub fn is_retryable_provider_error(err: &Error) -> bool {
    match err {
        Error::Provider { message } => {
            // Classify based on error message patterns (provider-agnostic).
            matches!(
                Error::from((message.as_ref(), None::<u16>)),
                Error::RetryableServer
            )
        }
        Error::RetryableServer => true,
        _ => false,
    }
}

impl From<rara_kernel::KernelError> for Error {
    fn from(err: rara_kernel::KernelError) -> Self {
        Self::Other {
            message: err.to_string().into(),
        }
    }
}

pub mod prelude {
    pub use super::*;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_eligible_for_retryable_server() {
        assert!(is_fallback_eligible(&Error::RetryableServer));
    }

    #[test]
    fn fallback_eligible_for_non_retryable() {
        assert!(is_fallback_eligible(&Error::NonRetryable));
    }

    #[test]
    fn fallback_eligible_for_other() {
        assert!(is_fallback_eligible(&Error::Other {
            message: "something went wrong".into(),
        }));
    }

    #[test]
    fn fallback_not_eligible_for_context_window() {
        assert!(!is_fallback_eligible(&Error::ContextWindow));
    }

    #[test]
    fn fallback_not_eligible_for_not_configured() {
        assert!(!is_fallback_eligible(&Error::ProviderNotConfigured {
            location: snafu::Location::new("test", 0, 0),
        }));
    }
}
