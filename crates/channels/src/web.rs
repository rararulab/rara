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

//! Web channel adapter — WebSocket + SSE implementation of [`ChannelAdapter`].
//!
//! # Design
//!
//! Unlike the Telegram adapter (which starts its own polling loop), the
//! `WebAdapter` exposes an [`axum::Router`] that the host application mounts.
//! The adapter tracks active sessions via [`DashMap`] with
//! [`tokio::sync::broadcast`] channels for fan-out to multiple WebSocket and
//! SSE connections.
//!
//! Inbound messages are handed to the kernel via [`KernelHandle`] in a
//! fire-and-forget fashion. Outbound delivery is handled separately via
//! [`ChannelAdapter`].
//!
//! # Endpoints
//!
//! | Method | Path        | Description                          |
//! |--------|-------------|--------------------------------------|
//! | GET    | `/ws`       | WebSocket upgrade (bidirectional)    |
//! | GET    | `/events`   | SSE stream (server-push)             |
//! | POST   | `/messages` | Send message (fire-and-forget)       |

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use axum::{
    Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use dashmap::DashMap;
use futures::{SinkExt, StreamExt, stream::Stream};
use rara_kernel::{
    channel::{
        adapter::ChannelAdapter,
        types::{AgentPhase, ChannelType, MessageContent},
    },
    error::KernelError,
    handle::KernelHandle,
    identity::UserId,
    io::{
        EgressError, Endpoint, EndpointAddress, EndpointRegistry, InteractionType,
        PlatformOutbound, RawPlatformMessage, ReplyContext, StreamEvent,
    },
};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast, watch};
use tracing::{debug, error, info, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default broadcast channel capacity per session.
const BROADCAST_CAPACITY: usize = 256;

// ---------------------------------------------------------------------------
// SSE event types (serialized as JSON in SSE data field)
// ---------------------------------------------------------------------------

/// An event sent over SSE / WebSocket to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebEvent {
    /// Agent response message (final reply).
    Message { content: String },
    /// Typing indicator.
    Typing,
    /// Agent phase change.
    Phase { phase: String },
    /// Error notification.
    Error { message: String },
    /// Incremental text output from LLM.
    TextDelta { text: String },
    /// Incremental reasoning/thinking text.
    ReasoningDelta { text: String },
    /// A tool call has started.
    ToolCallStart {
        name:      String,
        id:        String,
        arguments: serde_json::Value,
    },
    /// A tool call has finished.
    ToolCallEnd {
        id:             String,
        result_preview: String,
        success:        bool,
        error:          Option<String>,
    },
    /// Progress stage update.
    Progress { stage: String },
    /// Turn metrics summary (sent before Done).
    TurnMetrics {
        duration_ms: u64,
        iterations:  usize,
        tool_calls:  usize,
        model:       String,
    },
    /// A plan has been created with a goal and steps.
    PlanCreated {
        goal:  String,
        steps: Vec<String>,
    },
    /// A plan step has started executing.
    PlanStepStart {
        index: usize,
        task:  String,
        mode:  String,
    },
    /// A plan step has finished.
    PlanStepEnd {
        index:   usize,
        outcome: String,
        summary: String,
    },
    /// The plan has been revised.
    PlanReplan {
        reason:    String,
        new_steps: Vec<String>,
    },
    /// The plan has completed.
    PlanCompleted { summary: String },
    /// Stream completed (no more deltas).
    Done,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Query parameters for WebSocket and SSE endpoints.
#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    pub session_key: String,
    /// JWT access token (preferred). When provided, the user identity is
    /// extracted from the token claims instead of `user_id`.
    #[serde(default)]
    pub token:       Option<String>,
    #[serde(default = "default_user_id")]
    pub user_id:     String,
}

fn default_user_id() -> String { "anonymous".to_owned() }

