// Copyright 2025 Crrow
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
//! Inbound messages are handed to the [`InboundSink`] in a fire-and-forget
//! fashion. Outbound delivery is handled separately via
//! [`send`](ChannelAdapter::send).
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
        types::{AgentPhase, ChannelType, MessageContent, OutboundMessage},
    },
    error::KernelError,
    io::{
        egress::{
            EgressAdapter, EgressError, Endpoint, EndpointAddress, EndpointRegistry,
            PlatformOutbound,
        },
        ingress::{InboundSink, RawPlatformMessage},
        types::{InteractionType, ReplyContext as IoReplyContext},
    },
    process::principal::UserId,
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
    ToolCallStart { name: String, id: String },
    /// A tool call has finished.
    ToolCallEnd { id: String },
    /// Progress stage update.
    Progress { stage: String },
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
    #[serde(default = "default_user_id")]
    pub user_id:     String,
}

fn default_user_id() -> String { "anonymous".to_owned() }

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
    /// InboundSink handle (set during `start`).
    sink:              Arc<RwLock<Option<Arc<dyn InboundSink>>>>,
    /// StreamHub for subscribing to real-time token deltas.
    stream_hub:        Arc<RwLock<Option<Arc<rara_kernel::io::stream::StreamHub>>>>,
    /// EndpointRegistry for tracking connected users (set during startup).
    endpoint_registry: Arc<RwLock<Option<Arc<EndpointRegistry>>>>,
    /// Shutdown signal sender.
    shutdown_tx:       watch::Sender<bool>,
    /// Shutdown signal receiver (cloneable).
    shutdown_rx:       watch::Receiver<bool>,
}

impl WebAdapter {
    /// Create a new `WebAdapter`.
    pub fn new() -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            sessions: Arc::new(DashMap::new()),
            sink: Arc::new(RwLock::new(None)),
            stream_hub: Arc::new(RwLock::new(None)),
            endpoint_registry: Arc::new(RwLock::new(None)),
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Set the StreamHub for real-time token streaming.
    pub async fn set_stream_hub(&self, hub: Arc<rara_kernel::io::stream::StreamHub>) {
        let mut guard = self.stream_hub.write().await;
        *guard = Some(hub);
    }

    /// Set the EndpointRegistry for tracking connected users.
    pub async fn set_endpoint_registry(&self, registry: Arc<EndpointRegistry>) {
        let mut guard = self.endpoint_registry.write().await;
        *guard = Some(registry);
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
            shutdown_rx:       self.shutdown_rx.clone(),
        };

        Router::new()
            .route("/ws", get(ws_handler))
            .route("/events", get(sse_handler))
            .route("/messages", post(send_message_handler))
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
    sink:              Arc<RwLock<Option<Arc<dyn InboundSink>>>>,
    stream_hub:        Arc<RwLock<Option<Arc<rara_kernel::io::stream::StreamHub>>>>,
    endpoint_registry: Arc<RwLock<Option<Arc<EndpointRegistry>>>>,
    shutdown_rx:       watch::Receiver<bool>,
}

// ---------------------------------------------------------------------------
// Helper: build endpoint for a Web connection
// ---------------------------------------------------------------------------

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

/// Compute the UserId as the identity resolver would.
///
/// Mirrors `AppIdentityResolver`: `"web:{platform_user_id}"`.
fn web_user_id(user_id: &str) -> UserId { UserId(format!("web:{user_id}")) }

/// Register a web endpoint in the registry (if available).
async fn register_endpoint(registry: &RwLock<Option<Arc<EndpointRegistry>>>, user_id: &str, session_key: &str) {
    let guard = registry.read().await;
    if let Some(ref reg) = *guard {
        reg.register(&web_user_id(user_id), web_endpoint_for(session_key));
    }
}

