// Copyright 2025 Rararulab
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

//! Error types for the WeChat iLink Bot protocol layer.
//!
//! Ported from
//! [wechat-agent-rs](https://github.com/rararulab/wechat-agent-rs).

use snafu::Snafu;

/// Errors that can occur when interacting with the WeChat iLink Bot API.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    /// An HTTP request failed.
    #[snafu(display("HTTP error: {source}"))]
    Http { source: reqwest::Error },

    /// JSON serialization or deserialization failed.
    #[snafu(display("JSON error: {source}"))]
    Json { source: serde_json::Error },

    /// A filesystem I/O operation failed.
    #[snafu(display("IO error: {source}"))]
    Io { source: std::io::Error },

    /// The WeChat API returned a non-zero error code.
    #[snafu(display("API error (code {code}): {message}"))]
    Api {
        /// The numeric error code from the API.
        code:    i64,
        /// The human-readable error message.
        message: String,
    },

    /// The current session has expired and requires re-authentication.
    #[snafu(display("Session expired"))]
    SessionExpired,

    /// The login QR code has expired before being scanned.
    #[snafu(display("QR code expired"))]
    QrCodeExpired,

    /// The login flow failed for the given reason.
    #[snafu(display("Login failed: {reason}"))]
    LoginFailed {
        /// Description of why the login failed.
        reason: String,
    },

    /// No saved account was found in local storage.
    #[snafu(display("No account found"))]
    NoAccount,

    /// An AES encryption or decryption operation failed.
    #[snafu(display("Encryption error: {reason}"))]
    Encryption {
        /// Description of the encryption failure.
        reason: String,
    },
}

/// A convenience alias for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use snafu::IntoError;

    use super::*;

    #[test]
    fn test_error_display_api() {
        let err = ApiSnafu {
            code:    42_i64,
            message: "bad request".to_string(),
        }
        .build();
        let display = format!("{err}");
        assert!(
            display.contains("42"),
            "should contain error code, got: {display}"
        );
        assert!(
            display.contains("bad request"),
            "should contain message, got: {display}"
        );
    }

    #[test]
    fn test_error_display_login_failed() {
        let err = LoginFailedSnafu {
            reason: "invalid credentials".to_string(),
        }
        .build();
        let display = format!("{err}");
        assert!(
            display.contains("invalid credentials"),
            "should contain reason, got: {display}"
        );
    }

    #[test]
    fn test_error_display_encryption() {
        let err = EncryptionSnafu {
            reason: "bad key".to_string(),
        }
        .build();
        let display = format!("{err}");
        assert!(
            display.contains("bad key"),
            "should contain reason, got: {display}"
        );
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: Error = IoSnafu.into_error(io_err);
        assert!(
            matches!(err, Error::Io { .. }),
            "expected Io variant, got: {err:?}"
        );
        let display = format!("{err}");
        assert!(
            display.contains("file missing"),
            "should contain source message, got: {display}"
        );
    }

    #[test]
    fn test_error_from_json() {
        let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let err: Error = JsonSnafu.into_error(json_err);
        assert!(
            matches!(err, Error::Json { .. }),
            "expected Json variant, got: {err:?}"
        );
    }
}
