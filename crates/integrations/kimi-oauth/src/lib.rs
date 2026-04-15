//! Kimi Code OAuth integration — reads tokens from kimi-cli.
//!
//! This crate reads OAuth tokens persisted by
//! [kimi-cli](https://github.com/nicepkg/kimi-cli) at
//! `~/.kimi/credentials/kimi-code.json` and provides a
//! `KimiCredentialResolver` that injects the required
//! `Authorization` + `X-Msh-*` headers for the Kimi Code platform.
//!
//! Users authenticate via `kimi auth login` in kimi-cli; rara
//! piggybacks on those credentials with automatic refresh.

use async_trait::async_trait;
use rara_kernel::llm::{LlmCredential, LlmCredentialResolver};
use serde::{Deserialize, Serialize};
use snafu::{OptionExt as _, ResultExt as _, Snafu};

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

/// Crate-level error type for Kimi OAuth operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum KimiOauthError {
    /// Failed to read the token file from disk.
    #[snafu(display("failed to read token file {path}: {source}"))]
    TokenFileRead {
        path:   String,
        source: std::io::Error,
    },

    /// Failed to deserialize the token file contents.
    #[snafu(display("failed to parse token file {path}: {source}"))]
    TokenFileParse {
        path:   String,
        source: serde_json::Error,
    },

    /// Failed to write the token file to disk.
    #[snafu(display("failed to write token file {path}: {source}"))]
    TokenFileWrite {
        path:   String,
        source: std::io::Error,
    },

    /// Failed to read the device ID file.
    #[snafu(display("failed to read device ID from {path}: {source}"))]
    DeviceIdRead {
        path:   String,
        source: std::io::Error,
    },

    /// HTTP request to the token endpoint failed.
    #[snafu(display("{context} request failed: {source}"))]
    TokenRequest {
        context: String,
        source:  reqwest::Error,
    },

    /// Token endpoint returned a non-success HTTP status.
    #[snafu(display("{context} failed: {status} {body}"))]
    TokenRequestStatus {
        context: String,
        status:  reqwest::StatusCode,
        body:    String,
    },

    /// Failed to parse the JSON response from the token endpoint.
    #[snafu(display("failed to parse {context} response: {source}"))]
    TokenResponseParse {
        context: String,
        source:  reqwest::Error,
    },

    /// Generic validation error.
    #[snafu(display("{message}"))]
    Validation { message: String },
}

/// Crate-level result alias.
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

// ---------------------------------------------------------------------------
// Token file paths
// ---------------------------------------------------------------------------

/// Kimi CLI share directory (`~/.kimi`).
///
/// Respects the `KIMI_SHARE_DIR` environment variable for testing
/// and non-standard installations.
fn kimi_share_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("KIMI_SHARE_DIR") {
        return std::path::PathBuf::from(dir);
    }
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".kimi")
}

fn token_file_path() -> std::path::PathBuf {
    kimi_share_dir().join("credentials").join(TOKEN_FILENAME)
}

fn device_id_path() -> std::path::PathBuf { kimi_share_dir().join(DEVICE_ID_FILENAME) }

// ---------------------------------------------------------------------------
// Token I/O
// ---------------------------------------------------------------------------

/// Load kimi-cli's persisted OAuth tokens.
///
/// Returns `Ok(None)` if the token file does not exist.
pub async fn load_tokens() -> Result<Option<StoredKimiTokens>> {
    let path = token_file_path();
    let path_str = path.display().to_string();
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context(TokenFileReadSnafu { path: path_str }),
    };
    serde_json::from_str(&raw)
        .context(TokenFileParseSnafu { path: path_str })
        .map(Some)
}

/// Save refreshed tokens back to kimi-cli's token file.
async fn save_tokens(tokens: &StoredKimiTokens) -> Result<()> {
    let path = token_file_path();
    let path_str = path.display().to_string();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context(TokenFileWriteSnafu {
                path: parent.display().to_string(),
            })?;
    }
    let raw =
        serde_json::to_string_pretty(tokens).expect("StoredKimiTokens is always serializable");
    tokio::fs::write(&path, raw)
        .await
        .context(TokenFileWriteSnafu { path: path_str })?;

    // Restrict to owner-only read/write — tokens are sensitive credentials.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = tokio::fs::set_permissions(&path, perms).await;
    }

    Ok(())
}