/// Unregister a web endpoint from the registry (if available).
async fn unregister_endpoint(registry: &RwLock<Option<Arc<EndpointRegistry>>>, user_id: &str, session_key: &str) {
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
        reply_context:       Some(IoReplyContext {
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
/// The process_loop opens streams asynchronously, so we poll
/// `subscribe_session()` with a short delay until streams appear.
fn spawn_stream_forwarder(
    stream_hub: Arc<RwLock<Option<Arc<rara_kernel::io::stream::StreamHub>>>>,
    sessions: Arc<DashMap<String, broadcast::Sender<String>>>,
    session_key: String,
) {
    use rara_kernel::io::stream::StreamEvent;

    tokio::spawn(async move {
        let hub = {
            let guard = stream_hub.read().await;
            match guard.as_ref() {
                Some(hub) => Arc::clone(hub),
                None => return, // No StreamHub configured
            }
        };

        let session_id = rara_kernel::process::SessionId::new(&session_key);

        // Poll until stream appears (process_loop opens it asynchronously).
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
                    let web_event = match event {
                        StreamEvent::TextDelta(t) => WebEvent::TextDelta { text: t },
                        StreamEvent::ReasoningDelta(t) => WebEvent::ReasoningDelta { text: t },
                        StreamEvent::ToolCallStart { name, id } => {
                            WebEvent::ToolCallStart { name, id }
                        }
                        StreamEvent::ToolCallEnd { id } => WebEvent::ToolCallEnd { id },
                        StreamEvent::Progress { stage } => WebEvent::Progress { stage },
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
    register_endpoint(&state.endpoint_registry, &params.user_id, &params.session_key).await;

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
// ChannelAdapter trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ChannelAdapter for WebAdapter {
    fn channel_type(&self) -> ChannelType { ChannelType::Web }

    async fn start(&self, sink: Arc<dyn InboundSink>) -> Result<(), KernelError> {
        info!("WebAdapter started — sink registered");
        let mut guard = self.sink.write().await;
        *guard = Some(sink);
        Ok(())
    }

    async fn send(&self, message: OutboundMessage) -> Result<(), KernelError> {
        WebAdapter::broadcast_event(
            &self.sessions,
            &message.session_key,
            &WebEvent::Message {
                content: message.content,
            },
        );
        Ok(())
    }

    async fn stop(&self) -> Result<(), KernelError> {
        info!("WebAdapter stopping — clearing sessions");
        // Signal shutdown to all SSE/WS connections.
        let _ = self.shutdown_tx.send(true);
        // Clear sink reference.
        let mut guard = self.sink.write().await;
        *guard = None;
        // Clear session map.
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

// ---------------------------------------------------------------------------
// EgressAdapter trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl EgressAdapter for WebAdapter {
    fn channel_type(&self) -> ChannelType { ChannelType::Web }

    async fn send(&self, endpoint: &Endpoint, msg: PlatformOutbound) -> Result<(), EgressError> {
        // Extract the broadcast key from the endpoint's connection_id (the
        // original session_key the client connected with). This avoids the
        // double-prefix issue with PlatformOutbound::session_key.
        let broadcast_key = match &endpoint.address {
            EndpointAddress::Web { connection_id } => connection_id.as_str(),
            _ => return Ok(()),
        };

        let event = match msg {
            PlatformOutbound::Reply { content, .. } => WebEvent::Message { content },
            PlatformOutbound::StreamChunk { delta, .. } => WebEvent::TextDelta { text: delta },
            PlatformOutbound::Progress { text, .. } => WebEvent::Progress { stage: text },
        };

        WebAdapter::broadcast_event(&self.sessions, broadcast_key, &event);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use rara_kernel::io::types::IngestError;

    use super::*;

    #[test]
    fn build_raw_platform_message_fields() {
        let msg = build_raw_platform_message("sess-1", "user-42", "hello world");
        assert_eq!(msg.platform_chat_id, Some("sess-1".to_owned()));
        assert_eq!(msg.platform_user_id, "user-42");
        assert_eq!(msg.content.as_text(), "hello world");
        assert_eq!(msg.channel_type, ChannelType::Web);
        assert!(msg.platform_message_id.is_some());
    }

    #[test]
    fn web_event_serialization() {
        let event = WebEvent::Message {
            content: "hi".to_owned(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"message\""));
        assert!(json.contains("\"content\":\"hi\""));

        let typing = WebEvent::Typing;
        let json = serde_json::to_string(&typing).unwrap();
        assert!(json.contains("\"type\":\"typing\""));

        let phase = WebEvent::Phase {
            phase: "thinking".to_owned(),
        };
        let json = serde_json::to_string(&phase).unwrap();
        assert!(json.contains("\"type\":\"phase\""));
        assert!(json.contains("\"phase\":\"thinking\""));

        let error = WebEvent::Error {
            message: "oops".to_owned(),
        };
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"type\":\"error\""));
        assert!(json.contains("\"message\":\"oops\""));
    }

    #[test]
    fn web_event_deserialization() {
        let json = r#"{"type":"message","content":"hello"}"#;
        let event: WebEvent = serde_json::from_str(json).unwrap();
        match event {
            WebEvent::Message { content } => assert_eq!(content, "hello"),
            _ => panic!("expected Message variant"),
        }
    }

    #[test]
    fn session_broadcast_fan_out() {
        let sessions: DashMap<String, broadcast::Sender<String>> = DashMap::new();

        // Create session and subscribe two receivers.
        let tx = WebAdapter::get_or_create_session(&sessions, "sess-1");
        let mut rx1 = tx.subscribe();
        let mut rx2 = tx.subscribe();

        // Broadcast an event.
        WebAdapter::broadcast_event(
            &sessions,
            "sess-1",
            &WebEvent::Message {
                content: "broadcast-test".to_owned(),
            },
        );

        // Both receivers should get the message.
        let msg1 = rx1.try_recv().unwrap();
        let msg2 = rx2.try_recv().unwrap();
        assert_eq!(msg1, msg2);
        assert!(msg1.contains("broadcast-test"));
    }

    #[test]
    fn session_broadcast_no_receivers_does_not_panic() {
        let sessions: DashMap<String, broadcast::Sender<String>> = DashMap::new();
        let _tx = WebAdapter::get_or_create_session(&sessions, "orphan");

        // Broadcasting with no active receivers should not panic.
        WebAdapter::broadcast_event(&sessions, "orphan", &WebEvent::Typing);
    }

    #[test]
    fn broadcast_to_nonexistent_session_is_noop() {
        let sessions: DashMap<String, broadcast::Sender<String>> = DashMap::new();
        // No session "ghost" exists — should silently do nothing.
        WebAdapter::broadcast_event(&sessions, "ghost", &WebEvent::Typing);
    }

    #[test]
    fn adapter_channel_type_is_web() {
        let adapter = WebAdapter::new();
        assert_eq!(ChannelAdapter::channel_type(&adapter), ChannelType::Web);
    }

    #[test]
    fn router_has_expected_routes() {
        let adapter = WebAdapter::new();
        let router = adapter.router();
        // Verify the router can be created without panic.
        drop(router);
    }

    #[tokio::test]
    async fn adapter_start_sets_sink() {
        struct MockSink;
        #[async_trait]
        impl InboundSink for MockSink {
            async fn ingest(&self, _msg: RawPlatformMessage) -> Result<(), IngestError> { Ok(()) }
        }

        let adapter = WebAdapter::new();
        assert!(adapter.sink.read().await.is_none());

        adapter.start(Arc::new(MockSink)).await.unwrap();
        assert!(adapter.sink.read().await.is_some());
    }

    #[tokio::test]
    async fn adapter_stop_clears_state() {
        struct MockSink;
        #[async_trait]
        impl InboundSink for MockSink {
            async fn ingest(&self, _msg: RawPlatformMessage) -> Result<(), IngestError> { Ok(()) }
        }

        let adapter = WebAdapter::new();
        adapter.start(Arc::new(MockSink)).await.unwrap();

        // Create a session.
        WebAdapter::get_or_create_session(&adapter.sessions, "s1");
        assert!(!adapter.sessions.is_empty());

        adapter.stop().await.unwrap();
        assert!(adapter.sessions.is_empty());
        assert!(adapter.sink.read().await.is_none());
    }

    #[tokio::test]
    async fn send_broadcasts_to_session() {
        let adapter = WebAdapter::new();
        let tx = WebAdapter::get_or_create_session(&adapter.sessions, "sess-x");
        let mut rx = tx.subscribe();

        let outbound = OutboundMessage {
            channel_type:        ChannelType::Web,
            session_key:         "sess-x".to_owned(),
            content:             "hello from agent".to_owned(),
            metadata:            HashMap::new(),
            photo:               None,
            reply_markup:        None,
            edit_message_id:     None,
            reply_to_message_id: None,
        };
        ChannelAdapter::send(&adapter, outbound).await.unwrap();

        let received = rx.try_recv().unwrap();
        assert!(received.contains("hello from agent"));
    }

    #[tokio::test]
    async fn typing_indicator_broadcasts_typing_event() {
        let adapter = WebAdapter::new();
        let tx = WebAdapter::get_or_create_session(&adapter.sessions, "sess-t");
        let mut rx = tx.subscribe();

        adapter.typing_indicator("sess-t").await.unwrap();

        let received = rx.try_recv().unwrap();
        assert!(received.contains("\"type\":\"typing\""));
    }

    #[tokio::test]
    async fn set_phase_broadcasts_phase_event() {
        let adapter = WebAdapter::new();
        let tx = WebAdapter::get_or_create_session(&adapter.sessions, "sess-p");
        let mut rx = tx.subscribe();

        adapter
            .set_phase("sess-p", AgentPhase::Thinking)
            .await
            .unwrap();

        let received = rx.try_recv().unwrap();
        assert!(received.contains("\"type\":\"phase\""));
        assert!(received.contains("\"phase\":\"thinking\""));
    }

    // -----------------------------------------------------------------------
    // EndpointRegistry integration tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn set_endpoint_registry_injects_registry() {
        let adapter = WebAdapter::new();
        assert!(adapter.endpoint_registry.read().await.is_none());

        let registry = Arc::new(EndpointRegistry::new());
        adapter.set_endpoint_registry(registry).await;
        assert!(adapter.endpoint_registry.read().await.is_some());
    }

    #[tokio::test]
    async fn register_unregister_endpoint_lifecycle() {
        let registry = Arc::new(EndpointRegistry::new());
        let registry_lock: Arc<RwLock<Option<Arc<EndpointRegistry>>>> =
            Arc::new(RwLock::new(Some(registry.clone())));

        let user_id = "user-42";
        let session_key = "my-session";

        // Register
        register_endpoint(&registry_lock, user_id, session_key).await;
        let uid = web_user_id(user_id);
        assert!(registry.is_online(&uid));
        let endpoints = registry.get_endpoints(&uid);
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].channel_type, ChannelType::Web);
        match &endpoints[0].address {
            EndpointAddress::Web { connection_id } => {
                assert_eq!(connection_id, session_key);
            }
            _ => panic!("expected Web endpoint"),
        }

        // Unregister
        unregister_endpoint(&registry_lock, user_id, session_key).await;
        assert!(!registry.is_online(&uid));
        assert_eq!(registry.get_endpoints(&uid).len(), 0);
    }

    #[tokio::test]
    async fn register_endpoint_noop_when_no_registry() {
        let registry_lock: Arc<RwLock<Option<Arc<EndpointRegistry>>>> =
            Arc::new(RwLock::new(None));

        // Should not panic when registry is None.
        register_endpoint(&registry_lock, "user-1", "sess-1").await;
        unregister_endpoint(&registry_lock, "user-1", "sess-1").await;
    }

    #[tokio::test]
    async fn egress_send_routes_via_endpoint_connection_id() {
        let adapter = WebAdapter::new();

        // Create a session broadcast and subscribe.
        let tx = WebAdapter::get_or_create_session(&adapter.sessions, "my-chat");
        let mut rx = tx.subscribe();

        // Build endpoint with connection_id = "my-chat".
        let endpoint = Endpoint {
            channel_type: ChannelType::Web,
            address:      EndpointAddress::Web {
                connection_id: "my-chat".to_owned(),
            },
        };

        let msg = PlatformOutbound::Reply {
            session_key:   "web:web:my-chat".to_owned(), // wrong double-prefix
            content:       "hello via egress".to_owned(),
            attachments:   vec![],
            reply_context: None,
        };

        // EgressAdapter::send should use the endpoint's connection_id,
        // NOT the PlatformOutbound session_key.
        EgressAdapter::send(&adapter, &endpoint, msg).await.unwrap();

        let received = rx.try_recv().unwrap();
        assert!(received.contains("hello via egress"));
    }

    #[tokio::test]
    async fn egress_send_ignores_non_web_endpoint() {
        let adapter = WebAdapter::new();

        let endpoint = Endpoint {
            channel_type: ChannelType::Telegram,
            address:      EndpointAddress::Telegram {
                chat_id:   123,
                thread_id: None,
            },
        };

        let msg = PlatformOutbound::Reply {
            session_key:   "tg:123".to_owned(),
            content:       "should be ignored".to_owned(),
            attachments:   vec![],
            reply_context: None,
        };

        // Should return Ok without panicking.
        EgressAdapter::send(&adapter, &endpoint, msg).await.unwrap();
    }

    #[tokio::test]
    async fn egress_send_stream_chunk() {
        let adapter = WebAdapter::new();
        let tx = WebAdapter::get_or_create_session(&adapter.sessions, "stream-sess");
        let mut rx = tx.subscribe();

        let endpoint = Endpoint {
            channel_type: ChannelType::Web,
            address:      EndpointAddress::Web {
                connection_id: "stream-sess".to_owned(),
            },
        };

        let msg = PlatformOutbound::StreamChunk {
            session_key: "ignored".to_owned(),
            delta:       "token".to_owned(),
            edit_target: None,
        };

        EgressAdapter::send(&adapter, &endpoint, msg).await.unwrap();

        let received = rx.try_recv().unwrap();
        assert!(received.contains("\"type\":\"text_delta\""));
        assert!(received.contains("token"));
    }
}
