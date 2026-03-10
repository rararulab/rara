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

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::{
    detector::UpdateState,
    notifier::UpdateNotifier,
    supervisor::{SupervisorHandle, SupervisorStatus},
    trigger_manual_update,
};

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Application state shared across all admin HTTP handlers.
#[derive(Clone)]
pub struct GatewayAppState {
    pub supervisor_handle: SupervisorHandle,
    pub update_state_rx: watch::Receiver<UpdateState>,
    pub notifier: Arc<UpdateNotifier>,
    pub shutdown: CancellationToken,
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
struct GatewayUpdateResponse {
    ok: bool,
    updated: bool,
    message: String,
    current_rev: String,
    target_rev: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn get_status(State(state): State<GatewayAppState>) -> Json<GatewayStatusResponse> {
    let agent = state.supervisor_handle.status();
    let update = state.update_state_rx.borrow().clone();

    Json(GatewayStatusResponse {
        agent,
        update: UpdateStatusResponse {
            current_rev: update.current_rev,
            upstream_rev: update.upstream_rev,
            update_available: update.update_available,
            last_check_time: update.last_check_time.map(|t| t.to_rfc3339()),
        },
    })
}

async fn post_restart(State(state): State<GatewayAppState>) -> Json<OkResponse> {
    let _ = state.supervisor_handle.restart().await;
    Json(OkResponse { ok: true })
}

async fn post_update(
    State(state): State<GatewayAppState>,
) -> (StatusCode, Json<GatewayUpdateResponse>) {
    match trigger_manual_update(&state.supervisor_handle, state.notifier.as_ref()).await {
        Ok(result) => (
            StatusCode::OK,
            Json(GatewayUpdateResponse {
                ok: true,
                updated: result.updated,
                message: result.message,
                current_rev: result.current_rev,
                target_rev: result.target_rev,
            }),
        ),
        Err(message) => (
            if message.contains("already in progress") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            Json(GatewayUpdateResponse {
                ok: false,
                updated: false,
                message,
                current_rev: String::new(),
                target_rev: None,
            }),
        ),
    }
}

async fn post_shutdown(State(state): State<GatewayAppState>) -> Json<OkResponse> {
    state.shutdown.cancel();
    Json(OkResponse { ok: true })
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