fn stream_event_to_web_event(event: StreamEvent) -> Option<WebEvent> {
    match event {
        StreamEvent::TextDelta { text } => Some(WebEvent::TextDelta { text }),
        StreamEvent::ReasoningDelta { .. } | StreamEvent::TextClear => None,
        StreamEvent::ToolCallStart {
            name,
            id,
            arguments,
        } => Some(WebEvent::ToolCallStart {
            name,
            id,
            arguments,
        }),
        StreamEvent::ToolCallEnd {
            id,
            result_preview,
            success,
            error,
        } => Some(WebEvent::ToolCallEnd {
            id,
            result_preview,
            success,
            error,
        }),
        StreamEvent::Progress { stage } => Some(WebEvent::Progress { stage }),
        StreamEvent::TurnMetrics {
            duration_ms,
            iterations,
            tool_calls,
            model,
        } => Some(WebEvent::TurnMetrics {
            duration_ms,
            iterations,
            tool_calls,
            model,
        }),
        StreamEvent::PlanCreated { goal, steps } => {
            Some(WebEvent::PlanCreated { goal, steps })
        }
        StreamEvent::PlanStepStart { index, task, mode } => {
            Some(WebEvent::PlanStepStart { index, task, mode })
        }
        StreamEvent::PlanStepEnd {
            index,
            outcome,
            summary,
        } => Some(WebEvent::PlanStepEnd {
            index,
            outcome,
            summary,
        }),
        StreamEvent::PlanReplan { reason, new_steps } => {
            Some(WebEvent::PlanReplan { reason, new_steps })
        }
        StreamEvent::PlanCompleted { summary } => Some(WebEvent::PlanCompleted { summary }),
    }
}

/// JSON body for POST /messages.
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub session_key: String,
    pub user_id:     String,
    pub content:     String,
}

/// JSON response for POST /messages.
#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub accepted: bool,
}

// ---------------------------------------------------------------------------
// WebAdapter
// ---------------------------------------------------------------------------

/// Web channel adapter supporting WebSocket and SSE connections.
///
/// # Usage
///
/// ```rust,ignore
/// let adapter = WebAdapter::new();
/// let router = adapter.router();
/// // Mount into your axum app:
/// // app.nest("/chat", router)
/// ```
pub struct WebAdapter {
    /// Active sessions: session_key -> broadcast sender for outbound events.
    sessions:          Arc<DashMap<String, broadcast::Sender<String>>>,
    /// KernelHandle for dispatching inbound messages (set during `start`).
    sink:              Arc<RwLock<Option<KernelHandle>>>,
    /// StreamHub for subscribing to real-time token deltas.
    stream_hub:        Arc<RwLock<Option<Arc<rara_kernel::io::StreamHub>>>>,
    /// EndpointRegistry for tracking connected users (set during startup).
    endpoint_registry: Arc<RwLock<Option<Arc<EndpointRegistry>>>>,
    /// Owner token for verifying WebSocket auth tokens.
    owner_token:       Option<String>,
    /// Shutdown signal sender.
    shutdown_tx:       watch::Sender<bool>,
    /// Shutdown signal receiver (cloneable).
    shutdown_rx:       watch::Receiver<bool>,
}

impl WebAdapter {
    /// Create a new `WebAdapter`.
    pub fn new(owner_token: Option<String>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            sessions: Arc::new(DashMap::new()),
            sink: Arc::new(RwLock::new(None)),
            stream_hub: Arc::new(RwLock::new(None)),
            endpoint_registry: Arc::new(RwLock::new(None)),
            owner_token,
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Returns an [`axum::Router`] with WebSocket, SSE, and message endpoints.
    ///
    /// Mount this into your application:
    /// ```rust,ignore
    /// app.nest("/chat", adapter.router())
    /// ```
    pub fn router(&self) -> Router {
        let state = WebAdapterState {
            sessions:          Arc::clone(&self.sessions),
            sink:              Arc::clone(&self.sink),
            stream_hub:        Arc::clone(&self.stream_hub),
            endpoint_registry: Arc::clone(&self.endpoint_registry),
            owner_token:       self.owner_token.clone(),
            shutdown_rx:       self.shutdown_rx.clone(),
        };

        Router::new()
            .route("/ws", get(ws_handler))
            .route("/events", get(sse_handler))
            .route("/messages", post(send_message_handler))
            .route("/signals/{session_id}/interrupt", post(interrupt_handler))
            .with_state(state)
    }

    /// Get or create a broadcast channel for the given session key.
    fn get_or_create_session(
        sessions: &DashMap<String, broadcast::Sender<String>>,
        session_key: &str,
    ) -> broadcast::Sender<String> {
        sessions
            .entry(session_key.to_owned())
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
                tx
            })
            .clone()
    }

    /// Broadcast a serialized event to all subscribers of a session.
    fn broadcast_event(
        sessions: &DashMap<String, broadcast::Sender<String>>,
        session_key: &str,
        event: &WebEvent,
    ) {
        if let Some(tx) = sessions.get(session_key) {
            let json = match serde_json::to_string(event) {
                Ok(j) => j,
                Err(e) => {
                    error!(session_key, error = %e, "failed to serialize web event");
                    return;
                }
            };
            // Ignore send errors — they mean no active receivers.
            let _ = tx.send(json);
        }
    }
}

