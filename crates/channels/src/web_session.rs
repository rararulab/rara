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

//! Persistent per-session WebSocket endpoint — the single channel that
//! replaces the legacy split between [`crate::web`]'s per-turn chat WS
//! and [`crate::web_session_events`]'s tape-events WS.
//!
//! # Why
//!
//! The split design routed in-turn stream deltas (`text_delta`,
//! `tool_call_*`, `done`) over one socket and out-of-band tape mutations
//! (`tape_appended`) over a second socket. Order across the two sockets
//! is undefined, which produced the cross-WS race classes traced in
//! #1601, #1731, #1732, #1849, #1867, #1877, #1880, #1923. Folding both
//! event sources into a single ordered mpsc on one socket makes
//! `done`-then-`tape_appended` deterministic by construction.
//!
//! # Endpoint
//!
//! `GET /session/{session_key}` — WebSocket upgrade. Auth identical to
//! the legacy chat WS: prefer `Authorization: Bearer <owner-token>`,
//! fall back to `?token=<…>` query string for browser upgrades. Verified
//! against [`rara_kernel::auth::verify_owner_token`].
//!
//! # Frame contract
//!
//! - Outbound: [`crate::web::WebEvent`] (extended with [`Hello`] +
//!   [`TapeAppended`]).
//! - Inbound: [`InboundFrame`] — `{"type":"prompt", ...}` or
//!   `{"type":"abort"}`.
//!
//! # Phase scope
//!
//! This is phase (a) of #1935: **scaffold + hello + drain + forwarders**.
//! The endpoint is mounted alongside the legacy `/ws` so existing
//! frontends keep working unchanged. Inbound `prompt` is **rejected**
//! with an error frame so a half-built endpoint cannot accidentally
//! serve traffic; `abort` is accepted but only logged. Full inbound
//! handling lands in phase (b).
//!
//! [`Hello`]: crate::web::WebEvent::Hello
//! [`TapeAppended`]: crate::web::WebEvent::TapeAppended

use std::sync::Arc;

use axum::{
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use rara_kernel::{
    notification::{KernelNotification, NotificationFilter},
    session::SessionKey,
};
use serde::Deserialize;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};

use crate::web::{
    WebAdapter, WebAdapterState, WebEvent, bearer_token_from_headers, register_endpoint,
    stream_event_to_web_event, unregister_endpoint,
};

/// Query parameters for the persistent session WS endpoint.
///
/// Identical shape to the legacy chat WS: only an optional `token`
/// fallback for clients that cannot set an `Authorization` header. The
/// session key is taken from the path, never from query, and identity
/// comes from the server-trusted `WebAdapterState::owner_user_id`
/// post-auth — never from any client field.
#[derive(Debug, Deserialize)]
pub struct TokenQuery {
    /// Owner-token fallback for browser WebSocket upgrades that cannot
    /// set `Authorization: Bearer <token>`.
    #[serde(default)]
    pub token: Option<String>,
}

/// Inbound frames the persistent WS accepts from the client.
///
/// Phase (a) only handles `Abort` (logged, no-op until phase b). `Prompt`
/// is parsed so the contract is locked in but rejected at runtime with
/// an error frame — see module docs for why.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InboundFrame {
    /// User submitted a new message. Phase (b) will route this through
    /// `transcribe_audio_blocks` + `build_raw_platform_message` +
    /// `KernelHandle::submit_message`.
    Prompt {
        content: rara_kernel::channel::types::MessageContent,
    },
    /// User clicked stop. Phase (b) will dispatch
    /// `Signal::Interrupt` against the kernel session.
    Abort,
}

