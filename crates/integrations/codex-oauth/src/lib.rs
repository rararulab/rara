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

//! Shared Codex OAuth integration primitives.
//!
//! This crate centralizes all provider-specific OAuth behavior:
//! - OAuth URL construction and PKCE helpers
//! - Authorization-code and refresh-token exchanges
//! - Token persistence in keyring
//! - Token-expiry/refresh policy
//!
//! Callers in other layers should keep only orchestration logic.
//! For example:
//! - `backend-admin` should map HTTP requests/responses and call this crate.
//! - `workers` should load/refresh/persist tokens through this crate.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rara_keyring_store::{DefaultKeyringStore, KeyringStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// OpenAI authorization endpoint for Codex OAuth.
pub const CODEX_AUTH_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
/// OpenAI token endpoint for Codex OAuth.
pub const CODEX_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
/// Public client id used by this application for Codex OAuth.
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Requested OAuth scopes for Codex provider integration.
pub const CODEX_SCOPES: &str = "openid profile email offline_access";
/// Environment variable used to build callback URLs.
pub const PUBLIC_BASE_URL_ENV: &str = "RARA_PUBLIC_BASE_URL";
const REFRESH_SKEW_SECS: u64 = 60;
const CODEX_KEYRING_SERVICE: &str = "rara-ai-codex";
const CODEX_TOKEN_ACCOUNT: &str = "tokens";
const CODEX_PENDING_ACCOUNT: &str = "oauth-pending";

/// Persisted Codex credentials (keyring-backed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCodexTokens {
    pub access_token:    String,
    pub refresh_token:   Option<String>,
    pub id_token:        Option<String>,
    pub expires_at_unix: Option<u64>,
}

/// Temporary OAuth state stored between `/start` and `/callback`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingCodexOAuth {
    pub state:         String,
    pub code_verifier: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token:  String,
    refresh_token: Option<String>,
    id_token:      Option<String>,
    expires_in:    Option<u64>,
}

/// Build the external OAuth callback URI.
///
/// Defaults to `http://localhost:8000` when `RARA_PUBLIC_BASE_URL` is unset.
pub fn callback_uri() -> String {
    let base =
        std::env::var(PUBLIC_BASE_URL_ENV).unwrap_or_else(|_| "http://localhost:8000".into());
    format!(
        "{}/api/v1/ai/codex/oauth/callback",
        base.trim_end_matches('/')
    )
}

/// Construct the full authorization URL for redirecting the user.
pub fn build_auth_url(
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> Result<String, String> {
    let mut url = reqwest::Url::parse(CODEX_AUTH_ENDPOINT).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", CODEX_CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", CODEX_SCOPES)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
    Ok(url.into())
}

/// Load persisted Codex tokens from keyring.
pub fn load_tokens() -> Result<Option<StoredCodexTokens>, String> {
    let store = DefaultKeyringStore;
    let Some(raw) = store
        .load(CODEX_KEYRING_SERVICE, CODEX_TOKEN_ACCOUNT)
        .map_err(|e| format!("keyring load failed: {e}"))?
    else {
        return Ok(None);
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Save Codex tokens to keyring.
pub fn save_tokens(tokens: &StoredCodexTokens) -> Result<(), String> {
    let store = DefaultKeyringStore;
    let raw = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
    store
        .save(CODEX_KEYRING_SERVICE, CODEX_TOKEN_ACCOUNT, &raw)
        .map_err(|e| format!("keyring save failed: {e}"))
}

/// Delete persisted Codex tokens from keyring.
pub fn clear_tokens() -> Result<(), String> {
    let store = DefaultKeyringStore;
    let _ = store
        .delete(CODEX_KEYRING_SERVICE, CODEX_TOKEN_ACCOUNT)
        .map_err(|e| format!("keyring delete failed: {e}"))?;
    Ok(())
}

/// Load pending OAuth state from keyring.
pub fn load_pending_oauth() -> Result<Option<PendingCodexOAuth>, String> {
    let store = DefaultKeyringStore;
    let Some(raw) = store
        .load(CODEX_KEYRING_SERVICE, CODEX_PENDING_ACCOUNT)
        .map_err(|e| format!("keyring load failed: {e}"))?
    else {
        return Ok(None);
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Save pending OAuth state to keyring.
pub fn save_pending_oauth(pending: &PendingCodexOAuth) -> Result<(), String> {
    let store = DefaultKeyringStore;
    let raw = serde_json::to_string(pending).map_err(|e| e.to_string())?;
    store
        .save(CODEX_KEYRING_SERVICE, CODEX_PENDING_ACCOUNT, &raw)
        .map_err(|e| format!("keyring save failed: {e}"))
}

/// Clear pending OAuth state from keyring.
pub fn clear_pending_oauth() -> Result<(), String> {
    let store = DefaultKeyringStore;
    let _ = store
        .delete(CODEX_KEYRING_SERVICE, CODEX_PENDING_ACCOUNT)
        .map_err(|e| format!("keyring delete failed: {e}"))?;
    Ok(())
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
pub fn validate_state(expected: &str, actual: Option<&str>) -> Result<(), String> {
    let Some(actual) = actual else {
        return Err("missing oauth state".to_owned());
    };
    if expected.is_empty() {
        return Err("missing expected oauth state".to_owned());
    }
    if expected != actual {
        return Err("oauth state mismatch".to_owned());
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
    redirect_uri: &str,
) -> Result<StoredCodexTokens, String> {
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", CODEX_CLIENT_ID),
        ("code", code),
        ("redirect_uri", redirect_uri),
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
pub async fn refresh_tokens(current: &StoredCodexTokens) -> Result<StoredCodexTokens, String> {
    let refresh_token = current
        .refresh_token
        .as_deref()
        .ok_or_else(|| "codex token expired and no refresh token is available".to_owned())?;
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", CODEX_CLIENT_ID),
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

async fn send_token_request(form: &[(&str, &str)], context: &str) -> Result<TokenResponse, String> {
    let form_body = reqwest::Url::parse_with_params("https://localhost.invalid", form)
        .map_err(|e| format!("failed to encode {context} payload: {e}"))?
        .query()
        .unwrap_or_default()
        .to_owned();
    let response = reqwest::Client::new()
        .post(CODEX_TOKEN_ENDPOINT)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body)
        .send()
        .await
        .map_err(|e| format!("{context} request failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unavailable>".to_owned());
        return Err(format!("{context} failed: {status} {body}"));
    }
    response
        .json::<TokenResponse>()
        .await
        .map_err(|e| format!("failed to parse {context} response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_state_rejects_mismatch() {
        let err = validate_state("expected", Some("other")).expect_err("should fail");
        assert!(err.contains("state"));
    }

    #[test]
    fn validate_state_accepts_exact_match() {
        let result = validate_state("same", Some("same"));
        assert!(result.is_ok());
    }

    #[test]
    fn compute_expires_at_adds_offset() {
        assert_eq!(compute_expires_at_unix(1000, Some(120)), Some(1120));
        assert_eq!(compute_expires_at_unix(1000, None), None);
    }
}