// ---------------------------------------------------------------------------
// Shared state for axum handlers
// ---------------------------------------------------------------------------

/// Shared state passed to axum route handlers.
#[derive(Clone)]
struct WebAdapterState {
    sessions:          Arc<DashMap<String, broadcast::Sender<String>>>,
    sink:              Arc<RwLock<Option<KernelHandle>>>,
    stream_hub:        Arc<RwLock<Option<Arc<rara_kernel::io::StreamHub>>>>,
    endpoint_registry: Arc<RwLock<Option<Arc<EndpointRegistry>>>>,
    owner_token:       Option<String>,
    shutdown_rx:       watch::Receiver<bool>,
}

// ---------------------------------------------------------------------------
// Helper: build endpoint for a Web connection
// ---------------------------------------------------------------------------

/// Verify that the provided token matches the expected owner token.
fn verify_owner_token(expected: &str, provided: &str) -> bool { expected == provided }

/// Build a Web endpoint and its associated UserId for endpoint registration.
///
/// The `UserId` format matches [`AppIdentityResolver`] (`"web:{user_id}"`).
fn web_endpoint_for(session_key: &str) -> Endpoint {
    Endpoint {
        channel_type: ChannelType::Web,
        address:      EndpointAddress::Web {
            connection_id: session_key.to_owned(),
        },
    }
}

/// Compute the UserId matching what the identity resolver returns.
///
/// For authenticated web users, the `user_id` is the real kernel username
/// (extracted from JWT), so no prefix is needed.
fn web_user_id(user_id: &str) -> UserId { UserId(user_id.to_string()) }

/// Register a web endpoint in the registry (if available).
async fn register_endpoint(
    registry: &RwLock<Option<Arc<EndpointRegistry>>>,
    user_id: &str,
    session_key: &str,
) {
    let guard = registry.read().await;
    if let Some(ref reg) = *guard {
        reg.register(&web_user_id(user_id), web_endpoint_for(session_key));
    }
}

/// Unregister a web endpoint from the registry (if available).
async fn unregister_endpoint(
    registry: &RwLock<Option<Arc<EndpointRegistry>>>,
    user_id: &str,
    session_key: &str,
) {
    let guard = registry.read().await;
    if let Some(ref reg) = *guard {
        reg.unregister(&web_user_id(user_id), &web_endpoint_for(session_key));
    }
}

// ---------------------------------------------------------------------------
// Helper: build a RawPlatformMessage from request data
// ---------------------------------------------------------------------------

