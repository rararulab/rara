//! Axum HTTP route handlers for the Dock API.
//!
//! These handlers expose the dock session store over HTTP for the frontend
//! canvas workbench.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::Deserialize;

use crate::{
    DockBootstrapResponse, DockMutationBatch, DockSessionCreateRequest, DockSessionResponse,
    DockTurnRequest, DockTurnResponse, DockWorkspaceUpdateRequest, store::DockSessionStore,
};

// ---------------------------------------------------------------------------
// Router state
// ---------------------------------------------------------------------------

/// Shared state for dock route handlers.
#[derive(Clone)]
pub struct DockRouterState {
    pub store: Arc<DockSessionStore>,
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

/// Convert a [`crate::DockError`] into an axum response.
impl IntoResponse for crate::DockError {
    fn into_response(self) -> Response {
        let status = match &self {
            crate::DockError::InvalidSessionId { .. } => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = serde_json::json!({ "error": self.to_string() });
        (status, Json(body)).into_response()
    }
}

/// Internal result type that maps [`crate::DockError`] to an axum response.
type DockResult<T> = std::result::Result<T, crate::DockError>;

// ---------------------------------------------------------------------------
// Query parameter types
// ---------------------------------------------------------------------------

/// Query parameters for `GET /api/dock/session`.
#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    pub session_id:      String,
    #[serde(default)]
    pub selected_anchor: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/dock/bootstrap` — list sessions + active session.
async fn bootstrap_handler(State(state): State<DockRouterState>) -> DockResult<Response> {
    let docs = state.store.list_sessions()?;
    let workspace = state.store.load_workspace()?;

    let sessions = docs.into_iter().map(|d| d.session).collect();

    Ok(Json(DockBootstrapResponse {
        sessions,
        active_session_id: workspace.active_session_id,
    })
    .into_response())
}

/// `GET /api/dock/session` — load session state.
async fn session_handler(
    State(state): State<DockRouterState>,
    Query(query): Query<SessionQuery>,
) -> DockResult<Response> {
    let doc = state.store.ensure_session(&query.session_id)?;

    Ok(Json(DockSessionResponse {
        session:         doc.session,
        annotations:     doc.annotations,
        history:         Vec::new(),
        selected_anchor: query.selected_anchor,
        blocks:          Vec::new(),
        facts:           doc.facts,
    })
    .into_response())
}

/// `POST /api/dock/sessions` — create a new session.
async fn create_session_handler(
    State(state): State<DockRouterState>,
    Json(body): Json<DockSessionCreateRequest>,
) -> DockResult<Response> {
    let id = body.id.unwrap_or_else(|| ulid::Ulid::new().to_string());
    let title = body.title.as_deref().unwrap_or("Untitled");
    let doc = state.store.create_session(&id, title)?;

    Ok((StatusCode::CREATED, Json(doc.session)).into_response())
}

/// `POST /api/dock/sessions/:session_id/mutate` — apply human mutations.
async fn mutate_handler(
    State(state): State<DockRouterState>,
    Path(session_id): Path<String>,
    Json(body): Json<DockMutationBatch>,
) -> DockResult<Response> {
    let doc = state.store.apply_mutations(&session_id, &body.mutations)?;

    Ok(Json(DockSessionResponse {
        session:         doc.session,
        annotations:     doc.annotations,
        history:         Vec::new(),
        selected_anchor: None,
        blocks:          Vec::new(),
        facts:           doc.facts,
    })
    .into_response())
}

/// `POST /api/dock/turn` — agent turn (stub).
///
/// Full kernel integration will be wired in a future PR. For now this
/// returns an empty turn response so the API contract is established.
async fn turn_handler(
    State(state): State<DockRouterState>,
    Json(body): Json<DockTurnRequest>,
) -> DockResult<Response> {
    // Ensure the session exists.
    let doc = state.store.ensure_session(&body.session_id)?;

    Ok(Json(DockTurnResponse {
        session_id:      body.session_id,
        reply:           String::new(),
        mutations:       Vec::new(),
        history:         Vec::new(),
        selected_anchor: body.selected_anchor,
        session:         Some(doc.session),
        annotations:     doc.annotations,
        blocks:          body.blocks,
        facts:           doc.facts,
    })
    .into_response())
}

/// `PATCH /api/dock/workspace` — update the active session.
async fn update_workspace_handler(
    State(state): State<DockRouterState>,
    Json(body): Json<DockWorkspaceUpdateRequest>,
) -> DockResult<Response> {
    let ws = crate::models::DockWorkspaceState {
        active_session_id: body.active_session_id,
    };
    state.store.save_workspace(&ws)?;
    Ok(Json(serde_json::json!({ "ok": true })).into_response())
}

// ---------------------------------------------------------------------------
// Router constructor
// ---------------------------------------------------------------------------

/// Build the dock API router with all endpoints.
///
/// Mount this into the main application router:
/// ```rust,ignore
/// app.merge(dock_router(state))
/// ```
pub fn dock_router(state: DockRouterState) -> Router {
    Router::new()
        .route("/api/dock/bootstrap", get(bootstrap_handler))
        .route("/api/dock/session", get(session_handler))
        .route("/api/dock/sessions", post(create_session_handler))
        .route(
            "/api/dock/sessions/{session_id}/mutate",
            post(mutate_handler),
        )
        .route("/api/dock/turn", post(turn_handler))
        .route("/api/dock/workspace", patch(update_workspace_handler))
        .with_state(state)
}
