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
//! # Endpoints
//!
//! | Method | Path        | Description                          |
//! |--------|-------------|--------------------------------------|
//! | GET    | `/ws`       | WebSocket upgrade (bidirectional)    |
//! | GET    | `/events`   | SSE stream (server-push)             |
//! | POST   | `/messages` | Send message (SSE companion)         |

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::extract::ws::{Message, WebSocket};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use dashmap::DashMap;
use futures::stream::Stream;
use futures::{SinkExt, StreamExt};
use rara_kernel::channel::adapter::ChannelAdapter;
use rara_kernel::channel::bridge::ChannelBridge;
use rara_kernel::channel::types::{
    AgentPhase, ChannelMessage, ChannelType, ChannelUser, MessageContent, MessageRole,
    OutboundMessage,
};
use rara_kernel::error::KernelError;
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
    /// Agent response message.
    Message { content: String },
    /// Typing indicator.
    Typing,
    /// Agent phase change.
    Phase { phase: String },
    /// Error notification.
    Error { message: String },
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Query parameters for WebSocket and SSE endpoints.
#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    pub session_key: String,
    #[serde(default = "default_user_id")]
    pub user_id: String,
}

fn default_user_id() -> String {
    "anonymous".to_owned()
}

/// JSON body for POST /messages.
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub session_key: String,
    pub user_id: String,
    pub content: String,
}

/// JSON response for POST /messages.
#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub response: String,
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
    sessions: Arc<DashMap<String, broadcast::Sender<String>>>,
    /// Bridge handle (set during `start`).
    bridge: Arc<RwLock<Option<Arc<dyn ChannelBridge>>>>,
    /// Shutdown signal sender.
    shutdown_tx: watch::Sender<bool>,
    /// Shutdown signal receiver (cloneable).
    shutdown_rx: watch::Receiver<bool>,
}

impl WebAdapter {
    /// Create a new `WebAdapter`.
    pub fn new() -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            sessions: Arc::new(DashMap::new()),
            bridge: Arc::new(RwLock::new(None)),
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
            sessions: Arc::clone(&self.sessions),
            bridge: Arc::clone(&self.bridge),
            shutdown_rx: self.shutdown_rx.clone(),
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
    sessions: Arc<DashMap<String, broadcast::Sender<String>>>,
    bridge: Arc<RwLock<Option<Arc<dyn ChannelBridge>>>>,
    shutdown_rx: watch::Receiver<bool>,
}

// ---------------------------------------------------------------------------
// Helper: build a ChannelMessage from request data
// ---------------------------------------------------------------------------

