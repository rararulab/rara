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

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    routing::get,
};
use rara_kernel::{KernelHandle, audit::AuditFilter};
use serde::Deserialize;

use super::problem::ProblemDetails;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    pub limit: usize,
}

fn default_audit_limit() -> usize { 50 }

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn kernel_routes(handle: KernelHandle) -> Router {
    Router::new()
        .route("/api/v1/kernel/stats", get(get_stats))
        .route("/api/v1/kernel/processes", get(list_processes))
        .route(
            "/api/v1/kernel/processes/{agent_id}/turns",
            get(get_process_turns),
        )
        .route(
            "/api/v1/kernel/processes/{agent_id}/stream",
            get(stream_process),
        )
        .route("/api/v1/kernel/approvals", get(list_approvals))
        .route("/api/v1/kernel/audit", get(query_audit))
        .with_state(handle)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn get_stats(
    State(handle): State<KernelHandle>,
) -> Result<Json<rara_kernel::process::SystemStats>, ProblemDetails> {
    Ok(Json(handle.system_stats()))
}

async fn list_processes(
    State(handle): State<KernelHandle>,
) -> Result<Json<Vec<rara_kernel::process::ProcessStats>>, ProblemDetails> {
    Ok(Json(handle.list_processes().await))
}

async fn get_process_turns(
    State(handle): State<KernelHandle>,
    Path(agent_id): Path<String>,
) -> Result<Json<Vec<rara_kernel::agent_turn::TurnTrace>>, ProblemDetails> {
    let aid = rara_kernel::process::AgentId(
        uuid::Uuid::parse_str(&agent_id)
            .map_err(|e| ProblemDetails::bad_request(format!("invalid agent_id: {e}")))?,
    );
    // Verify the process exists before returning traces.
    if handle.process_table().get(aid).is_none() {
        return Err(ProblemDetails::not_found(
            "Process Not Found",
            format!("process not found: {agent_id}"),
        ));
    }
    Ok(Json(handle.get_process_turns(aid)))
}

async fn list_approvals(
    State(handle): State<KernelHandle>,
) -> Result<Json<Vec<rara_kernel::approval::ApprovalRequest>>, ProblemDetails> {
    Ok(Json(handle.security().approval().list_pending()))
}

async fn query_audit(
    State(handle): State<KernelHandle>,
    Query(params): Query<AuditQuery>,
) -> Result<Json<Vec<rara_kernel::audit::AuditEvent>>, ProblemDetails> {
    let filter = AuditFilter {
        limit: params.limit,
        ..Default::default()
    };
    Ok(Json(handle.audit_query(filter).await))
}

async fn stream_process(
    State(handle): State<KernelHandle>,
    Path(agent_id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, ProblemDetails> {
    let aid = rara_kernel::process::AgentId(
        uuid::Uuid::parse_str(&agent_id)
            .map_err(|e| ProblemDetails::bad_request(format!("invalid agent_id: {e}")))?,
    );

    let process = handle.process_table().get(aid).ok_or_else(|| {
        ProblemDetails::not_found(
            "Process Not Found",
            format!("process not found: {agent_id}"),
        )
    })?;
    let session_id = process.session_id.clone();
    let stream_hub = handle.stream_hub().clone();

    Ok(ws.on_upgrade(move |socket| handle_process_stream(socket, stream_hub, session_id)))
}

async fn handle_process_stream(
    mut socket: WebSocket,
    stream_hub: std::sync::Arc<rara_kernel::io::stream::StreamHub>,
    session_id: rara_kernel::process::SessionId,
) {
    use tokio::time::Duration;

    let mut poll_interval = tokio::time::interval(Duration::from_millis(200));
    let mut receivers: Vec<tokio::sync::broadcast::Receiver<rara_kernel::io::stream::StreamEvent>> =
        Vec::new();

    loop {
        // If no active receivers, try to subscribe
        if receivers.is_empty() {
            let subs = stream_hub.subscribe_session(&session_id);
            receivers = subs.into_iter().map(|(_, rx)| rx).collect();
            if receivers.is_empty() {
                // No active stream — wait and retry
                tokio::select! {
                    _ = poll_interval.tick() => continue,
                    msg = socket.recv() => {
                        match msg {
                            Some(Ok(Message::Close(_))) | None => return,
                            _ => continue,
                        }
                    }
                }
            }
        }

        // Drain from all receivers
        let mut got_event = false;
        for rx in &mut receivers {
            match rx.try_recv() {
                Ok(event) => {
                    got_event = true;
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    let _ = socket
                        .send(Message::Text(r#"{"type":"done"}"#.into()))
                        .await;
                    return;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "process stream subscriber lagged");
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
            }
        }

        if !got_event {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(50)) => {},
                msg = socket.recv() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => return,
                        _ => {}
                    }
                }
            }
        }
    }
}
