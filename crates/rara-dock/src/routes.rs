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

//! Axum HTTP route handlers for the Dock API.
//!
//! These handlers expose the dock session store over HTTP for the frontend
//! canvas workbench.

use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::json;
use snafu::ResultExt;
use tracing::{debug, warn};

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
    /// Shared mutation sink — dock tools push mutations here during
    /// execution; the turn handler drains them after the agent loop.
    pub mutation_sink: crate::tools::DockMutationSink,
    /// Guard preventing concurrent turns on the same dock session.
    /// Contains session IDs with in-flight turns.
    pub in_flight:     Arc<Mutex<HashSet<String>>>,
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

    let timestamp = chrono::Utc::now().to_rfc3339();

    let state = rara_kernel::memory::HandoffState {
        summary: Some(format!("{input_preview} → {reply_preview}")),
        owner: Some("agent".into()),
        extra: Some(json!({
            "dock_turn": true,
            "input_preview": input_preview,
            "reply_preview": reply_preview,
            "timestamp": timestamp,
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
                    .and_then(|e| e.get("timestamp"))
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

/// Find a specific anchor by name and extract its snapshot (blocks + facts).
async fn find_anchor_snapshot(
    tape_service: &rara_kernel::memory::TapeService,
    session_id: &str,
    anchor_name: &str,
) -> Option<DockCanvasSnapshot> {
    let tape_name = dock_tape_name(session_id);
    let anchors = tape_service.anchors(&tape_name, 100).await.ok()?;

    anchors
        .into_iter()
        .find(|a| a.name == anchor_name)
        .and_then(|a| {
            a.state
                .get("extra")
                .and_then(|e| e.get("snapshot"))
                .and_then(|s| serde_json::from_value::<DockCanvasSnapshot>(s.clone()).ok())
        })
}

// ---------------------------------------------------------------------------
// Kernel session helpers
// ---------------------------------------------------------------------------

/// Ensure a kernel session and channel binding exist for a dock session.
///
/// Dock session IDs are ULIDs, but the kernel uses UUID-based
/// `SessionKey`s.  We derive a deterministic UUID so the dock handler
/// can predict the key and subscribe to streams after `ingest()`.
async fn ensure_dock_kernel_session(
    kernel: &rara_kernel::handle::KernelHandle,
    dock_session_id: &str,
) -> Result<rara_kernel::session::SessionKey, crate::DockError> {
    use rara_kernel::session::{ChannelBinding, SessionEntry, SessionKey};

    let session_key = SessionKey::deterministic(dock_session_id);

    let index = kernel.session_index();

    if index
        .get_session(&session_key)
        .await
        .context(crate::error::KernelSessionSnafu)?
        .is_some()
    {
        return Ok(session_key);
    }

    let now = chrono::Utc::now();
    let entry = SessionEntry {
        key:            session_key,
        title:          Some(format!("Dock: {dock_session_id}")),
        model:          None,
        thinking_level: None,
        system_prompt:  None,
        message_count:  0,
        preview:        None,
        metadata:       None,
        created_at:     now,
        updated_at:     now,
    };
    index
        .create_session(&entry)
        .await
        .context(crate::error::KernelSessionSnafu)?;

    let binding = ChannelBinding {
        channel_type: rara_kernel::channel::types::ChannelType::Web,
        chat_id: dock_session_id.to_string(),
        thread_id: None,
        session_key,
        created_at: now,
        updated_at: now,
    };
    index
        .bind_channel(&binding)
        .await
        .context(crate::error::KernelSessionSnafu)?;

    Ok(session_key)
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
///
/// When `selected_anchor` is provided, the handler restores the historical
/// canvas snapshot from that anchor instead of returning current state.
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

    // When a history anchor is selected, restore the snapshot from that
    // anchor rather than returning the current persisted state.
    let (blocks, facts) = if let (Some(anchor_name), Some(tape_svc)) =
        (&query.selected_anchor, &state.tape_service)
    {
        match find_anchor_snapshot(tape_svc, &query.session_id, anchor_name).await {
            Some(snapshot) => (snapshot.blocks, snapshot.facts),
            None => (doc.blocks, doc.facts),
        }
    } else {
        (doc.blocks, doc.facts)
    };

    Ok(Json(DockSessionResponse {
        session: doc.session,
        annotations: doc.annotations,
        history,
        selected_anchor: query.selected_anchor,
        blocks,
        facts,
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

/// `POST /api/dock/turn` — agent turn (SSE streaming).
///
/// Dispatches the turn to the kernel via `ingest()`, then returns an SSE
/// stream that forwards real-time events (`text_delta`, `tool_call_start`,
/// `tool_call_end`) and concludes with a `dock_turn_complete` event
/// carrying the authoritative post-turn canvas state.
///
/// When no kernel is available (tests), returns a plain JSON response with
/// current state.
async fn turn_handler(
    State(state): State<DockRouterState>,
    Json(body): Json<DockTurnRequest>,
) -> DockResult<Response> {
    // Reject concurrent turns on the same session to prevent mutation
    // mixing in the shared DockMutationSink.
    //
    // The RAII guard is created immediately after insertion so that ANY
    // early return (test-mode, session-setup error, ingest error) will
    // automatically remove the session from the in-flight set.
    struct InFlightGuard(Arc<Mutex<HashSet<String>>>, String);
    impl Drop for InFlightGuard {
        fn drop(&mut self) {
            let mut guard = self.0.lock();
            guard.remove(&self.1);
        }
    }
    let in_flight_guard = {
        let mut guard = state.in_flight.lock();
        if !guard.insert(body.session_id.clone()) {
            return Err(crate::DockError::Kernel {
                message: "a turn is already in progress for this session".into(),
            });
        }
        InFlightGuard(state.in_flight.clone(), body.session_id.clone())
    };

    // Use persisted server-side document as source of truth for the prompt,
    // not the client-supplied state which may be stale in multi-tab scenarios.
    let doc = state.store.ensure_session(&body.session_id)?;

    let system_prompt = build_dock_system_prompt(&doc.facts);
    let user_prompt = build_dock_user_prompt(
        &body.content,
        &doc.blocks,
        &doc.annotations,
        body.selected_anchor.as_deref(),
    );

    let input_preview = body.content.chars().take(80).collect::<String>();

    // Without a kernel, return current state (test mode).
    let Some(ref kernel) = state.kernel_handle else {
        debug!(
            session_id = %body.session_id,
            "dock turn recorded (no kernel — test mode)"
        );
        return Ok(Json(DockTurnResponse {
            session_id:      body.session_id,
            reply:           String::new(),
            mutations:       Vec::new(),
            history:         Vec::new(),
            selected_anchor: body.selected_anchor,
            session:         Some(doc.session),
            annotations:     doc.annotations,
            blocks:          doc.blocks,
            facts:           doc.facts,
        })
        .into_response());
    };

    // Ensure kernel session + channel binding for this dock session.
    let session_key = ensure_dock_kernel_session(kernel, &body.session_id).await?;

    // Build and ingest the message into the kernel agent loop.
    let combined_content = format!("{system_prompt}\n\n{user_prompt}");
    let raw = rara_kernel::io::RawPlatformMessage {
        channel_type:        rara_kernel::channel::types::ChannelType::Web,
        platform_message_id: Some(ulid::Ulid::new().to_string()),
        platform_user_id:    "dock".to_owned(),
        platform_chat_id:    Some(body.session_id.clone()),
        content:             rara_kernel::channel::types::MessageContent::Text(combined_content),
        reply_context:       Some(rara_kernel::io::ReplyContext {
            thread_id:                None,
            reply_to_platform_msg_id: None,
            interaction_type:         rara_kernel::io::InteractionType::Message,
        }),
        metadata:            HashMap::new(),
    };

    kernel
        .ingest(raw)
        .await
        .context(crate::error::KernelIngestSnafu)?;

    debug!(
        session_id = %body.session_id,
        input_len = body.content.len(),
        "dock turn dispatched to kernel"
    );

    // Return an SSE stream that forwards kernel events and emits
    // dock_turn_complete once the agent loop finishes.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    let hub = Arc::clone(kernel.stream_hub());
    let store = state.store.clone();
    let tape_service = state.tape_service.clone();
    let mutation_sink = state.mutation_sink.clone();
    let session_id = body.session_id.clone();

    tokio::spawn(async move {
        // Move the RAII guard into the spawned task so it is released
        // when the stream-forwarding task exits.
        let _guard = in_flight_guard;

        // Poll until the kernel opens a stream for this session.
        let mut attempts = 0;
        let subs = loop {
            let s = hub.subscribe_session(&session_key);
            if !s.is_empty() || attempts > 50 {
                break s;
            }
            attempts += 1;
            tokio::time::sleep(Duration::from_millis(50)).await;
        };

        if subs.is_empty() {
            warn!(session_id = %session_id, "no streams found for dock session");
            let _ = tx
                .send(Ok(Event::default()
                    .event("error")
                    .json_data(json!({"error": "no stream available"}))
                    .unwrap()))
                .await;
            return;
        }

        let mut reply_text = String::new();

        for (_, mut rx_stream) in subs {
            while let Ok(event) = rx_stream.recv().await {
                match &event {
                    rara_kernel::io::StreamEvent::TextDelta { text } => {
                        reply_text.push_str(text);
                        let _ = tx
                            .send(Ok(Event::default()
                                .event("text_delta")
                                .json_data(json!({"text": text}))
                                .unwrap()))
                            .await;
                    }
                    rara_kernel::io::StreamEvent::ToolCallStart {
                        name,
                        id,
                        arguments,
                    } => {
                        let _ = tx
                            .send(Ok(Event::default()
                                .event("tool_call_start")
                                .json_data(json!({"name": name, "id": id, "arguments": arguments}))
                                .unwrap()))
                            .await;
                    }
                    rara_kernel::io::StreamEvent::ToolCallEnd {
                        id,
                        result_preview,
                        success,
                        error,
                    } => {
                        let _ = tx
                            .send(Ok(Event::default()
                                .event("tool_call_end")
                                .json_data(json!({
                                    "id": id,
                                    "result_preview": result_preview,
                                    "success": success,
                                    "error": error,
                                }))
                                .unwrap()))
                            .await;
                    }
                    // ToolOutput is a live preview of tool execution (e.g. bash
                    // stdout). Dock SSE does not forward it yet — the canvas
                    // only needs final results. Can be added as a follow-up.
                    rara_kernel::io::StreamEvent::ToolOutput { .. } => {}
                    _ => {}
                }
            }
        }

        // Stream closed — agent turn complete.
        // Drain full mutations from the shared sink (tools push here
        // during execute, bypassing the truncated result_preview).
        let mutations = mutation_sink.drain(&session_key);
        if !mutations.is_empty() {
            if let Err(e) = store.apply_mutations(&session_id, &mutations) {
                warn!(error = %e, "failed to apply dock mutations after turn");
            }
        }

        // Reload authoritative state.
        let doc = match store.ensure_session(&session_id) {
            Ok(d) => d,
            Err(e) => {
                warn!(error = %e, "failed to reload dock session after turn");
                return;
            }
        };

        // Write post-turn tape anchor with final canvas state.
        if let Some(ref tape_svc) = tape_service {
            let snapshot = DockCanvasSnapshot {
                blocks: doc.blocks.clone(),
                facts:  doc.facts.clone(),
            };
            let reply_preview: String = reply_text.chars().take(80).collect();
            write_dock_anchor(
                tape_svc,
                &session_id,
                &input_preview,
                &reply_preview,
                &snapshot,
            )
            .await;
        }

        let history = if let Some(ref tape_svc) = tape_service {
            read_dock_history(tape_svc, &session_id, None).await
        } else {
            Vec::new()
        };

        // Emit the final dock_turn_complete event with authoritative state.
        // Always reset selected_anchor to null so the frontend exits
        // history-viewing mode after a live turn completes.
        let _ = tx
            .send(Ok(Event::default()
                .event("dock_turn_complete")
                .json_data(json!({
                    "session_id": session_id,
                    "reply": reply_text,
                    "mutations": mutations,
                    "blocks": doc.blocks,
                    "facts": doc.facts,
                    "annotations": doc.annotations,
                    "history": history,
                    "selected_anchor": null,
                    "session": doc.session,
                }))
                .unwrap()))
            .await;

        let _ = tx.send(Ok(Event::default().event("done").data(""))).await;
    });

    let stream = futures::stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|item| (item, rx))
    });

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
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
