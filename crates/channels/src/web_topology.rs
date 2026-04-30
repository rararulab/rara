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

//! Cross-session topology WebSocket endpoint
//! (`/topology/{root_session_key}`) — single multiplexed stream of every
//! `StreamEvent` emitted on a root session **and** every descendant session
//! transitively spawned from it (#1999).
//!
//! # Why a separate endpoint
//!
//! `/session/{session_key}` is per-session by construction: it subscribes
//! to one `StreamHub::subscribe_session_events` receiver tied to the
//! URL-pinned key. That socket renders inline topology markers
//! (`SubagentSpawned` / `SubagentDone` / `TapeForked` are forwarded by
//! `crate::web::stream_event_to_web_event`), but it never sees events on
//! a *child* session's bus — children have their own session keys and
//! their own buses. Without this endpoint, the multi-agent topology UI
//! would have to open one WebSocket per discovered descendant, producing
//! a fan-out that is fragile under reconnects and misses any descendant
//! spawned during the gap.
//!
//! This handler folds all of those buses into one socket: bootstrap a
//! snapshot via `process_table.children_of(root)` recursively, then watch
//! for `SubagentSpawned` events on every already-subscribed bus and add
//! the new child to the watch set on the fly.
//!
//! # Frame contract (outbound)
//!
//! Every frame is a JSON object with a `"type"` discriminator.
//! [`TopologyFrame`] enumerates the variants:
//!
//! - `hello` — first frame; carries the root key and the descendant snapshot
//!   (so the client can render the initial tree without waiting for events).
//! - `session_subscribed` — sent each time a new descendant's bus is added,
//!   typically immediately after a `SubagentSpawned` event arrives. Lets the
//!   client grow its tree in lock-step with the server's subscription set.
//! - `event` — a `StreamEvent` (mapped via the same
//!   `crate::web::stream_event_to_web_event` used by the per-session WS),
//!   tagged with the originating `session_key` so the client knows which node
//!   in the tree it belongs to.
//! - `error` — terminal frame for fatal handler errors (rare; subscriber lag is
//!   logged but not surfaced because the server keeps streaming).
//!
//! # No `unsubscribe` frame
//!
//! Subscriptions are one-way: once a descendant is added, the handler
//! keeps draining its bus until the connection drops. Trying to detect
//! "child finished, drop the subscription" introduces races against
//! trailing events (`StreamClosed`, late `TapeAppended`) for marginal
//! resource savings — a topology subtree is bounded by `process_table`'s
//! own lifecycle. The whole `HashMap<SessionKey, Receiver>` is dropped
//! when the connection closes, releasing every subscription at once.

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use rara_kernel::session::SessionKey;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};

use crate::{
    web::{WebAdapterState, WebEvent, bearer_token_from_headers, stream_event_to_web_event},
    web_session::TokenQuery,
};

/// Outbound frame sent over the cross-session topology WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TopologyFrame {
    /// Initial frame after auth + connect. Carries the root session and a
    /// snapshot of its descendants discovered at connect time so the
    /// client can render the initial tree before any live event arrives.
    Hello {
        root_session_key:    String,
        /// Descendant `(child, parent)` pairs in BFS order, including the
        /// transitive closure of `children_of`. `parent == root` for direct
        /// children.
        initial_descendants: Vec<TopologyDescendant>,
    },
    /// A new session has been added to the watch set. Sent the first time
    /// the handler subscribes to a session's bus — either because it was
    /// in the initial snapshot or because a `SubagentSpawned` event named
    /// it. Lets the client grow its tree without inferring it from
    /// per-event `session_key`s.
    SessionSubscribed {
        session_key: String,
        parent:      Option<String>,
    },
    /// A `StreamEvent` observed on one of the watched buses, mapped to a
    /// `WebEvent` (same mapping as the per-session WS) and tagged with the
    /// originating session.
    Event {
        session_key: String,
        event:       WebEvent,
    },
    /// Terminal error frame. Sent on fatal handler conditions before the
    /// socket closes. Lag on a single bus is logged but not surfaced —
    /// the handler keeps streaming.
    Error { message: String },
}

