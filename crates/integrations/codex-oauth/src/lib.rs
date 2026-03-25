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

//! Shared Codex OAuth integration primitives.
//!
//! This crate centralizes all provider-specific OAuth behavior:
//! - OAuth URL construction and PKCE helpers
//! - Authorization-code and refresh-token exchanges
//! - Token persistence via file-based storage
//! - Short-lived pending OAuth state persistence
//! - Token-expiry/refresh policy
//! - Ephemeral local callback server on port 1455
//!
//! The Codex public OAuth client (`app_EMoamEEZ73f0CkXaXp7hrann`) only
//! accepts `http://localhost:1455/auth/callback` as its redirect URI.
//! We therefore spin up a one-shot axum server on that port to capture
//! the authorization code, exchange it for tokens, and redirect the
//! browser to the frontend settings page.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rara_kernel::llm::{LlmCredential, LlmCredentialResolver};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use snafu::{OptionExt as _, ResultExt as _, Snafu};
use std::collections::HashMap;

/// OpenAI authorization endpoint for Codex OAuth.
pub const CODEX_AUTH_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
/// OpenAI token endpoint for Codex OAuth.
pub const CODEX_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
/// Default public client id for Codex OAuth.
///
/// Override with `RARA_CODEX_CLIENT_ID` in environments where this default is
/// not accepted.
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Requested OAuth scopes for Codex provider integration.
pub const CODEX_SCOPES: &str = "openid profile email offline_access";
/// The **only** redirect URI accepted by the Codex public OAuth client.
pub const CODEX_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
/// Environment variable used to override OAuth client id.
pub const CODEX_CLIENT_ID_ENV: &str = "RARA_CODEX_CLIENT_ID";
const REFRESH_SKEW_SECS: u64 = 60;
const CODEX_TOKEN_FILENAME: &str = "codex_tokens.json";
// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Crate-level error type for Codex OAuth operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum CodexOauthError {
    /// Failed to parse the authorization URL.
    #[snafu(display("failed to parse auth URL: {reason}"))]
    AuthUrlParse { reason: String },

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

    /// Failed to create the parent directory for the token file.
    #[snafu(display("failed to create directory {path}: {source}"))]
    TokenDirCreate {
        path:   String,
        source: std::io::Error,
    },

    /// Failed to write the token file to disk.
    #[snafu(display("failed to write token file {path}: {source}"))]
    TokenFileWrite {
        path:   String,
        source: std::io::Error,
    },

    /// Failed to set file permissions on the token file.
    #[snafu(display("failed to set permissions on {path}: {source}"))]
    TokenFilePermissions {
        path:   String,
        source: std::io::Error,
    },

    /// Failed to delete the token file from disk.
    #[snafu(display("failed to delete token file {path}: {source}"))]
    TokenFileDelete {
        path:   String,
        source: std::io::Error,
    },

    /// Failed to serialize tokens to JSON.
    #[snafu(display("failed to serialize tokens: {source}"))]
    TokenSerialize { source: serde_json::Error },

    /// OAuth flow validation error (state mismatch, missing fields, etc.).
    #[snafu(display("{message}"))]
    OAuthValidation { message: String },

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

    /// Failed to URL-encode form parameters for the token request.
    #[snafu(display("failed to encode {context} payload: {reason}"))]
    TokenRequestEncode { context: String, reason: String },

    /// Failed to parse the callback URL pasted by the user.
    #[snafu(display("failed to parse callback URL: {reason}"))]
    CallbackUrlParse { reason: String },
}

