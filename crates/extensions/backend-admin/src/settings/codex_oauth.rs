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
use rara_domain_shared::settings::codex_oauth::{
    CODEX_CLIENT_ID, CODEX_TOKEN_ENDPOINT, PendingCodexOAuth, StoredCodexTokens, build_auth_url,
    callback_uri, clear_pending_oauth, clear_tokens, compute_expires_at_unix,
    generate_code_challenge, generate_code_verifier, generate_nonce, load_pending_oauth,
    load_tokens, now_unix, save_pending_oauth, save_tokens, validate_state,
};
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::settings::SettingsSvc;

const CODEX_SUCCESS_REDIRECT: &str = "/settings?section=providers&codex_oauth=success";
const CODEX_ERROR_REDIRECT: &str = "/settings?section=providers&codex_oauth=error";

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