/// Axum handler for `GET /session/{session_key}`.
pub(crate) async fn session_ws_handler(
    ws: WebSocketUpgrade,
    Path(session_key): Path<String>,
    Query(query): Query<TokenQuery>,
    headers: axum::http::HeaderMap,
    State(state): State<WebAdapterState>,
) -> Response {
    let header_token = bearer_token_from_headers(&headers);
    let query_token = query.token.as_deref().filter(|t| !t.is_empty());
    let provided = header_token.or(query_token);
    match provided {
        Some(tok) if rara_kernel::auth::verify_owner_token(&state.owner_token, tok) => {
            info!(%session_key, "persistent session WS auth via owner token");
        }
        Some(_) => {
            warn!(%session_key, "invalid owner token on persistent session WS");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("invalid token"))
                .expect("static unauthorized response");
        }
        None => {
            warn!(%session_key, "missing owner token on persistent session WS");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("missing token"))
                .expect("static unauthorized response");
        }
    }

    let key = match SessionKey::try_from_raw(&session_key) {
        Ok(k) => k,
        Err(e) => {
            warn!(%session_key, error = %e, "invalid session key on persistent session WS");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::BAD_REQUEST)
                .body(axum::body::Body::from("invalid session key"))
                .expect("static bad request response");
        }
    };

    ws.on_upgrade(move |socket| handle_session_ws(socket, session_key, key, state))
}

