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

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rara_keyring_store::{DefaultKeyringStore, KeyringStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CODEX_AUTH_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
pub const CODEX_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const CODEX_SCOPES: &str = "openid profile email offline_access";
pub const PUBLIC_BASE_URL_ENV: &str = "RARA_PUBLIC_BASE_URL";
const REFRESH_SKEW_SECS: u64 = 60;
const CODEX_KEYRING_SERVICE: &str = "rara-ai-codex";
const CODEX_TOKEN_ACCOUNT: &str = "tokens";
const CODEX_PENDING_ACCOUNT: &str = "oauth-pending";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCodexTokens {
    pub access_token:    String,
    pub refresh_token:   Option<String>,
    pub id_token:        Option<String>,
    pub expires_at_unix: Option<u64>,
}

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

pub fn callback_uri() -> String {
    let base =
        std::env::var(PUBLIC_BASE_URL_ENV).unwrap_or_else(|_| "http://localhost:8000".into());
    format!(
        "{}/api/v1/ai/codex/oauth/callback",
        base.trim_end_matches('/')
    )
}

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

pub fn clear_tokens() -> Result<(), String> {
    let store = DefaultKeyringStore;
    let _ = store
        .delete(CODEX_KEYRING_SERVICE, CODEX_TOKEN_ACCOUNT)
        .map_err(|e| format!("keyring delete failed: {e}"))?;
    Ok(())
}

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

pub fn save_pending_oauth(pending: &PendingCodexOAuth) -> Result<(), String> {
    let store = DefaultKeyringStore;
    let raw = serde_json::to_string(pending).map_err(|e| e.to_string())?;
    store
        .save(CODEX_KEYRING_SERVICE, CODEX_PENDING_ACCOUNT, &raw)
        .map_err(|e| format!("keyring save failed: {e}"))
}

pub fn clear_pending_oauth() -> Result<(), String> {
    let store = DefaultKeyringStore;
    let _ = store
        .delete(CODEX_KEYRING_SERVICE, CODEX_PENDING_ACCOUNT)
        .map_err(|e| format!("keyring delete failed: {e}"))?;
    Ok(())
}

pub fn generate_nonce() -> String { uuid::Uuid::new_v4().simple().to_string() }

pub fn generate_code_verifier() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub fn generate_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

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

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn compute_expires_at_unix(now_unix: u64, expires_in_secs: Option<u64>) -> Option<u64> {
    expires_in_secs.map(|v| now_unix.saturating_add(v))
}

pub fn should_refresh_token(expires_at_unix: Option<u64>) -> bool {
    let Some(expires_at_unix) = expires_at_unix else {
        return false;
    };
    now_unix().saturating_add(REFRESH_SKEW_SECS) >= expires_at_unix
}

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
