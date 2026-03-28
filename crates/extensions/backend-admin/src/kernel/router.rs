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
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::get,
};
use futures::StreamExt;
use rara_kernel::{handle::KernelHandle, session::SessionKey};

use super::problem::ProblemDetails;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn kernel_routes(handle: KernelHandle) -> Router {
    Router::new()
        .route("/api/v1/kernel/stats", get(get_stats))
        .route("/api/v1/kernel/sessions", get(list_sessions))
        .route(
            "/api/v1/kernel/sessions/{session_key}/turns",
            get(get_session_turns),
        )
        .route(
            "/api/v1/kernel/sessions/{session_key}/stream",
            get(stream_session),
        )
        .route("/api/v1/kernel/events/stream", get(stream_kernel_events))
        .route("/api/v1/kernel/approvals", get(list_approvals))
        .with_state(handle)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn get_stats(
    State(handle): State<KernelHandle>,
) -> Result<Json<rara_kernel::session::SystemStats>, ProblemDetails> {
    Ok(Json(handle.system_stats()))
}

async fn list_sessions(
    State(handle): State<KernelHandle>,
) -> Result<Json<Vec<rara_kernel::session::SessionStats>>, ProblemDetails> {
    Ok(Json(handle.list_processes()))
}

async fn get_session_turns(
    State(handle): State<KernelHandle>,
    Path(session_key): Path<String>,
) -> Result<Json<Vec<rara_kernel::agent::TurnTrace>>, ProblemDetails> {
    let key = SessionKey::try_from_raw(&session_key)
        .map_err(|e| ProblemDetails::bad_request(format!("invalid session_key: {e}")))?;
    // Verify the session exists before returning traces.
    if !handle.process_table().contains(&key) {
        return Err(ProblemDetails::not_found(
            "Session Not Found",
            format!("session not found: {session_key}"),
        ));
    }
    Ok(Json(handle.get_process_turns(key)))
}

async fn list_approvals(
    State(handle): State<KernelHandle>,
) -> Result<Json<Vec<rara_kernel::security::ApprovalRequest>>, ProblemDetails> {
    Ok(Json(handle.security().approval().list_pending()))
}

async fn stream_session(
    State(handle): State<KernelHandle>,
    Path(session_key): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, ProblemDetails> {
    let key = SessionKey::try_from_raw(&session_key)
        .map_err(|e| ProblemDetails::bad_request(format!("invalid session_key: {e}")))?;

    if !handle.process_table().contains(&key) {
        return Err(ProblemDetails::not_found(
            "Session Not Found",
            format!("session not found: {session_key}"),
        ));
    }
    let stream_hub = handle.stream_hub().clone();

    Ok(ws.on_upgrade(move |socket| handle_session_stream(socket, stream_hub, key)))
}

async fn stream_kernel_events(
    State(_handle): State<KernelHandle>,
) -> Sse<impl futures::Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    // TODO: re-implement via EventBus once kernel event observation is added back.
    let stream = futures::stream::empty().boxed();

    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn handle_session_stream(
    mut socket: WebSocket,
    stream_hub: std::sync::Arc<rara_kernel::io::StreamHub>,
    session_key: SessionKey,
) {
    use tokio::time::Duration;

    let mut poll_interval = tokio::time::interval(Duration::from_millis(200));
    let mut receivers: Vec<tokio::sync::broadcast::Receiver<rara_kernel::io::StreamEvent>> =
        Vec::new();

    loop {
        // If no active receivers, try to subscribe
        if receivers.is_empty() {
            let subs = stream_hub.subscribe_session(&session_key);
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
                    tracing::warn!(lagged = n, "session stream subscriber lagged");
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
