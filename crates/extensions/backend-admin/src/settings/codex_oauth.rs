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

use axum::{Json, extract::State, http::StatusCode};
use rara_codex_oauth::{
    PendingCodexOAuth, build_auth_url, clear_pending_oauth, clear_tokens,
    generate_code_challenge, generate_code_verifier, generate_nonce, load_tokens,
    save_pending_oauth, start_callback_server,
};
use serde::Serialize;
use tracing::warn;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::settings::SettingsSvc;

// Note: this module intentionally stays thin.
// Provider-specific OAuth/token logic lives in `rara-codex-oauth`.
//
// The callback is handled by an ephemeral server on localhost:1455
// (required by OpenAI's registered redirect_uri), NOT by the main
// backend server. So we only expose /start, /status, /disconnect here.
pub(super) fn routes() -> OpenApiRouter<SettingsSvc> {
    OpenApiRouter::new().nest(
        "/api/v1/ai/codex/oauth",
        OpenApiRouter::new()
            .routes(routes!(oauth_start))
            .routes(routes!(oauth_status))
            .routes(routes!(oauth_disconnect)),
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

    let auth_url = build_auth_url(&oauth_state, &code_challenge)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let pending = PendingCodexOAuth {
        state: oauth_state,
        code_verifier,
    };
    save_pending_oauth(&pending).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Start ephemeral callback server on localhost:1455 (OpenAI's
    // pre-registered redirect_uri). It handles a single callback then
    // shuts itself down.
    start_callback_server()
        .await
        .map_err(|e| {
            warn!(error = %e, "failed to start codex oauth callback server");
            (StatusCode::INTERNAL_SERVER_ERROR, e)
        })?;

    Ok(Json(OAuthStartResponse { auth_url }))
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