/// Descendant entry in the [`TopologyFrame::Hello`] snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyDescendant {
    pub session_key: String,
    pub parent:      String,
}

/// Axum handler for `GET /topology/{root_session_key}`.
pub(crate) async fn topology_ws_handler(
    ws: WebSocketUpgrade,
    Path(root_session_key): Path<String>,
    Query(query): Query<TokenQuery>,
    headers: axum::http::HeaderMap,
    State(state): State<WebAdapterState>,
) -> Response {
    let header_token = bearer_token_from_headers(&headers);
    let query_token = query.token.as_deref().filter(|t| !t.is_empty());
    let provided = header_token.or(query_token);
    match provided {
        Some(tok) if rara_kernel::auth::verify_owner_token(&state.owner_token, tok) => {
            info!(%root_session_key, "topology WS auth via owner token");
        }
        Some(_) => {
            warn!(%root_session_key, "invalid owner token on topology WS");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("invalid token"))
                .expect("static unauthorized response");
        }
        None => {
            warn!(%root_session_key, "missing owner token on topology WS");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("missing token"))
                .expect("static unauthorized response");
        }
    }

    let root = match SessionKey::try_from_raw(&root_session_key) {
        Ok(k) => k,
        Err(e) => {
            warn!(%root_session_key, error = %e, "invalid root session key on topology WS");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::BAD_REQUEST)
                .body(axum::body::Body::from("invalid session key"))
                .expect("static bad request response");
        }
    };

    ws.on_upgrade(move |socket| handle_topology_ws(socket, root_session_key, root, state))
}