fn build_raw_platform_message(
    session_key: &str,
    user_id: &str,
    content: &str,
) -> RawPlatformMessage {
    RawPlatformMessage {
        channel_type:        ChannelType::Web,
        platform_message_id: Some(ulid::Ulid::new().to_string()),
        platform_user_id:    user_id.to_owned(),
        platform_chat_id:    Some(session_key.to_owned()),
        content:             MessageContent::Text(content.to_owned()),
        reply_context:       Some(ReplyContext {
            thread_id:                None,
            reply_to_platform_msg_id: None,
            interaction_type:         InteractionType::Message,
        }),
        metadata:            HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// WebSocket handler
// ---------------------------------------------------------------------------

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<SessionQuery>,
    State(state): State<WebAdapterState>,
) -> Response {
    // If an owner token is provided, verify it.
    if let Some(ref token) = params.token {
        if !token.is_empty() {
            if let Some(ref expected) = state.owner_token {
                if verify_owner_token(expected, token) {
                    info!(session_key = %params.session_key, "WebSocket auth via owner token");
                } else {
                    warn!(session_key = %params.session_key, "invalid owner token, rejecting");
                    return axum::response::Response::builder()
                        .status(axum::http::StatusCode::UNAUTHORIZED)
                        .body(axum::body::Body::from("invalid token"))
                        .unwrap();
                }
            } else {
                warn!("owner token not configured, ignoring token");
            }
        }
    }

    info!(
        session_key = %params.session_key,
        user_id = %params.user_id,
        "WebSocket upgrade request"
    );
    ws.on_upgrade(move |socket| handle_ws(socket, params, state))
}

async fn handle_ws(socket: WebSocket, params: SessionQuery, state: WebAdapterState) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Subscribe to broadcast for this session.
    let tx = WebAdapter::get_or_create_session(&state.sessions, &params.session_key);
    let mut rx = tx.subscribe();

    let session_key = params.session_key.clone();
    let mut shutdown_rx = state.shutdown_rx.clone();

    // Register this connection in the EndpointRegistry.
    register_endpoint(&state.endpoint_registry, &params.user_id, &session_key).await;

    // Task: forward broadcast events to the WebSocket client.
    let send_task = {
        let session_key = session_key.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Ok(text) => {
                                if ws_tx.send(Message::Text(text.into())).await.is_err() {
                                    debug!(session_key, "WebSocket send failed, closing");
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(session_key, skipped = n, "WebSocket receiver lagged");
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                debug!(session_key, "broadcast channel closed");
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        debug!(session_key, "shutdown signal received, closing WebSocket sender");
                        break;
                    }
                }
            }
        })
    };

    // Task: read messages from the WebSocket client, dispatch to sink.
    let recv_task = {
        let sink = Arc::clone(&state.sink);
        let stream_hub = Arc::clone(&state.stream_hub);
        let sessions = Arc::clone(&state.sessions);
        let session_key = session_key.clone();
        let user_id = params.user_id.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_rx.next().await {
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => {
                        debug!(session_key, "WebSocket client closed connection");
                        break;
                    }
                    _ => continue,
                };

                if text.trim().is_empty() {
                    continue;
                }

                let raw = build_raw_platform_message(&session_key, &user_id, &text);

                let guard = sink.read().await;
                if let Some(ref s) = *guard {
                    // Send typing indicator before processing.
                    WebAdapter::broadcast_event(&sessions, &session_key, &WebEvent::Typing);
                    if let Err(e) = s.ingest(raw).await {
                        error!(session_key, error = %e, "sink ingest failed");
                        WebAdapter::broadcast_event(
                            &sessions,
                            &session_key,
                            &WebEvent::Error {
                                message: e.to_string(),
                            },
                        );
                    } else {
                        // Spawn a stream forwarder to bridge StreamHub → WebSocket.
                        spawn_stream_forwarder(
                            Arc::clone(&stream_hub),
                            Arc::clone(&sessions),
                            session_key.clone(),
                        );
                    }
                } else {
                    warn!(session_key, "sink not set, cannot dispatch message");
                    WebAdapter::broadcast_event(
                        &sessions,
                        &session_key,
                        &WebEvent::Error {
                            message: "adapter not started".to_owned(),
                        },
                    );
                }
            }
        })
    };

    // Wait for either task to finish, then abort the other.
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }

    // Unregister this connection from the EndpointRegistry.
    unregister_endpoint(&state.endpoint_registry, &params.user_id, &session_key).await;

    info!(session_key, "WebSocket connection closed");
}

// ---------------------------------------------------------------------------
// StreamHub → WebSocket forwarder
// ---------------------------------------------------------------------------

