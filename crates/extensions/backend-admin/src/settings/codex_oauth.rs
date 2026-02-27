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
use tracing::warn;
use rara_codex_oauth::{
    PendingCodexOAuth, build_auth_url, callback_uri, clear_pending_oauth, clear_tokens,
    exchange_authorization_code, frontend_base_url, generate_code_challenge,
    generate_code_verifier, generate_nonce, load_pending_oauth, load_tokens, save_pending_oauth,
    save_tokens, validate_state,
};
use serde::Serialize;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::settings::SettingsSvc;

fn success_redirect() -> String {
    format!("{}/settings?section=providers&codex_oauth=success", frontend_base_url())
}

fn error_redirect() -> String {
    format!("{}/settings?section=providers&codex_oauth=error", frontend_base_url())
}

// Note: this module intentionally stays thin.
// Provider-specific OAuth/token logic lives in `rara-codex-oauth`.
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

#[derive(Debug, serde::Deserialize)]
pub struct OAuthCallbackQuery {
    code:  Option<String>,
    state: Option<String>,
    error: Option<String>,
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
    let err_url = error_redirect();
    if let Some(ref oauth_err) = query.error {
        warn!(error = %oauth_err, "codex oauth callback received error from provider");
        return Redirect::to(&err_url);
    }

    let Some(pending) = load_pending_oauth().ok().flatten() else {
        warn!("codex oauth callback: no pending oauth state found");
        return Redirect::to(&err_url);
    };
    if let Err(e) = validate_state(&pending.state, query.state.as_deref()) {
        warn!(error = %e, "codex oauth callback: state validation failed");
        return Redirect::to(&err_url);
    }
    let Some(code) = query.code.as_deref() else {
        warn!("codex oauth callback: missing authorization code");
        return Redirect::to(&err_url);
    };

    // Perform the provider token exchange in integration layer, then persist.
    let tokens =
        match exchange_authorization_code(code, &pending.code_verifier, &callback_uri()).await {
            Ok(tokens) => tokens,
            Err(e) => {
                warn!(error = %e, "codex oauth token exchange failed");
                return Redirect::to(&err_url);
            }
        };
    if let Err(e) = save_tokens(&tokens) {
        warn!(error = %e, "codex oauth: failed to save tokens");
        return Redirect::to(&err_url);
    }
    let _ = clear_pending_oauth();

    Redirect::to(&success_redirect())
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