/// Run the cross-session topology WS connection until the client closes
/// or the server shuts down.
///
/// The handler maintains a per-connection `HashMap<SessionKey, JoinHandle>`
/// of bus forwarders. Each forwarder owns one
/// `StreamHub::subscribe_session_events` receiver and pushes mapped
/// frames onto a single ordered mpsc that the send task drains.
///
/// When a `SubagentSpawned` arrives on any watched bus the handler spawns
/// a new forwarder for `child_session` (idempotent: re-spawn on a key that
/// is already watched is a no-op).
async fn handle_topology_ws(
    socket: WebSocket,
    root_session_key_str: String,
    root: SessionKey,
    state: WebAdapterState,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    let stream_hub = {
        let guard = state.stream_hub.read().await;
        match guard.as_ref() {
            Some(h) => Arc::clone(h),
            None => {
                warn!(
                    root_session_key = %root_session_key_str,
                    "topology WS rejected: stream_hub not set (adapter not started)"
                );
                let _ = send_error(&mut ws_tx, "adapter not started").await;
                return;
            }
        }
    };

    // Bootstrap descendants from the process table. Snapshot taken
    // synchronously *before* we subscribe to any bus, so any spawn that
    // races the snapshot will be observed via the live `SubagentSpawned`
    // path — duplicates are fine (the per-session subscribe is idempotent).
    let descendants = {
        let sink_guard = state.sink.read().await;
        match sink_guard.as_ref() {
            Some(handle) => collect_descendants(handle.process_table(), root),
            None => {
                // No kernel handle yet — proceed with an empty snapshot.
                // The client still receives Hello and any later live events
                // once start() runs (subscribe_session_events doesn't need
                // the kernel handle, only stream_hub).
                Vec::new()
            }
        }
    };

    let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<TopologyFrame>();

    // Hello first.
    let hello = TopologyFrame::Hello {
        root_session_key:    root_session_key_str.clone(),
        initial_descendants: descendants
            .iter()
            .map(|(child, parent)| TopologyDescendant {
                session_key: child.to_string(),
                parent:      parent.to_string(),
            })
            .collect(),
    };
    if frame_tx.send(hello).is_err() {
        return;
    }

    // Track which sessions already have a forwarder. Keyed by SessionKey
    // so re-subscribing on the same key is a no-op.
    let mut watched: HashMap<SessionKey, tokio::task::JoinHandle<()>> = HashMap::new();

    // Subscribe root + every snapshot descendant.
    spawn_forwarder(
        &mut watched,
        &stream_hub,
        &frame_tx,
        root,
        None,
        &root_session_key_str,
    );
    for (child, parent) in descendants {
        spawn_forwarder(
            &mut watched,
            &stream_hub,
            &frame_tx,
            child,
            Some(parent),
            &root_session_key_str,
        );
    }

    // Frame_tx is held by the dispatch task below + every forwarder. The
    // copy in this scope is dropped after the forwarders are spawned so
    // the receiver sees a clean close when every forwarder + dispatcher
    // is done.
    let dispatch_tx = frame_tx.clone();
    drop(frame_tx);

    let mut shutdown_rx = state.shutdown_rx.clone();
    let mut watched_keys: std::collections::HashSet<SessionKey> = watched.keys().copied().collect();

    let send_task = {
        let root_str = root_session_key_str.clone();
        let stream_hub = Arc::clone(&stream_hub);
        let dispatch_tx = dispatch_tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = frame_rx.recv() => {
                        let Some(frame) = msg else { break };

                        // If this frame is a SubagentSpawned event,
                        // grow the watch set before forwarding.
                        if let TopologyFrame::Event { session_key: parent_str, event: WebEvent::SubagentSpawned { child_session, .. } } = &frame {
                            if let Ok(child_key) = SessionKey::try_from_raw(child_session) {
                                if watched_keys.insert(child_key) {
                                    let parent_key = SessionKey::try_from_raw(parent_str).ok();
                                    spawn_dynamic_forwarder(
                                        &stream_hub,
                                        &dispatch_tx,
                                        child_key,
                                        parent_key,
                                        &root_str,
                                    );
                                    let subscribed = TopologyFrame::SessionSubscribed {
                                        session_key: child_session.clone(),
                                        parent:      Some(parent_str.clone()),
                                    };
                                    if let Err(e) = send_frame(&mut ws_tx, &subscribed).await {
                                        debug!(root_session_key = %root_str, error = %e, "topology WS subscribed-frame send failed");
                                        break;
                                    }
                                }
                            }
                        }

                        if let Err(e) = send_frame(&mut ws_tx, &frame).await {
                            debug!(root_session_key = %root_str, error = %e, "topology WS frame send failed");
                            break;
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        debug!(root_session_key = %root_str, "topology WS shutdown signal received");
                        break;
                    }
                }
            }
        })
    };

    // Drain inbound WS frames so transport-level Pings / Closes are
    // serviced. We accept no inbound application frames on this endpoint.
    let recv_task = {
        let root_str = root_session_key_str.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_rx.next().await {
                match msg {
                    Message::Close(_) => {
                        debug!(root_session_key = %root_str, "client closed topology WS");
                        break;
                    }
                    // Ping/Pong are auto-handled by axum/tungstenite.
                    Message::Text(_) | Message::Binary(_) => {
                        debug!(root_session_key = %root_str, "topology WS dropped inbound app frame (endpoint is one-way)");
                    }
                    _ => continue,
                }
            }
        })
    };

    drop(dispatch_tx);

    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
    for (_key, handle) in watched.drain() {
        handle.abort();
    }
    info!(root_session_key = %root_session_key_str, "topology WS closed");
}

/// Walk `process_table` to find every transitive descendant of `root`.
/// Returns `(child, parent)` pairs in BFS order so the client can build
/// the tree top-down. The root itself is not included — callers add it
/// to the watch set explicitly.
fn collect_descendants(
    process_table: &rara_kernel::session::SessionTable,
    root: SessionKey,
) -> Vec<(SessionKey, SessionKey)> {
    let mut out = Vec::new();
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for stats in process_table.children_of(parent) {
            out.push((stats.session_key, parent));
            frontier.push(stats.session_key);
        }
    }
    out
}