/// Crate-level result alias.
pub type Result<T> = std::result::Result<T, CodexOauthError>;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Persisted Codex credentials (file-backed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCodexTokens {
    pub access_token:    String,
    pub refresh_token:   Option<String>,
    pub id_token:        Option<String>,
    pub expires_at_unix: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token:  String,
    refresh_token: Option<String>,
    id_token:      Option<String>,
    expires_in:    Option<u64>,
}

/// Path to the Codex OAuth token file.
fn token_file_path() -> std::path::PathBuf { rara_paths::config_dir().join(CODEX_TOKEN_FILENAME) }

/// Construct the full authorization URL for redirecting the user.
///
/// Uses the fixed redirect URI `http://localhost:1455/auth/callback` that is
/// pre-registered with the Codex public OAuth client.
pub fn build_auth_url(state: &str, code_challenge: &str) -> Result<String> {
    let client_id = codex_client_id();
    let mut url =
        reqwest::Url::parse(CODEX_AUTH_ENDPOINT).map_err(|e| CodexOauthError::AuthUrlParse {
            reason: e.to_string(),
        })?;
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", CODEX_REDIRECT_URI)
        .append_pair("response_type", "code")
        .append_pair("scope", CODEX_SCOPES)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true");
    Ok(url.into())
}

/// Load persisted Codex tokens from the token file.
pub async fn load_tokens() -> Result<Option<StoredCodexTokens>> {
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

/// Save Codex tokens to the token file.
pub async fn save_tokens(tokens: &StoredCodexTokens) -> Result<()> {
    let path = token_file_path();
    let path_str = path.display().to_string();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context(TokenDirCreateSnafu {
                path: parent.display().to_string(),
            })?;
    }
    let raw = serde_json::to_string_pretty(tokens).context(TokenSerializeSnafu)?;
    tokio::fs::write(&path, raw)
        .await
        .context(TokenFileWriteSnafu { path: &path_str })?;

    // Restrict to owner-only read/write — tokens are sensitive credentials.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        tokio::fs::set_permissions(&path, perms)
            .await
            .context(TokenFilePermissionsSnafu { path: &path_str })?;
    }

    Ok(())
}

/// Delete persisted Codex tokens.
pub async fn clear_tokens() -> Result<()> {
    let path = token_file_path();
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context(TokenFileDeleteSnafu {
            path: path.display().to_string(),
        }),
    }
}

/// Extract `code` and `state` query parameters from a callback URL pasted by
/// the user.
///
/// The callback URL is the full URL the browser was redirected to after
/// authorization (e.g. `http://localhost:1455/auth/callback?code=...&state=...`).
/// The browser redirect may have failed — the user copies the URL from the
/// address bar and pastes it into the CLI, which then completes the exchange
/// locally without needing a running HTTP server.
pub fn parse_callback_url(url: &str) -> Result<(String, String)> {
    let parsed =
        reqwest::Url::parse(url.trim()).map_err(|e| CodexOauthError::CallbackUrlParse {
            reason: e.to_string(),
        })?;

    let params: HashMap<_, _> = parsed.query_pairs().into_owned().collect();

    if let Some(err) = params.get("error") {
        return OAuthValidationSnafu {
            message: format!("provider returned error: {err}"),
        }
        .fail();
    }

    let code = params
        .get("code")
        .cloned()
        .context(OAuthValidationSnafu {
            message: "missing authorization code in callback URL",
        })?;

    let state = params
        .get("state")
        .cloned()
        .context(OAuthValidationSnafu {
            message: "missing state in callback URL",
        })?;

    Ok((code, state))
}

/// Generate a cryptographically random OAuth state value.
pub fn generate_nonce() -> String { uuid::Uuid::new_v4().simple().to_string() }

/// Generate a PKCE code verifier.
pub fn generate_code_verifier() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Compute a PKCE code challenge from verifier (S256).
pub fn generate_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Validate callback state against expected state.
pub fn validate_state(expected: &str, actual: Option<&str>) -> Result<()> {
    let Some(actual) = actual else {
        return OAuthValidationSnafu {
            message: "missing oauth state",
        }
        .fail();
    };
    if expected.is_empty() {
        return OAuthValidationSnafu {
            message: "missing expected oauth state",
        }
        .fail();
    }
    if expected != actual {
        return OAuthValidationSnafu {
            message: "oauth state mismatch",
        }
        .fail();
    }
    Ok(())
}

/// Current unix timestamp in seconds.
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Convert `expires_in` value into absolute unix expiry.
pub fn compute_expires_at_unix(now_unix: u64, expires_in_secs: Option<u64>) -> Option<u64> {
    expires_in_secs.map(|v| now_unix.saturating_add(v))
}

/// Whether token is close enough to expiry that refresh should be attempted.
pub fn should_refresh_token(expires_at_unix: Option<u64>) -> bool {
    let Some(expires_at_unix) = expires_at_unix else {
        return false;
    };
    now_unix().saturating_add(REFRESH_SKEW_SECS) >= expires_at_unix
}

