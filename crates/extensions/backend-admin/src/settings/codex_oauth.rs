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

use axum::{Json, extract::State, http::StatusCode, response::Redirect};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rara_keyring_store::{DefaultKeyringStore, KeyringStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::settings::SettingsSvc;

const CODEX_AUTH_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_SCOPES: &str = "openid profile email offline_access";
const CODEX_SUCCESS_REDIRECT: &str = "/settings?section=providers&codex_oauth=success";
const CODEX_ERROR_REDIRECT: &str = "/settings?section=providers&codex_oauth=error";
const PUBLIC_BASE_URL_ENV: &str = "RARA_PUBLIC_BASE_URL";
const CODEX_KEYRING_SERVICE: &str = "rara-ai-codex";
const CODEX_TOKEN_ACCOUNT: &str = "tokens";
const CODEX_PENDING_ACCOUNT: &str = "oauth-pending";

pub(super) fn routes() -> OpenApiRouter<SettingsSvc> {
    OpenApiRouter::new().nest(
        "/api/v1/ai/codex/oauth",
        OpenApiRouter::new()
            .routes(routes!(oauth_start))
            .routes(routes!(oauth_status))
            .routes(routes!(oauth_disconnect))
            .route("/callback", axum::routing::get(oauth_callback)),
    )
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct OAuthStartResponse {
    pub auth_url: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct OAuthStatusResponse {
    pub connected:       bool,
    pub expires_at_unix: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    code:  Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token:  String,
    refresh_token: Option<String>,
    id_token:      Option<String>,
    expires_in:    Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCodexTokens {
    pub access_token:    String,
    pub refresh_token:   Option<String>,
    pub id_token:        Option<String>,
    pub expires_at_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingCodexOAuth {
    state:         String,
    code_verifier: String,
}

#[utoipa::path(
    post,
    path = "/start",
    tag = "ai-admin",
    responses((status = 200, body = OAuthStartResponse))
)]
async fn oauth_start(
    State(_state): State<SettingsSvc>,
) -> Result<Json<OAuthStartResponse>, (StatusCode, String)> {
    let oauth_state = generate_nonce();
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let redirect_uri = callback_uri();

    let auth_url = build_auth_url(&redirect_uri, &oauth_state, &code_challenge)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let pending = PendingCodexOAuth {
        state: oauth_state,
        code_verifier,
    };
    save_pending_oauth(&pending).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(OAuthStartResponse { auth_url }))
}

async fn oauth_callback(
    State(_state): State<SettingsSvc>,
    axum::extract::Query(query): axum::extract::Query<OAuthCallbackQuery>,
) -> Redirect {
    if query.error.is_some() {
        return Redirect::to(CODEX_ERROR_REDIRECT);
    }

    let Some(pending) = load_pending_oauth().ok().flatten() else {
        return Redirect::to(CODEX_ERROR_REDIRECT);
    };
    if validate_state(&pending.state, query.state.as_deref()).is_err() {
        return Redirect::to(CODEX_ERROR_REDIRECT);
    }
    let Some(code) = query.code.as_deref() else {
        return Redirect::to(CODEX_ERROR_REDIRECT);
    };

    let token_response = match exchange_code(code, &pending.code_verifier, &callback_uri()).await {
        Ok(token_response) => token_response,
        Err(_) => return Redirect::to(CODEX_ERROR_REDIRECT),
    };

    let tokens = StoredCodexTokens {
        access_token:    token_response.access_token,
        refresh_token:   token_response.refresh_token,
        id_token:        token_response.id_token,
        expires_at_unix: compute_expires_at_unix(now_unix(), token_response.expires_in),
    };
    if save_tokens(&tokens).is_err() {
        return Redirect::to(CODEX_ERROR_REDIRECT);
    }
    let _ = clear_pending_oauth();

    Redirect::to(CODEX_SUCCESS_REDIRECT)
}

#[utoipa::path(
    get,
    path = "/status",
    tag = "ai-admin",
    responses((status = 200, body = OAuthStatusResponse))
)]
async fn oauth_status(State(_state): State<SettingsSvc>) -> Json<OAuthStatusResponse> {
    let tokens = load_tokens().ok().flatten();
    Json(OAuthStatusResponse {
        connected:       tokens.is_some(),
        expires_at_unix: tokens.and_then(|v| v.expires_at_unix),
    })
}