/// Spawn a forwarder for `session` and remember the join handle in
/// `watched`. Idempotent: a duplicate key is a no-op.
fn spawn_forwarder(
    watched: &mut HashMap<SessionKey, tokio::task::JoinHandle<()>>,
    stream_hub: &Arc<rara_kernel::io::StreamHub>,
    frame_tx: &mpsc::UnboundedSender<TopologyFrame>,
    session: SessionKey,
    parent: Option<SessionKey>,
    root_session_key_str: &str,
) {
    if watched.contains_key(&session) {
        return;
    }
    // Announce subscription before the forwarder starts so the client
    // observes the same ordering for snapshot + growth.
    let announce = TopologyFrame::SessionSubscribed {
        session_key: session.to_string(),
        parent:      parent.map(|p| p.to_string()),
    };
    if frame_tx.send(announce).is_err() {
        return;
    }
    let handle = spawn_bus_forwarder(stream_hub, frame_tx.clone(), session, root_session_key_str);
    watched.insert(session, handle);
}

/// Variant of [`spawn_forwarder`] used by the dispatch task at runtime,
/// when growing the watch set in response to a `SubagentSpawned`. Skips
/// the `watched` map (the caller maintains its own `HashSet` to dedupe)
/// and skips the `SessionSubscribed` announce because the dispatch task
/// emits it inline so it lands in-order with the spawn event.
fn spawn_dynamic_forwarder(
    stream_hub: &Arc<rara_kernel::io::StreamHub>,
    frame_tx: &mpsc::UnboundedSender<TopologyFrame>,
    session: SessionKey,
    _parent: Option<SessionKey>,
    root_session_key_str: &str,
) -> tokio::task::JoinHandle<()> {
    spawn_bus_forwarder(stream_hub, frame_tx.clone(), session, root_session_key_str)
}

/// Spawn a task that forwards every event from `session`'s session bus
/// onto the connection's frame mpsc, mapped through
/// `stream_event_to_web_event` and tagged with the originating
/// `session_key`. Lagged buses are logged at warn and the loop continues
/// (consistent with the per-session WS forwarder in `web_session.rs`).
fn spawn_bus_forwarder(
    stream_hub: &Arc<rara_kernel::io::StreamHub>,
    frame_tx: mpsc::UnboundedSender<TopologyFrame>,
    session: SessionKey,
    root_session_key_str: &str,
) -> tokio::task::JoinHandle<()> {
    let mut rx = stream_hub.subscribe_session_events(&session);
    let session_str = session.to_string();
    let root_str = root_session_key_str.to_owned();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let Some(web_event) = stream_event_to_web_event(event) else {
                        continue;
                    };
                    let frame = TopologyFrame::Event {
                        session_key: session_str.clone(),
                        event:       web_event,
                    };
                    if frame_tx.send(frame).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(
                        root_session_key = %root_str,
                        session_key = %session_str,
                        skipped = n,
                        "topology WS bus lagged"
                    );
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

async fn send_frame<S>(ws_tx: &mut S, frame: &TopologyFrame) -> Result<(), axum::Error>
where
    S: SinkExt<Message, Error = axum::Error> + Unpin,
{
    let json = match serde_json::to_string(frame) {
        Ok(j) => j,
        Err(e) => {
            error!(error = %e, "serialize topology frame");
            return Ok(());
        }
    };
    ws_tx.send(Message::Text(json.into())).await
}

async fn send_error<S>(ws_tx: &mut S, message: &str) -> Result<(), axum::Error>
where
    S: SinkExt<Message, Error = axum::Error> + Unpin,
{
    let frame = TopologyFrame::Error {
        message: message.to_owned(),
    };
    send_frame(ws_tx, &frame).await
}