/// Spawn a background task that subscribes to StreamHub for the given session
/// and forwards `StreamEvent`s as `WebEvent`s through the session broadcast.
///
/// The session runtime opens streams asynchronously, so we poll
/// `subscribe_session()` with a short delay until streams appear.
fn spawn_stream_forwarder(
    stream_hub: Arc<RwLock<Option<Arc<rara_kernel::io::StreamHub>>>>,
    sessions: Arc<DashMap<String, broadcast::Sender<String>>>,
    session_key: String,
) {
    tokio::spawn(async move {
        let hub = {
            let guard = stream_hub.read().await;
            match guard.as_ref() {
                Some(hub) => Arc::clone(hub),
                None => return, // No StreamHub configured
            }
        };

        let session_id = match rara_kernel::session::SessionKey::try_from_raw(&session_key) {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!(session_key = %session_key, "invalid session key for stream forwarder");
                return;
            }
        };

        // Poll until stream appears (session runtime opens it asynchronously).
        let mut attempts = 0;
        let subs = loop {
            let s = hub.subscribe_session(&session_id);
            if !s.is_empty() || attempts > 50 {
                break s;
            }
            attempts += 1;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };

        if subs.is_empty() {
            debug!(session_key, "no streams found after polling");
            return;
        }

        for (_stream_id, mut rx) in subs {
            let sessions = Arc::clone(&sessions);
            let session_key = session_key.clone();
            tokio::spawn(async move {
                while let Ok(event) = rx.recv().await {
                    let Some(web_event) = stream_event_to_web_event(event) else {
                        continue;
                    };
                    WebAdapter::broadcast_event(&sessions, &session_key, &web_event);
                }
                // Stream closed — send Done event.
                WebAdapter::broadcast_event(&sessions, &session_key, &WebEvent::Done);
            });
        }
    });
}

// ---------------------------------------------------------------------------
// SSE handler
// ---------------------------------------------------------------------------