#[utoipa::path(
    post,
    path = "/disconnect",
    tag = "ai-admin",
    responses((status = 200, body = OAuthStatusResponse))
)]
async fn oauth_disconnect(
    State(_state): State<SettingsSvc>,
) -> Result<Json<OAuthStatusResponse>, (StatusCode, String)> {
    clear_tokens().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    clear_pending_oauth().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(OAuthStatusResponse {
        connected:       false,
        expires_at_unix: None,
    }))
}

fn callback_uri() -> String {
    let base =
        std::env::var(PUBLIC_BASE_URL_ENV).unwrap_or_else(|_| "http://localhost:8000".into());
    format!(
        "{}/api/v1/ai/codex/oauth/callback",
        base.trim_end_matches('/')
    )
}

fn build_auth_url(redirect_uri: &str, state: &str, code_challenge: &str) -> Result<String, String> {
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

async fn exchange_code(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, String> {
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", CODEX_CLIENT_ID),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ];
    let form_body = reqwest::Url::parse_with_params("https://localhost.invalid", form)
        .map_err(|e| format!("failed to encode oauth payload: {e}"))?
        .query()
        .unwrap_or_default()
        .to_owned();
    let client = reqwest::Client::new();
    let response = client
        .post(CODEX_TOKEN_ENDPOINT)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body)
        .send()
        .await
        .map_err(|e| format!("oauth token exchange request failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unavailable>".to_owned());
        return Err(format!("oauth token exchange failed: {status} {body}"));
    }

    response
        .json::<TokenResponse>()
        .await
        .map_err(|e| format!("failed to parse oauth token response: {e}"))
}

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

pub fn save_tokens(tokens: &StoredCodexTokens) -> Result<(), String> {
    let store = DefaultKeyringStore;
    let raw = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
    store
        .save(CODEX_KEYRING_SERVICE, CODEX_TOKEN_ACCOUNT, &raw)
        .map_err(|e| format!("keyring save failed: {e}"))
}

fn clear_tokens() -> Result<(), String> {
    let store = DefaultKeyringStore;
    let _ = store
        .delete(CODEX_KEYRING_SERVICE, CODEX_TOKEN_ACCOUNT)
        .map_err(|e| format!("keyring delete failed: {e}"))?;
    Ok(())
}

fn load_pending_oauth() -> Result<Option<PendingCodexOAuth>, String> {
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

fn save_pending_oauth(pending: &PendingCodexOAuth) -> Result<(), String> {
    let store = DefaultKeyringStore;
    let raw = serde_json::to_string(pending).map_err(|e| e.to_string())?;
    store
        .save(CODEX_KEYRING_SERVICE, CODEX_PENDING_ACCOUNT, &raw)
        .map_err(|e| format!("keyring save failed: {e}"))
}

fn clear_pending_oauth() -> Result<(), String> {
    let store = DefaultKeyringStore;
    let _ = store
        .delete(CODEX_KEYRING_SERVICE, CODEX_PENDING_ACCOUNT)
        .map_err(|e| format!("keyring delete failed: {e}"))?;
    Ok(())
}

fn generate_nonce() -> String { uuid::Uuid::new_v4().simple().to_string() }

fn generate_code_verifier() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn generate_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn validate_state(expected: &str, actual: Option<&str>) -> Result<(), String> {
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

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn compute_expires_at_unix(now_unix: u64, expires_in_secs: Option<u64>) -> Option<u64> {
    expires_in_secs.map(|v| now_unix.saturating_add(v))
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
