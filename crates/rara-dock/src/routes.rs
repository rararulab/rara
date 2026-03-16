//! Axum HTTP route handlers for the Dock API.
//!
//! These handlers expose the dock session store over HTTP for the frontend
//! canvas workbench.

use std::{collections::HashMap, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::Deserialize;
use serde_json::json;
use tracing::{debug, error, warn};

use crate::{
    DockBootstrapResponse, DockCanvasSnapshot, DockHistoryEntry, DockMutationBatch,
    DockSessionCreateRequest, DockSessionResponse, DockTurnRequest, DockTurnResponse,
    DockWorkspaceUpdateRequest,
    state::{build_dock_system_prompt, build_dock_user_prompt},
    store::DockSessionStore,
};

// ---------------------------------------------------------------------------
// Router state
// ---------------------------------------------------------------------------

/// Shared state for dock route handlers.
#[derive(Clone)]
pub struct DockRouterState {
    pub store:         Arc<DockSessionStore>,
    /// Optional tape service for writing anchors and reading history.
    /// `None` during unit tests or when the kernel is not available.
    pub tape_service:  Option<rara_kernel::memory::TapeService>,
    /// Optional kernel handle for dispatching agent turns via `ingest()`.
    /// `None` during unit tests or standalone mode.
    pub kernel_handle: Option<rara_kernel::handle::KernelHandle>,
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
// Tape anchor helpers
// ---------------------------------------------------------------------------

/// Canonical tape name for a dock session's history anchors.
fn dock_tape_name(session_id: &str) -> String { format!("dock:{session_id}") }

/// Write a tape anchor capturing the dock turn snapshot.
async fn write_dock_anchor(
    tape_service: &rara_kernel::memory::TapeService,
    session_id: &str,
    input_preview: &str,
    reply_preview: &str,
    snapshot: &DockCanvasSnapshot,
) {
    let tape_name = dock_tape_name(session_id);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let anchor_name = format!("dock/turn/{now_ms}");

    let state = rara_kernel::memory::HandoffState {
        summary: Some(format!("{input_preview} → {reply_preview}")),
        owner: Some("agent".into()),
        extra: Some(json!({
            "dock_turn": true,
            "input_preview": input_preview,
            "reply_preview": reply_preview,
            "snapshot": snapshot,
        })),
        ..Default::default()
    };

    if let Err(e) = tape_service.handoff(&tape_name, &anchor_name, state).await {
        warn!(session_id, error = %e, "failed to write dock tape anchor");
    }
}

/// Read dock history entries from tape anchors for a session.
async fn read_dock_history(
    tape_service: &rara_kernel::memory::TapeService,
    session_id: &str,
    selected_anchor: Option<&str>,
) -> Vec<DockHistoryEntry> {
    let tape_name = dock_tape_name(session_id);

    let anchors = match tape_service.anchors(&tape_name, 100).await {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };

    anchors
        .into_iter()
        .filter(|a| {
            // Dock metadata is stored in HandoffState.extra, which serializes
            // as a nested "extra" object inside the anchor state.
            a.state
                .get("extra")
                .and_then(|e| e.get("dock_turn"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .map(|a| {
            let extra = a.state.get("extra");
            let is_selected = selected_anchor.is_some_and(|sel| sel == a.name);
            DockHistoryEntry {
                id: a.name.clone(),
                anchor_name: a.name,
                timestamp: extra
                    .and_then(|e| e.get("input_preview"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                label: extra
                    .and_then(|e| e.get("input_preview"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("turn")
                    .to_string(),
                preview: extra
                    .and_then(|e| e.get("reply_preview"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                state: a.state,
                is_selected,
            }
        })
        .collect()
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

    // Read dock history from tape if available.
    let history = if let Some(ref tape_svc) = state.tape_service {
        read_dock_history(
            tape_svc,
            &query.session_id,
            query.selected_anchor.as_deref(),
        )
        .await
    } else {
        Vec::new()
    };

    Ok(Json(DockSessionResponse {
        session: doc.session,
        annotations: doc.annotations,
        history,
        selected_anchor: query.selected_anchor,
        blocks: doc.blocks,
        facts: doc.facts,
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
        blocks:          doc.blocks,
        facts:           doc.facts,
    })
    .into_response())
}

/// `POST /api/dock/turn` — agent turn.
///
/// Builds dock system/user prompts from the request and dispatches the turn
/// to the kernel via `ingest()`.  The kernel runs the agent loop
/// asynchronously — dock tool calls produce `DockMutation`s that are emitted
/// as `StreamEvent::DockTurnComplete` and forwarded to the frontend via SSE.
///
/// The handler returns immediately with `202 Accepted` and current session
/// state.  The authoritative post-turn state arrives via SSE once the agent
/// loop completes.
async fn turn_handler(
    State(state): State<DockRouterState>,
    Json(body): Json<DockTurnRequest>,
) -> DockResult<Response> {
    let doc = state.store.ensure_session(&body.session_id)?;

    // Build prompts — the user prompt embeds canvas context so the agent
    // sees the current dock state.
    let system_prompt = build_dock_system_prompt(&body.facts);
    let user_prompt = build_dock_user_prompt(
        &body.content,
        &body.blocks,
        &body.annotations,
        body.selected_anchor.as_deref(),
    );

    let input_preview = body.content.chars().take(80).collect::<String>();

    // Dispatch to the kernel agent loop if available.
    if let Some(ref kernel) = state.kernel_handle {
        // Combine dock system prompt and user prompt into the message
        // content so the agent sees full canvas context.
        let combined_content = format!("{system_prompt}\n\n{user_prompt}");

        let raw = rara_kernel::io::RawPlatformMessage {
            channel_type:        rara_kernel::channel::types::ChannelType::Web,
            platform_message_id: Some(ulid::Ulid::new().to_string()),
            platform_user_id:    "dock".to_owned(),
            platform_chat_id:    Some(body.session_id.clone()),
            content:             rara_kernel::channel::types::MessageContent::Text(
                combined_content,
            ),
            reply_context:       Some(rara_kernel::io::ReplyContext {
                thread_id:                None,
                reply_to_platform_msg_id: None,
                interaction_type:         rara_kernel::io::InteractionType::Message,
            }),
            metadata:            HashMap::new(),
        };

        if let Err(e) = kernel.ingest(raw).await {
            error!(
                session_id = %body.session_id,
                error = %e,
                "failed to ingest dock turn into kernel"
            );
            return Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("kernel ingest failed: {e}") })),
            )
                .into_response());
        }

        debug!(
            session_id = %body.session_id,
            input_len = body.content.len(),
            "dock turn dispatched to kernel"
        );
    } else {
        debug!(
            session_id = %body.session_id,
            "dock turn recorded (no kernel — test mode)"
        );
    }

    // Write tape anchor capturing the turn input.
    let snapshot = DockCanvasSnapshot {
        blocks: body.blocks.clone(),
        facts:  doc.facts.clone(),
    };
    if let Some(ref tape_svc) = state.tape_service {
        write_dock_anchor(tape_svc, &body.session_id, &input_preview, "", &snapshot).await;
    }

    // Read history for the response.
    let history = if let Some(ref tape_svc) = state.tape_service {
        read_dock_history(tape_svc, &body.session_id, body.selected_anchor.as_deref()).await
    } else {
        Vec::new()
    };

    // Return 202 Accepted with current state — the authoritative post-turn
    // state will arrive via SSE when the agent loop completes.
    Ok((
        StatusCode::ACCEPTED,
        Json(DockTurnResponse {
            session_id: body.session_id,
            reply: String::new(),
            mutations: Vec::new(),
            history,
            selected_anchor: body.selected_anchor,
            session: Some(doc.session),
            annotations: doc.annotations,
            blocks: doc.blocks,
            facts: doc.facts,
        }),
    )
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