async fn sse_handler(
    Query(params): Query<SessionQuery>,
    State(state): State<WebAdapterState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    info!(session_key = %params.session_key, "SSE connection opened");

    // Register this connection in the EndpointRegistry.
    register_endpoint(
        &state.endpoint_registry,
        &params.user_id,
        &params.session_key,
    )
    .await;

    let tx = WebAdapter::get_or_create_session(&state.sessions, &params.session_key);
    let rx = tx.subscribe();
    let shutdown_rx = state.shutdown_rx.clone();
    let registry_for_cleanup = Arc::clone(&state.endpoint_registry);
    let user_for_cleanup = params.user_id.clone();
    let key_for_cleanup = params.session_key.clone();

    let stream = futures::stream::unfold(
        (rx, shutdown_rx, params.session_key.clone()),
        |(mut rx, mut shutdown_rx, session_key)| async move {
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Ok(data) => {
                                let event: Result<Event, std::convert::Infallible> =
                                    Ok(Event::default().data(data));
                                return Some((event, (rx, shutdown_rx, session_key)));
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(session_key, skipped = n, "SSE receiver lagged");
                                let err_event = serde_json::json!({
                                    "type": "error",
                                    "message": format!("missed {n} events")
                                });
                                let event: Result<Event, std::convert::Infallible> =
                                    Ok(Event::default().data(err_event.to_string()));
                                return Some((event, (rx, shutdown_rx, session_key)));
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                debug!(session_key, "broadcast channel closed, ending SSE stream");
                                return None;
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        debug!(session_key, "shutdown signal, ending SSE stream");
                        return None;
                    }
                }
            }
        },
    );

    // Use a oneshot to signal when the SSE stream ends so we can unregister.
    let (cleanup_tx, cleanup_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        // Wait for stream to end (sender dropped when stream completes).
        let _ = cleanup_rx.await;
        unregister_endpoint(&registry_for_cleanup, &user_for_cleanup, &key_for_cleanup).await;
    });

    // Chain the stream to drop the cleanup sender when done.
    let stream = CleanupStream {
        inner:    Box::pin(stream),
        _cleanup: cleanup_tx,
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Wrapper that holds a oneshot sender — when this stream is dropped, the
/// sender drops too, which signals the cleanup task to unregister.
struct CleanupStream<S> {
    inner:    std::pin::Pin<Box<S>>,
    _cleanup: tokio::sync::oneshot::Sender<()>,
}

impl<S: Stream> Stream for CleanupStream<S> {
    type Item = S::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

// ---------------------------------------------------------------------------
// POST /messages handler
// ---------------------------------------------------------------------------

async fn send_message_handler(
    State(state): State<WebAdapterState>,
    axum::Json(body): axum::Json<SendMessageRequest>,
) -> Response {
    debug!(
        session_key = %body.session_key,
        user_id = %body.user_id,
        "POST /messages"
    );

    // Ensure session broadcast exists.
    WebAdapter::get_or_create_session(&state.sessions, &body.session_key);

    let raw = build_raw_platform_message(&body.session_key, &body.user_id, &body.content);

    let guard = state.sink.read().await;
    match &*guard {
        Some(sink) => {
            // Broadcast typing indicator.
            WebAdapter::broadcast_event(&state.sessions, &body.session_key, &WebEvent::Typing);

            match sink.ingest(raw).await {
                Ok(()) => {
                    spawn_stream_forwarder(
                        Arc::clone(&state.stream_hub),
                        Arc::clone(&state.sessions),
                        body.session_key.clone(),
                    );
                    axum::Json(SendMessageResponse { accepted: true }).into_response()
                }
                Err(e) => {
                    error!(session_key = %body.session_key, error = %e, "ingest failed");
                    let status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
                    (status, e.to_string()).into_response()
                }
            }
        }
        None => {
            let status = axum::http::StatusCode::SERVICE_UNAVAILABLE;
            (status, "adapter not started").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// POST /signals/{session_id}/interrupt handler
// ---------------------------------------------------------------------------

async fn interrupt_handler(
    axum::extract::Path(session_id): axum::extract::Path<String>,
    State(state): State<WebAdapterState>,
) -> Response {
    let session_key = match rara_kernel::session::SessionKey::try_from_raw(&session_id) {
        Ok(k) => k,
        Err(_) => {
            return (axum::http::StatusCode::BAD_REQUEST, "invalid session key").into_response();
        }
    };

    let guard = state.sink.read().await;
    match &*guard {
        Some(handle) => {
            match handle.send_signal(session_key, rara_kernel::session::Signal::Interrupt) {
                Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
                Err(e) => {
                    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
                }
            }
        }
        None => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "adapter not started",
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// ChannelAdapter trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ChannelAdapter for WebAdapter {
    fn channel_type(&self) -> ChannelType { ChannelType::Web }

    async fn send(&self, endpoint: &Endpoint, msg: PlatformOutbound) -> Result<(), EgressError> {
        let broadcast_key = match &endpoint.address {
            EndpointAddress::Web { connection_id } => connection_id.as_str(),
            _ => return Ok(()),
        };

        let event = match msg {
            PlatformOutbound::Reply { content, .. } => WebEvent::Message { content },
            PlatformOutbound::StreamChunk { delta, .. } => WebEvent::TextDelta { text: delta },
            PlatformOutbound::Progress { text } => WebEvent::Progress { stage: text },
        };

        WebAdapter::broadcast_event(&self.sessions, broadcast_key, &event);
        Ok(())
    }

    async fn start(&self, handle: KernelHandle) -> Result<(), KernelError> {
        *self.stream_hub.write().await = Some(handle.stream_hub().clone());
        *self.endpoint_registry.write().await = Some(handle.endpoint_registry().clone());
        let mut guard = self.sink.write().await;
        *guard = Some(handle);
        info!("WebAdapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<(), KernelError> {
        info!("WebAdapter stopping — clearing sessions");
        let _ = self.shutdown_tx.send(true);
        let mut guard = self.sink.write().await;
        *guard = None;
        self.sessions.clear();
        Ok(())
    }

    async fn typing_indicator(&self, session_key: &str) -> Result<(), KernelError> {
        WebAdapter::broadcast_event(&self.sessions, session_key, &WebEvent::Typing);
        Ok(())
    }

    async fn set_phase(&self, session_key: &str, phase: AgentPhase) -> Result<(), KernelError> {
        WebAdapter::broadcast_event(
            &self.sessions,
            session_key,
            &WebEvent::Phase {
                phase: phase.to_string(),
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rara_kernel::io::StreamEvent;

    use super::{WebEvent, stream_event_to_web_event};

    #[test]
    fn reasoning_deltas_are_forwarded_to_web_clients() {
        let event = StreamEvent::ReasoningDelta {
            text: "internal".to_owned(),
        };

        assert!(matches!(
            stream_event_to_web_event(event),
            Some(WebEvent::ReasoningDelta { text }) if text == "internal"
        ));
    }

    #[test]
    fn text_deltas_still_reach_web_clients() {
        let event = StreamEvent::TextDelta {
            text: "hello".to_owned(),
        };

        assert!(matches!(
            stream_event_to_web_event(event),
            Some(WebEvent::TextDelta { text }) if text == "hello"
        ));
    }
}