/// Exchange OAuth authorization code for tokens.
pub async fn exchange_authorization_code(
    code: &str,
    code_verifier: &str,
) -> Result<StoredCodexTokens> {
    let client_id = codex_client_id();
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", client_id.as_str()),
        ("code", code),
        ("redirect_uri", CODEX_REDIRECT_URI),
        ("code_verifier", code_verifier),
    ];
    let token = send_token_request(&form, "oauth token exchange").await?;
    Ok(StoredCodexTokens {
        access_token:    token.access_token,
        refresh_token:   token.refresh_token,
        id_token:        token.id_token,
        expires_at_unix: compute_expires_at_unix(now_unix(), token.expires_in),
    })
}

/// Refresh access token using a stored refresh token.
///
/// If token endpoint omits `refresh_token` or `id_token`, previous values are
/// preserved.
pub async fn refresh_tokens(current: &StoredCodexTokens) -> Result<StoredCodexTokens> {
    let refresh_token = current
        .refresh_token
        .as_deref()
        .context(OAuthValidationSnafu {
            message: "codex token expired and no refresh token is available",
        })?;
    let client_id = codex_client_id();
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", client_id.as_str()),
        ("refresh_token", refresh_token),
    ];
    let token = send_token_request(&form, "codex token refresh").await?;
    Ok(StoredCodexTokens {
        access_token:    token.access_token,
        refresh_token:   token
            .refresh_token
            .or_else(|| current.refresh_token.clone()),
        id_token:        token.id_token.or_else(|| current.id_token.clone()),
        expires_at_unix: compute_expires_at_unix(now_unix(), token.expires_in),
    })
}

// ---------------------------------------------------------------------------
// CodexCredentialResolver
// ---------------------------------------------------------------------------

/// Dynamic credential resolver for OpenAI via Codex OAuth.
///
/// On each call to `resolve`, loads the current token from disk,
/// refreshes it if expired, and returns a fresh `LlmCredential`.
pub struct CodexCredentialResolver;

#[async_trait]
impl LlmCredentialResolver for CodexCredentialResolver {
    async fn resolve(&self) -> rara_kernel::error::Result<LlmCredential> {
        use snafu::OptionExt as _;

        let mut tokens = load_tokens()
            .await
            .map_err(|e| rara_kernel::error::KernelError::Provider {
                message: e.to_string().into(),
            })?
            .context(rara_kernel::error::ProviderNotConfiguredSnafu)?;

        if should_refresh_token(tokens.expires_at_unix) {
            match refresh_tokens(&tokens).await {
                Ok(refreshed) => {
                    if let Err(e) = save_tokens(&refreshed).await {
                        tracing::warn!("failed to persist refreshed codex tokens: {e}");
                    }
                    tokens = refreshed;
                }
                Err(e) => {
                    tracing::warn!(
                        expires_at_unix = ?tokens.expires_at_unix,
                        "codex token refresh failed, using existing token: {e}",
                    );
                }
            }
        }

        Ok(LlmCredential {
            base_url: "https://api.openai.com/v1".to_owned(),
            api_key:  tokens.access_token,
        })
    }
}

fn codex_client_id() -> String {
    std::env::var(CODEX_CLIENT_ID_ENV).unwrap_or_else(|_| CODEX_CLIENT_ID.to_owned())
}

async fn send_token_request(form: &[(&str, &str)], context: &str) -> Result<TokenResponse> {
    let ctx = context.to_owned();
    let form_body = reqwest::Url::parse_with_params("https://localhost.invalid", form)
        .map_err(|e| CodexOauthError::TokenRequestEncode {
            context: ctx.clone(),
            reason:  e.to_string(),
        })?
        .query()
        .unwrap_or_default()
        .to_owned();
    let response = reqwest::Client::new()
        .post(CODEX_TOKEN_ENDPOINT)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body)
        .send()
        .await
        .context(TokenRequestSnafu { context: &ctx })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unavailable>".to_owned());
        return TokenRequestStatusSnafu {
            context: ctx,
            status,
            body,
        }
        .fail();
    }
    response
        .json::<TokenResponse>()
        .await
        .context(TokenResponseParseSnafu { context: ctx })
}
