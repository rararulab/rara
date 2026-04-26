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

//! Session-events WebSocket — server-pushed notification stream that
//! survives across user turns.
//!
//! The chat WebSocket in [`crate::web`] is per-`streamFn` call: it opens
//! when the user submits a turn and closes when the kernel emits `done`.
//! That leaves the UI deaf to tape mutations that arrive outside a user
//! turn, e.g. when a background task completes and the kernel injects a
//! synthetic re-entry that produces an assistant summary (#1849).
//!
//! This module exposes a separate, lighter WS endpoint mounted at
//! `/events/{session_key}` (under the same auth as the chat WS) which
//! forwards [`KernelNotification::TapeAppended`] frames for the matching
//! session. The frame payload is intentionally minimal — the frontend
//! treats it as a refetch trigger rather than a data source.

use axum::{
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use rara_kernel::{
    handle::KernelHandle,
    notification::{KernelNotification, NotificationFilter},
    session::SessionKey,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};

/// Query parameters for the session events WS endpoint.
#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Owner token fallback when the client cannot set
    /// `Authorization: Bearer <token>`.
    #[serde(default)]
    pub token: Option<String>,
}

/// Frames sent over the session events WebSocket.
///
/// Stable, additive contract: the client matches on `type` and ignores
/// unknown variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEventFrame {
    /// Sent immediately on connect for liveness — frontend uses it to
    /// confirm the socket is established before arming reconnect logic.
    Hello,
    /// A new entry was appended to the session's tape. Clients refetch
    /// messages on receipt.
    TapeAppended {
        entry_id:  u64,
        role:      Option<String>,
        timestamp: String,
    },
}

/// State required by the session-events WS handler.
///
/// A trait object so the handler does not depend on `WebAdapter`'s
/// concrete state struct (which has many other fields irrelevant here).
#[derive(Clone)]
pub struct SessionEventsState {
    pub owner_token: String,
    /// Shared kernel handle, populated by `WebAdapter::start`.
    pub handle:      std::sync::Arc<tokio::sync::RwLock<Option<KernelHandle>>>,
}

/// Axum handler for `GET /events/{session_key}`.
pub async fn events_ws_handler(
    ws: WebSocketUpgrade,
    Path(session_key): Path<String>,
    Query(query): Query<EventsQuery>,
    headers: axum::http::HeaderMap,
    State(state): State<SessionEventsState>,
) -> Response {
    let header_token = bearer_token_from_headers(&headers);
    let query_token = query.token.as_deref().filter(|t| !t.is_empty());
    let provided = header_token.or(query_token);
    match provided {
        Some(tok) if rara_kernel::auth::verify_owner_token(&state.owner_token, tok) => {
            info!(%session_key, "session-events WS auth via owner token");
        }
        Some(_) => {
            warn!(%session_key, "invalid owner token on session-events WS");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("invalid token"))
                .expect("static unauthorized response");
        }
        None => {
            warn!(%session_key, "missing owner token on session-events WS");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("missing token"))
                .expect("static unauthorized response");
        }
    }

    let key = match SessionKey::try_from_raw(&session_key) {
        Ok(k) => k,
        Err(e) => {
            warn!(%session_key, error = %e, "invalid session key on session-events WS");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::BAD_REQUEST)
                .body(axum::body::Body::from("invalid session key"))
                .expect("static bad request response");
        }
    };

    ws.on_upgrade(move |socket| handle_events_ws(socket, key, state))
}

async fn handle_events_ws(socket: WebSocket, key: SessionKey, state: SessionEventsState) {
    let handle = {
        let guard = state.handle.read().await;
        match guard.as_ref() {
            Some(h) => h.clone(),
            None => {
                warn!(session_key = %key, "kernel handle not yet attached, closing WS");
                return;
            }
        }
    };

    let mut subscription = handle
        .notification_bus()
        .subscribe(NotificationFilter::default())
        .await;

    let (mut ws_tx, _ws_rx) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<SessionEventFrame>();

    // Send the initial `hello` so the client knows the socket is live.
    if out_tx.send(SessionEventFrame::Hello).is_err() {
        return;
    }

    // Forwarder: kernel notification bus → per-WS mpsc, filtered by session.
    let forwarder = {
        let target = key.clone();
        let out_tx = out_tx.clone();
        tokio::spawn(async move {
            loop {
                match subscription.recv().await {
                    Ok(KernelNotification::TapeAppended {
                        session_key,
                        entry_id,
                        role,
                        timestamp,
                    }) if session_key == target => {
                        let frame = SessionEventFrame::TapeAppended {
                            entry_id,
                            role,
                            timestamp: timestamp.to_string(),
                        };
                        if out_tx.send(frame).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {} // other event types ignored
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(session_key = %target, skipped = n, "session-events bus lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    drop(out_tx);

    while let Some(frame) = out_rx.recv().await {
        let json = match serde_json::to_string(&frame) {
            Ok(j) => j,
            Err(e) => {
                warn!(error = %e, "serialize session event frame");
                continue;
            }
        };
        if ws_tx.send(Message::Text(json.into())).await.is_err() {
            debug!(session_key = %key, "session-events WS send failed, closing");
            break;
        }
    }

    forwarder.abort();
}

fn bearer_token_from_headers(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
}