/// Run the persistent per-session WS connection until the client closes,
/// the server shuts down, or one of the forwarders exits.
///
/// Three forwarder tasks fan into a single mpsc so order is preserved
/// across event sources:
///
/// 1. Adapter-local broadcast bus — `Typing` / `Error` / `Phase` / egress
///    replies.
/// 2. Kernel `StreamHub::subscribe_session_events` — in-turn `text_delta`,
///    `tool_call_*`, `done`, etc.
/// 3. Kernel notification bus filtered to this session — `tape_appended` on
///    every tape write, both in-turn (post-`done`) and out-of-turn (background
///    tasks, scheduled re-entries).
///
/// The reply buffer is drained atomically with the adapter-bus subscribe
/// so a reconnect within the TTL window replays "important" events that
/// fired while no listener was attached (#1804 invariant).
async fn handle_session_ws(
    socket: WebSocket,
    session_key_str: String,
    session_key: SessionKey,
    state: WebAdapterState,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    register_endpoint(
        &state.endpoint_registry,
        &state.owner_user_id,
        &session_key_str,
    )
    .await;

    let (ws_event_tx, mut ws_event_rx) = mpsc::unbounded_channel::<WebEvent>();

    // Send the initial `hello` so the client knows the socket is live.
    // Sequenced before any forwarder spawn so it always lands first.
    if ws_event_tx.send(WebEvent::Hello).is_err() {
        return;
    }

    let adapter_bus = WebAdapter::get_or_create_adapter_bus(&state.adapter_events, session_key);

    // Atomic subscribe + buffer drain. Holding the per-session mutex
    // across both ops means no event reaches this WS twice (live
    // broadcast and snapshot both observed) and no event goes missing
    // (publish strictly between subscribe and drain lands on the
    // broadcast). See `web_reply_buffer` module docs.
    let (mut adapter_rx, backlog) = state
        .reply_buffer
        .subscribe_and_drain(&session_key, &adapter_bus);
    if !backlog.is_empty() {
        debug!(
            session_key = %session_key_str,
            count = backlog.len(),
            "draining web reply buffer to new persistent session WS"
        );
        for ev in backlog {
            if ws_event_tx.send(ev).is_err() {
                return;
            }
        }
    }

    let adapter_forwarder = {
        let ws_event_tx = ws_event_tx.clone();
        let skey = session_key_str.clone();
        tokio::spawn(async move {
            loop {
                match adapter_rx.recv().await {
                    Ok(ev) => {
                        if ws_event_tx.send(ev).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(session_key = %skey, skipped = n, "session WS adapter bus lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    let stream_forwarder = {
        let ws_event_tx = ws_event_tx.clone();
        let stream_hub = Arc::clone(&state.stream_hub);
        let skey = session_key_str.clone();
        tokio::spawn(async move {
            let hub = {
                let guard = stream_hub.read().await;
                match guard.as_ref() {
                    Some(h) => Arc::clone(h),
                    None => return,
                }
            };
            let mut rx = hub.subscribe_session_events(&session_key);
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let Some(web_event) = stream_event_to_web_event(event) else {
                            continue;
                        };
                        if ws_event_tx.send(web_event).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(
                            session_key = %skey,
                            skipped = n,
                            "session WS stream bus lagged"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    // Forwarder 3: kernel notification bus → per-WS mpsc, filtered to
    // `TapeAppended` for this session. The kernel publishes
    // `TapeAppended` *after* the DB write in
    // `crates/kernel/src/memory/service.rs`, so for any in-turn append
    // the natural emit order across the kernel's own buses is
    // `StreamClosed` (→ `Done`) then `TapeAppended`. Funnelling both
    // into one ordered mpsc preserves that order on the wire.
    let notification_forwarder = {
        let ws_event_tx = ws_event_tx.clone();
        let sink = Arc::clone(&state.sink);
        let target = session_key;
        let skey = session_key_str.clone();
        tokio::spawn(async move {
            let handle = {
                let guard = sink.read().await;
                match guard.as_ref() {
                    Some(h) => h.clone(),
                    None => return,
                }
            };
            let mut subscription = handle
                .notification_bus()
                .subscribe(NotificationFilter::default())
                .await;
            loop {
                match subscription.recv().await {
                    Ok(KernelNotification::TapeAppended {
                        session_key,
                        entry_id,
                        role,
                        timestamp,
                    }) if session_key == target => {
                        let frame = WebEvent::TapeAppended {
                            entry_id,
                            role,
                            timestamp: timestamp.to_string(),
                        };
                        if ws_event_tx.send(frame).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(
                            session_key = %skey,
                            skipped = n,
                            "session WS notification bus lagged"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    // Drop the extra sender held here so when all forwarders exit the
    // mpsc receiver sees a clean close and the send task terminates.
    drop(ws_event_tx);

    let mut shutdown_rx = state.shutdown_rx.clone();

    let send_task = {
        let session_key_str = session_key_str.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = ws_event_rx.recv() => {
                        let Some(event) = msg else { break; };
                        let json = match serde_json::to_string(&event) {
                            Ok(j) => j,
                            Err(e) => {
                                error!(
                                    session_key = %session_key_str,
                                    error = %e,
                                    "serialize web event on session WS"
                                );
                                continue;
                            }
                        };
                        if ws_tx.send(Message::Text(json.into())).await.is_err() {
                            debug!(
                                session_key = %session_key_str,
                                "session WS send failed, closing"
                            );
                            break;
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        debug!(
                            session_key = %session_key_str,
                            "session WS shutdown signal received"
                        );
                        break;
                    }
                }
            }
        })
    };

    let recv_task = {
        let adapter_events = Arc::clone(&state.adapter_events);
        let reply_buffer = state.reply_buffer.clone();
        let session_key_str = session_key_str.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_rx.next().await {
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => {
                        debug!(session_key = %session_key_str, "client closed session WS");
                        break;
                    }
                    _ => continue,
                };

                if text.trim().is_empty() {
                    continue;
                }

                match serde_json::from_str::<InboundFrame>(&text) {
                    Ok(InboundFrame::Abort) => {
                        // Phase (a): log only. Phase (b) will dispatch
                        // `Signal::Interrupt` against the kernel session.
                        info!(
                            session_key = %session_key_str,
                            "session WS received abort (phase-a no-op)"
                        );
                    }
                    Ok(InboundFrame::Prompt { .. }) => {
                        // Phase (a): the prompt path is not implemented
                        // yet. Reject loudly so the endpoint cannot serve
                        // traffic accidentally — better a visible error
                        // than a silently dropped message.
                        warn!(
                            session_key = %session_key_str,
                            "session WS received prompt during phase-a scaffold; rejecting"
                        );
                        WebAdapter::publish_adapter_event(
                            &adapter_events,
                            &reply_buffer,
                            &session_key,
                            WebEvent::Error {
                                message: "persistent session WS prompt handling not yet \
                                          implemented (phase b)"
                                    .to_owned(),
                            },
                        );
                    }
                    Err(e) => {
                        warn!(
                            session_key = %session_key_str,
                            error = %e,
                            "failed to parse inbound frame on session WS"
                        );
                        WebAdapter::publish_adapter_event(
                            &adapter_events,
                            &reply_buffer,
                            &session_key,
                            WebEvent::Error {
                                message: format!("invalid frame: {e}"),
                            },
                        );
                    }
                }
            }
        })
    };

    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
    adapter_forwarder.abort();
    stream_forwarder.abort();
    notification_forwarder.abort();

    unregister_endpoint(
        &state.endpoint_registry,
        &state.owner_user_id,
        &session_key_str,
    )
    .await;
    info!(session_key = %session_key_str, "persistent session WS closed");
}
