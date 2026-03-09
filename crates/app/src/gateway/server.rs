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

//! Gateway admin HTTP server — exposes status, restart, and shutdown endpoints.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    routing::{get, post},
};
use serde::Serialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::{
    UpdateDetector,
    detector::UpdateState,
    notifier::UpdateNotifier,
    pipeline::trigger_update,
    supervisor::{SupervisorHandle, SupervisorStatus},
};

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Application state shared across all admin HTTP handlers.
#[derive(Clone)]
pub struct GatewayAppState {
    pub supervisor_handle: SupervisorHandle,
    pub update_state_rx: watch::Receiver<UpdateState>,
    pub update_state_tx: watch::Sender<UpdateState>,
    pub shutdown: CancellationToken,
    pub notifier: Arc<UpdateNotifier>,
    pub owner_token: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct GatewayStatusResponse {
    agent: SupervisorStatus,
    update: UpdateStatusResponse,
}

#[derive(Debug, Serialize)]
struct UpdateStatusResponse {
    current_rev: String,
    upstream_rev: Option<String>,
    update_available: bool,
    last_check_time: Option<String>,
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    ok: bool,
    error: String,
    detail: String,
    status: u16,
}

#[derive(Debug, Serialize)]
struct GatewayCommandResponse {
    ok: bool,
    action: String,
    status: String,
    detail: String,
    target_rev: Option<String>,
    active_rev: Option<String>,
    rolled_back: Option<bool>,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn api_error(
    status: StatusCode,
    error: &str,
    detail: impl Into<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            ok: false,
            error: error.to_owned(),
            detail: detail.into(),
            status: status.as_u16(),
        }),
    )
}

fn require_owner_token(
    headers: &HeaderMap,
    state: &GatewayAppState,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let Some(expected_token) = state
        .owner_token
        .as_deref()
        .filter(|token| !token.is_empty())
    else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "owner_token_not_configured",
            "gateway admin API requires AppConfig.owner_token to be set",
        ));
    };

    let Some(raw_header) = headers.get(header::AUTHORIZATION) else {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "missing_authorization",
            "send Authorization: Bearer <owner_token>",
        ));
    };

    let Ok(raw_header) = raw_header.to_str() else {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_authorization",
            "authorization header must be valid UTF-8",
        ));
    };

    let Some(actual_token) = raw_header.strip_prefix("Bearer ") else {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_authorization",
            "authorization header must use Bearer token auth",
        ));
    };

    if actual_token != expected_token {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "forbidden",
            "owner token did not match",
        ));
    }

    Ok(())
}

async fn get_status(
    State(state): State<GatewayAppState>,
    headers: HeaderMap,
) -> ApiResult<GatewayStatusResponse> {
    require_owner_token(&headers, &state)?;

    let agent = state.supervisor_handle.status();
    let update = state.update_state_rx.borrow().clone();

    Ok(Json(GatewayStatusResponse {
        agent,
        update: UpdateStatusResponse {
            current_rev: update.current_rev,
            upstream_rev: update.upstream_rev,
            update_available: update.update_available,
            last_check_time: update.last_check_time.map(|t| t.to_rfc3339()),
        },
    }))
}

async fn post_restart(
    State(state): State<GatewayAppState>,
    headers: HeaderMap,
) -> ApiResult<GatewayCommandResponse> {
    require_owner_token(&headers, &state)?;

    state.supervisor_handle.restart().await.map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "restart_failed",
            e.to_string(),
        )
    })?;

    Ok(Json(GatewayCommandResponse {
        ok: true,
        action: "restart".to_owned(),
        status: "accepted".to_owned(),
        detail: "restart command sent to gateway supervisor".to_owned(),
        target_rev: None,
        active_rev: None,
        rolled_back: None,
    }))
}