/// Read device ID from kimi-cli's `device_id` file.
async fn read_device_id() -> Result<String> {
    let path = device_id_path();
    let path_str = path.display().to_string();
    tokio::fs::read_to_string(&path)
        .await
        .map(|s| s.trim().to_owned())
        .context(DeviceIdReadSnafu { path: path_str })
}

/// Whether token is close enough to expiry that refresh should be attempted.
pub fn should_refresh_token(tokens: &StoredKimiTokens) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    tokens.expires_at - now < REFRESH_SKEW_SECS
}

// ---------------------------------------------------------------------------
// Token refresh
// ---------------------------------------------------------------------------

/// Raw token response from the Kimi OAuth token endpoint.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token:  String,
    refresh_token: String,
    expires_in:    f64,
    scope:         String,
    token_type:    String,
}

/// Refresh the access token using kimi-cli's refresh token.
pub async fn refresh_tokens(current: &StoredKimiTokens) -> Result<StoredKimiTokens> {
    let form = [
        ("client_id", KIMI_CODE_CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", current.refresh_token.as_str()),
    ];
    let response = reqwest::Client::new()
        .post(KIMI_AUTH_TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .context(TokenRequestSnafu {
            context: "kimi token refresh",
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unavailable>".into());
        return TokenRequestStatusSnafu {
            context: "kimi token refresh",
            status,
            body,
        }
        .fail();
    }
    let token: TokenResponse = response.json().await.context(TokenResponseParseSnafu {
        context: "kimi token refresh",
    })?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    Ok(StoredKimiTokens {
        access_token:  token.access_token,
        refresh_token: token.refresh_token,
        expires_at:    now + token.expires_in,
        scope:         token.scope,
        token_type:    token.token_type,
        expires_in:    token.expires_in,
    })
}

// ---------------------------------------------------------------------------
// Common headers
// ---------------------------------------------------------------------------

/// Build the `X-Msh-*` metadata headers required by Kimi Code platform.
///
/// Fails if `~/.kimi/device_id` is unreadable — requests with a
/// placeholder device ID would be rejected by the server anyway.
async fn kimi_common_headers() -> Result<Vec<(String, String)>> {
    let device_id = read_device_id().await?;

    // Best-effort hostname — no extra crate dependency.
    let device_name = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "unknown".into());

    let device_model = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);

    Ok(vec![
        ("X-Msh-Platform".into(), "kimi_cli".into()),
        ("X-Msh-Version".into(), "0.0.1".into()),
        ("X-Msh-Device-Name".into(), device_name),
        ("X-Msh-Device-Model".into(), device_model),
        ("X-Msh-Os-Version".into(), std::env::consts::OS.into()),
        ("X-Msh-Device-Id".into(), device_id),
    ])
}

// ---------------------------------------------------------------------------
// KimiCredentialResolver
// ---------------------------------------------------------------------------

/// Dynamic credential resolver for the Kimi Code platform.
///
/// On each call to [`resolve`](LlmCredentialResolver::resolve), loads the
/// current token from disk, refreshes it if near expiry, and returns a
/// fresh [`LlmCredential`] with the required `X-Msh-*` headers.
pub struct KimiCredentialResolver;

#[async_trait]
impl LlmCredentialResolver for KimiCredentialResolver {
    async fn resolve(&self) -> rara_kernel::error::Result<LlmCredential> {
        let mut tokens = load_tokens()
            .await
            .map_err(|e| rara_kernel::error::KernelError::Provider {
                message: e.to_string().into(),
            })?
            .context(rara_kernel::error::ProviderNotConfiguredSnafu)?;

        if should_refresh_token(&tokens) {
            match refresh_tokens(&tokens).await {
                Ok(refreshed) => {
                    if let Err(e) = save_tokens(&refreshed).await {
                        tracing::warn!("failed to persist refreshed kimi tokens: {e}");
                    }
                    tokens = refreshed;
                }
                Err(e) => {
                    tracing::warn!(
                        expires_at = tokens.expires_at,
                        "kimi token refresh failed, using existing token: {e}",
                    );
                }
            }
        }

        let headers =
            kimi_common_headers()
                .await
                .map_err(|e| rara_kernel::error::KernelError::Provider {
                    message: e.to_string().into(),
                })?;

        let mut cred = LlmCredential::new(KIMI_CODE_BASE_URL, &tokens.access_token);
        for (name, value) in headers {
            cred = cred.with_header(name, value);
        }
        Ok(cred)
    }
}