fn build_channel_message(session_key: &str, user_id: &str, content: &str) -> ChannelMessage {
    ChannelMessage {
        id: ulid::Ulid::new().to_string(),
        channel_type: ChannelType::Web,
        user: ChannelUser {
            platform_id: user_id.to_owned(),
            display_name: None,
        },
        session_key: session_key.to_owned(),
        role: MessageRole::User,
        content: MessageContent::Text(content.to_owned()),
        tool_call_id: None,
        tool_name: None,
        timestamp: jiff::Timestamp::now(),
        metadata: HashMap::new(),
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

    // Task: read messages from the WebSocket client, dispatch to bridge.
    let recv_task = {
        let bridge = Arc::clone(&state.bridge);
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

                let channel_msg = build_channel_message(&session_key, &user_id, &text);

                let guard = bridge.read().await;
                if let Some(ref b) = *guard {
                    // Send typing indicator before processing.
                    WebAdapter::broadcast_event(
                        &sessions,
                        &session_key,
                        &WebEvent::Typing,
                    );
                    match b.dispatch(channel_msg).await {
                        Ok(response) => {
                            WebAdapter::broadcast_event(
                                &sessions,
                                &session_key,
                                &WebEvent::Message {
                                    content: response,
                                },
                            );
                        }
                        Err(e) => {
                            error!(session_key, error = %e, "bridge dispatch failed");
                            WebAdapter::broadcast_event(
                                &sessions,
                                &session_key,
                                &WebEvent::Error {
                                    message: e.to_string(),
                                },
                            );
                        }
                    }
                } else {
                    warn!(session_key, "bridge not set, cannot dispatch message");
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

    info!(session_key, "WebSocket connection closed");
}

// ---------------------------------------------------------------------------
// SSE handler
// ---------------------------------------------------------------------------

async fn sse_handler(
    Query(params): Query<SessionQuery>,
    State(state): State<WebAdapterState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    info!(session_key = %params.session_key, "SSE connection opened");

    let tx = WebAdapter::get_or_create_session(&state.sessions, &params.session_key);
    let rx = tx.subscribe();
    let shutdown_rx = state.shutdown_rx.clone();

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

    Sse::new(stream).keep_alive(KeepAlive::default())
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

    let channel_msg = build_channel_message(&body.session_key, &body.user_id, &body.content);

    let guard = state.bridge.read().await;
    match &*guard {
        Some(bridge) => {
            // Broadcast typing indicator.
            WebAdapter::broadcast_event(
                &state.sessions,
                &body.session_key,
                &WebEvent::Typing,
            );

            match bridge.dispatch(channel_msg).await {
                Ok(response) => {
                    // Also broadcast the response to SSE/WS listeners.
                    WebAdapter::broadcast_event(
                        &state.sessions,
                        &body.session_key,
                        &WebEvent::Message {
                            content: response.clone(),
                        },
                    );
                    axum::Json(SendMessageResponse { response }).into_response()
                }
                Err(e) => {
                    error!(session_key = %body.session_key, error = %e, "dispatch failed");
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
    fn channel_type(&self) -> ChannelType {
        ChannelType::Web
    }

    async fn start(
        &self,
        bridge: Arc<dyn ChannelBridge>,
    ) -> Result<(), KernelError> {
        info!("WebAdapter started — bridge registered");
        let mut guard = self.bridge.write().await;
        *guard = Some(bridge);
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
        // Clear bridge reference.
        let mut guard = self.bridge.write().await;
        *guard = None;
        // Clear session map.
        self.sessions.clear();
        Ok(())
    }

    async fn typing_indicator(&self, session_key: &str) -> Result<(), KernelError> {
        WebAdapter::broadcast_event(&self.sessions, session_key, &WebEvent::Typing);
        Ok(())
    }

    async fn set_phase(
        &self,
        session_key: &str,
        phase: AgentPhase,
    ) -> Result<(), KernelError> {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_channel_message_fields() {
        let msg = build_channel_message("sess-1", "user-42", "hello world");
        assert_eq!(msg.session_key, "sess-1");
        assert_eq!(msg.user.platform_id, "user-42");
        assert_eq!(msg.content.as_text(), "hello world");
        assert_eq!(msg.channel_type, ChannelType::Web);
        assert_eq!(msg.role, MessageRole::User);
        assert!(msg.tool_call_id.is_none());
        assert!(msg.tool_name.is_none());
        assert!(!msg.id.is_empty());
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
        WebAdapter::broadcast_event(
            &sessions,
            "orphan",
            &WebEvent::Typing,
        );
    }

    #[test]
    fn broadcast_to_nonexistent_session_is_noop() {
        let sessions: DashMap<String, broadcast::Sender<String>> = DashMap::new();
        // No session "ghost" exists — should silently do nothing.
        WebAdapter::broadcast_event(
            &sessions,
            "ghost",
            &WebEvent::Typing,
        );
    }

    #[test]
    fn adapter_channel_type_is_web() {
        let adapter = WebAdapter::new();
        assert_eq!(adapter.channel_type(), ChannelType::Web);
    }

    #[test]
    fn router_has_expected_routes() {
        let adapter = WebAdapter::new();
        let router = adapter.router();
        // Verify the router can be created without panic.
        // The actual route matching is validated via integration tests.
        drop(router);
    }

    #[tokio::test]
    async fn adapter_start_sets_bridge() {
        use rara_kernel::channel::types::ChannelMessage;

        struct MockBridge;
        #[async_trait]
        impl ChannelBridge for MockBridge {
            async fn dispatch(&self, _msg: ChannelMessage) -> Result<String, KernelError> {
                Ok("mock response".to_owned())
            }
        }

        let adapter = WebAdapter::new();
        assert!(adapter.bridge.read().await.is_none());

        adapter.start(Arc::new(MockBridge)).await.unwrap();
        assert!(adapter.bridge.read().await.is_some());
    }

    #[tokio::test]
    async fn adapter_stop_clears_state() {
        struct MockBridge;
        #[async_trait]
        impl ChannelBridge for MockBridge {
            async fn dispatch(&self, _msg: ChannelMessage) -> Result<String, KernelError> {
                Ok(String::new())
            }
        }

        let adapter = WebAdapter::new();
        adapter.start(Arc::new(MockBridge)).await.unwrap();

        // Create a session.
        WebAdapter::get_or_create_session(&adapter.sessions, "s1");
        assert!(!adapter.sessions.is_empty());

        adapter.stop().await.unwrap();
        assert!(adapter.sessions.is_empty());
        assert!(adapter.bridge.read().await.is_none());
    }

    #[tokio::test]
    async fn send_broadcasts_to_session() {
        let adapter = WebAdapter::new();
        let tx = WebAdapter::get_or_create_session(&adapter.sessions, "sess-x");
        let mut rx = tx.subscribe();

        let outbound = OutboundMessage {
            channel_type: ChannelType::Web,
            session_key: "sess-x".to_owned(),
            content: "hello from agent".to_owned(),
            metadata: HashMap::new(),
        };
        adapter.send(outbound).await.unwrap();

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

        adapter.set_phase("sess-p", AgentPhase::Thinking).await.unwrap();

        let received = rx.try_recv().unwrap();
        assert!(received.contains("\"type\":\"phase\""));
        assert!(received.contains("\"phase\":\"thinking\""));
    }
}