async fn post_update(
    State(state): State<GatewayAppState>,
    headers: HeaderMap,
) -> ApiResult<GatewayCommandResponse> {
    require_owner_token(&headers, &state)?;

    let fresh_state = UpdateDetector::probe()
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, "update_probe_failed", e))?;
    let _ = state.update_state_tx.send(fresh_state.clone());

    if !fresh_state.update_available {
        return Ok(Json(GatewayCommandResponse {
            ok: true,
            action: "update".to_owned(),
            status: "no_update".to_owned(),
            detail: format!("already up to date at {}", fresh_state.current_rev),
            target_rev: fresh_state.upstream_rev,
            active_rev: Some(fresh_state.current_rev),
            rolled_back: None,
        }));
    }

    let Some(target_rev) = fresh_state.upstream_rev.clone() else {
        return Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "missing_upstream_rev",
            "update probe reported available update without upstream revision",
        ));
    };

    let summary = trigger_update(
        &target_rev,
        &state.supervisor_handle,
        state.notifier.as_ref(),
    )
    .await
    .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, "update_failed", e))?;

    Ok(Json(GatewayCommandResponse {
        ok: summary.ok,
        action: "update".to_owned(),
        status: summary.status,
        detail: summary.detail,
        target_rev: summary.target_rev,
        active_rev: summary.active_rev,
        rolled_back: summary.rolled_back,
    }))
}

async fn post_shutdown(
    State(state): State<GatewayAppState>,
    headers: HeaderMap,
) -> ApiResult<OkResponse> {
    require_owner_token(&headers, &state)?;
    state.shutdown.cancel();
    Ok(Json(OkResponse { ok: true }))
}

// ---------------------------------------------------------------------------
// Server bootstrap
// ---------------------------------------------------------------------------

/// Build the admin [`Router`] with all gateway routes.
pub fn router(state: GatewayAppState) -> Router {
    Router::new()
        .route("/gateway/status", get(get_status))
        .route("/gateway/restart", post(post_restart))
        .route("/gateway/update", post(post_update))
        .route("/gateway/shutdown", post(post_shutdown))
        .with_state(state)
}

/// Start the gateway admin HTTP server on the given `bind_address`.
///
/// Returns a [`tokio::task::JoinHandle`] that resolves when the server exits.
pub async fn serve(
    bind_address: &str,
    state: GatewayAppState,
) -> Result<tokio::task::JoinHandle<()>, std::io::Error> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(bind_address).await?;
    info!(address = %bind_address, "Gateway admin HTTP server listening");

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "Gateway admin HTTP server error");
        }
    });

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;
    use crate::{GatewayConfig, gateway::UpdateNotifier};

    fn test_state(owner_token: Option<&str>) -> GatewayAppState {
        let config = GatewayConfig {
            check_interval: std::time::Duration::from_secs(60),
            health_timeout: 30,
            health_poll_interval: std::time::Duration::from_secs(2),
            max_restart_attempts: 3,
            auto_update: true,
            bind_address: "127.0.0.1:25556".to_owned(),
            repo_url: "https://github.com/rararulab/rara".to_owned(),
        };
        let notifier = Arc::new(UpdateNotifier::new(
            "test-token",
            1,
            "test-version",
            &config.repo_url,
        ));
        let (supervisor, supervisor_handle) =
            super::super::supervisor::SupervisorService::new(config, "25555", notifier.clone());
        std::mem::forget(supervisor);
        let initial_state = UpdateState {
            current_rev: "abc".to_owned(),
            upstream_rev: Some("abc".to_owned()),
            last_check_time: None,
            update_available: false,
        };
        let (update_state_tx, update_state_rx) = watch::channel(initial_state);

        GatewayAppState {
            supervisor_handle,
            update_state_rx,
            update_state_tx,
            shutdown: CancellationToken::new(),
            notifier,
            owner_token: owner_token.map(ToOwned::to_owned),
        }
    }

    #[tokio::test]
    async fn status_requires_bearer_token() {
        let app = router(test_state(Some("secret")));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/gateway/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn restart_accepts_matching_bearer_token() {
        let app = router(test_state(Some("secret")));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/gateway/restart")
                    .header(header::AUTHORIZATION, "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn status_rejects_when_owner_token_missing() {
        let app = router(test_state(None));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/gateway/status")
                    .header(header::AUTHORIZATION, "Bearer anything")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(parsed["error"], "owner_token_not_configured");
    }
}
