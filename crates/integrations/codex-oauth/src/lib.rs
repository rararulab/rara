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
//! - Short-lived pending OAuth state persistence
//! - Token-expiry/refresh policy
//! - Ephemeral local callback server on port 1455
//!
//! The Codex public OAuth client (`app_EMoamEEZ73f0CkXaXp7hrann`) only
//! accepts `http://localhost:1455/auth/callback` as its redirect URI.
//! We therefore spin up a one-shot axum server on that port to capture
//! the authorization code, exchange it for tokens, and redirect the
//! browser to the frontend settings page.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rara_keyring_store::{DefaultKeyringStore, KeyringStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
/// Local port that the ephemeral callback server binds to.
pub const CODEX_CALLBACK_PORT: u16 = 1455;
/// Environment variable for the frontend base URL used for post-OAuth redirects.
/// Falls back to `http://localhost:5173`.
pub const FRONTEND_BASE_URL_ENV: &str = "RARA_FRONTEND_URL";
/// Environment variable used to override OAuth client id.
pub const CODEX_CLIENT_ID_ENV: &str = "RARA_CODEX_CLIENT_ID";
const REFRESH_SKEW_SECS: u64 = 60;
const CODEX_KEYRING_SERVICE: &str = "rara-ai-codex";
const CODEX_TOKEN_ACCOUNT: &str = "tokens";
static PENDING_OAUTH: std::sync::LazyLock<std::sync::Mutex<Option<PendingCodexOAuth>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

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

/// Construct the full authorization URL for redirecting the user.
///
/// Uses the fixed redirect URI `http://localhost:1455/auth/callback` that is
/// pre-registered with the Codex public OAuth client.
pub fn build_auth_url(
    state: &str,
    code_challenge: &str,
) -> Result<String, String> {
    let client_id = codex_client_id();
    let mut url = reqwest::Url::parse(CODEX_AUTH_ENDPOINT).map_err(|e| e.to_string())?;
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

/// Load pending OAuth state from in-process temporary storage.
pub fn load_pending_oauth() -> Result<Option<PendingCodexOAuth>, String> {
    let guard = PENDING_OAUTH
        .lock()
        .map_err(|_| "pending oauth lock poisoned".to_owned())?;
    Ok(guard.clone())
}

/// Save pending OAuth state to in-process temporary storage.
pub fn save_pending_oauth(pending: &PendingCodexOAuth) -> Result<(), String> {
    let mut guard = PENDING_OAUTH
        .lock()
        .map_err(|_| "pending oauth lock poisoned".to_owned())?;
    *guard = Some(pending.clone());
    Ok(())
}

/// Clear pending OAuth state from in-process temporary storage.
pub fn clear_pending_oauth() -> Result<(), String> {
    let mut guard = PENDING_OAUTH
        .lock()
        .map_err(|_| "pending oauth lock poisoned".to_owned())?;
    *guard = None;
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
) -> Result<StoredCodexTokens, String> {
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
pub async fn refresh_tokens(current: &StoredCodexTokens) -> Result<StoredCodexTokens, String> {
    let refresh_token = current
        .refresh_token
        .as_deref()
        .ok_or_else(|| "codex token expired and no refresh token is available".to_owned())?;
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

/// Build the frontend base URL used for post-OAuth redirects.
///
/// Priority: `RARA_FRONTEND_URL` > `http://localhost:5173`
pub fn frontend_base_url() -> String {
    std::env::var(FRONTEND_BASE_URL_ENV)
        .unwrap_or_else(|_| "http://localhost:5173".into())
        .trim_end_matches('/')
        .to_owned()
}

// ---------------------------------------------------------------------------
// Ephemeral callback server
// ---------------------------------------------------------------------------

/// Start a one-shot HTTP server on `localhost:1455` that waits for the OAuth
/// callback, exchanges the code for tokens, saves them, and redirects the
/// browser to the frontend settings page.
///
/// Returns `Ok(())` after successfully handling the callback or an error
/// description on failure. The server shuts itself down after the first
/// request to `/auth/callback`.
pub async fn start_callback_server() -> Result<(), String> {
    use std::sync::Arc;
    use tokio::sync::oneshot;

    let (tx, rx) = oneshot::channel::<Result<(), String>>();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));

    let app = axum::Router::new().route(
        "/auth/callback",
        axum::routing::get({
            let tx = Arc::clone(&tx);
            move |query: axum::extract::Query<CallbackQuery>| {
                let tx = Arc::clone(&tx);
                async move { handle_callback(query, tx).await }
            }
        }),
    );

    let addr: std::net::SocketAddr = ([127, 0, 0, 1], CODEX_CALLBACK_PORT).into();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("failed to bind callback server on {addr}: {e}"))?;

    tracing::info!("codex oauth callback server listening on {addr}");

    // Serve until the callback is handled.
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                // Wait for callback handler to signal completion.
                let _ = rx.await;
                tracing::info!("codex oauth callback server shutting down");
            })
            .await
            .ok();
    });

    Ok(())
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code:  Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn handle_callback(
    axum::extract::Query(query): axum::extract::Query<CallbackQuery>,
    tx: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Result<(), String>>>>>,
) -> axum::response::Redirect {
    let frontend = frontend_base_url();
    let err_url = format!("{frontend}/settings?section=providers&codex_oauth=error");
    let ok_url = format!("{frontend}/settings?section=providers&codex_oauth=success");

    let result = handle_callback_inner(&query).await;
    let redirect_url = match &result {
        Ok(()) => &ok_url,
        Err(e) => {
            tracing::warn!(error = %e, "codex oauth callback failed");
            &err_url
        }
    };

    // Signal the server to shut down (fire-and-forget).
    if let Ok(mut guard) = tx.lock() {
        if let Some(tx) = guard.take() {
            let _ = tx.send(result);
        }
    }

    axum::response::Redirect::to(redirect_url)
}

async fn handle_callback_inner(query: &CallbackQuery) -> Result<(), String> {
    if let Some(ref oauth_err) = query.error {
        return Err(format!("provider returned error: {oauth_err}"));
    }

    let pending = load_pending_oauth()?
        .ok_or_else(|| "no pending oauth state found".to_owned())?;

    validate_state(&pending.state, query.state.as_deref())?;

    let code = query
        .code
        .as_deref()
        .ok_or_else(|| "missing authorization code".to_owned())?;

    let tokens = exchange_authorization_code(code, &pending.code_verifier).await?;
    save_tokens(&tokens)?;
    clear_pending_oauth()?;

    tracing::info!("codex oauth tokens saved successfully");
    Ok(())
}

fn codex_client_id() -> String {
    std::env::var(CODEX_CLIENT_ID_ENV).unwrap_or_else(|_| CODEX_CLIENT_ID.to_owned())
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

    #[test]
    fn build_auth_url_includes_required_params() {
        let url = build_auth_url("test-state", "test-challenge").unwrap();
        assert!(url.contains("id_token_add_organizations=true"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("redirect_uri=http"));
        assert!(url.contains("localhost%3A1455"));
        assert!(url.contains("code_challenge_method=S256"));
    }
}
