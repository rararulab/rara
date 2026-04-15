//! Kimi Code OAuth integration — reads tokens from kimi-cli.
//!
//! This crate reads OAuth tokens persisted by
//! [kimi-cli](https://github.com/nicepkg/kimi-cli) at
//! `~/.kimi/credentials/kimi-code.json` and provides a
//! `LlmCredentialResolver` that injects the required
//! `Authorization` + `X-Msh-*` headers for the Kimi Code platform.
//!
//! Users authenticate via `kimi auth login` in kimi-cli; rara
//! piggybacks on those credentials with automatic refresh.

use serde::{Deserialize, Serialize};
use snafu::Snafu;

/// Kimi Code platform API base URL.
pub const KIMI_CODE_BASE_URL: &str = "https://api.kimi.com/coding/v1";

/// OAuth token endpoint for refresh.
const KIMI_AUTH_TOKEN_ENDPOINT: &str = "https://auth.kimi.com/api/oauth/token";

/// Kimi Code public OAuth client ID (same as kimi-cli).
const KIMI_CODE_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";

/// Refresh skew — attempt refresh this many seconds before expiry.
const REFRESH_SKEW_SECS: f64 = 300.0;

/// Token file name within `~/.kimi/credentials/`.
const TOKEN_FILENAME: &str = "kimi-code.json";

/// Device ID file within `~/.kimi/`.
const DEVICE_ID_FILENAME: &str = "device_id";

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum KimiOauthError {
    #[snafu(display("failed to read token file {path}: {source}"))]
    TokenFileRead {
        path:   String,
        source: std::io::Error,
    },

    #[snafu(display("failed to parse token file {path}: {source}"))]
    TokenFileParse {
        path:   String,
        source: serde_json::Error,
    },

    #[snafu(display("failed to read device ID from {path}: {source}"))]
    DeviceIdRead {
        path:   String,
        source: std::io::Error,
    },

    #[snafu(display("{context} request failed: {source}"))]
    TokenRequest {
        context: String,
        source:  reqwest::Error,
    },

    #[snafu(display("{context} failed: {status} {body}"))]
    TokenRequestStatus {
        context: String,
        status:  reqwest::StatusCode,
        body:    String,
    },

    #[snafu(display("failed to parse {context} response: {source}"))]
    TokenResponseParse {
        context: String,
        source:  reqwest::Error,
    },

    #[snafu(display("{message}"))]
    Validation { message: String },
}

pub type Result<T> = std::result::Result<T, KimiOauthError>;

// ---------------------------------------------------------------------------
// Token data types
// ---------------------------------------------------------------------------

/// Persisted Kimi OAuth token (matches kimi-cli's file format).
///
/// Field names and types match kimi-cli's `OAuthToken.to_dict()` output
/// exactly — `expires_at` is a float unix timestamp, not integer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredKimiTokens {
    pub access_token:  String,
    pub refresh_token: String,
    /// Absolute expiry time as unix timestamp (seconds, float).
    pub expires_at:    f64,
    pub scope:         String,
    pub token_type:    String,
    /// Original `expires_in` value from the OAuth response.
    pub expires_in:    f64,
}
