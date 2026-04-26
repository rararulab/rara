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
//!
//! Each WebSocket / SSE connection has two event sources merged into a single
//! per-connection `mpsc` channel that feeds the socket:
//!
//! 1. The kernel's **session-level** event bus
//!    ([`StreamHub::subscribe_session_events`]) — a permanent subscription
//!    taken at connect time that survives individual stream turnover. This is
//!    the fix for #1647: when the kernel interrupts turn A (buffer+interrupt)
//!    and re-injects as turn B, turn B opens a brand-new per-stream channel;
//!    the session-level bus lets us keep streaming without re-subscribing on
//!    every inbound message.
//! 2. An adapter-local `DashMap<SessionKey, broadcast::Sender<WebEvent>>` for
//!    events that never flow through the kernel: `Typing`, `Error`, `Phase`,
//!    and outbound replies delivered via [`ChannelAdapter::send`].
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
//! | (none) |             | Audio flows as `AudioBase64` content blocks via WS |

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
        PlatformOutbound, RawPlatformMessage, ReplyContext, StreamEvent, StreamHub,
    },
    security::{ApprovalRequest, ApprovalResponse},
    session::SessionKey,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast, mpsc, watch};
use tracing::{debug, error, info, warn};

use crate::web_reply_buffer::ReplyBuffer;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Per-session broadcast capacity for adapter-local events (Typing, Error,
/// Phase, outbound replies) that do NOT flow through the kernel's
/// [`StreamHub`]. Kernel stream events reach WS/SSE via
/// [`StreamHub::subscribe_session_events`], which uses its own capacity.
const ADAPTER_EVENT_CAPACITY: usize = 256;

// ---------------------------------------------------------------------------
// SSE event types (serialized as JSON in SSE data field)
// ---------------------------------------------------------------------------

/// An event sent over SSE / WebSocket to the client.
#[derive(Debug, Clone, Serialize, Deserialize, strum::IntoStaticStr)]
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
    /// Discard any in-flight assistant text the client has rendered for the
    /// current turn. Emitted by the kernel from two sites in
    /// `crates/kernel/src/agent/mod.rs`:
    ///
    /// - the tool-call branch (around line 1676) — clears intermediate
    ///   narration before the upcoming `ToolCallStart` arrives;
    /// - the anti-laziness nudge branch (around line 1935) — clears the
    ///   abandoned ack text before the next iteration's `TextDelta` stream.
    ///
    /// Without this signal, the next `TextDelta` stream would be appended
    /// on top of the now-stale narration on the client.
    TextClear,
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
    /// LLM's rationale for the current tool call batch.
    TurnRationale { text: String },
    /// Progress stage update.
    Progress { stage: String },
    /// Turn metrics summary (sent before Done).
    TurnMetrics {
        duration_ms: u64,
        iterations:  usize,
        tool_calls:  usize,
        model:       String,
    },
    /// Final per-turn token usage (sent before Done). Clients use this to
    /// populate the cost pill + token display in the chat UI. `cache_read`
    /// / `cache_write` are `0` until the kernel tracks them; `cost` is
    /// reported as `0.0` and recomputed client-side against the session's
    /// model pricing table.
    Usage {
        input:        u32,
        output:       u32,
        cache_read:   u32,
        cache_write:  u32,
        total_tokens: u32,
        cost:         f64,
        model:        String,
    },
    /// A plan has been created with a goal and steps.
    PlanCreated {
        goal:                    String,
        total_steps:             usize,
        compact_summary:         String,
        estimated_duration_secs: Option<u32>,
    },
    /// Incremental plan progress update.
    PlanProgress {
        current_step: usize,
        total_steps:  usize,
        step_status:  rara_kernel::io::PlanStepStatus,
        status_text:  String,
    },
    /// The plan has been revised.
    PlanReplan { reason: String },
    /// The plan has completed.
    PlanCompleted { summary: String },
    /// A background agent has been spawned.
    BackgroundTaskStarted {
        task_id:     String,
        agent_name:  String,
        description: String,
    },
    /// A background agent has finished.
    BackgroundTaskDone {
        task_id: String,
        status:  rara_kernel::io::BackgroundTaskStatus,
    },
    /// Kernel has persisted the turn's execution trace; the row is now
    /// retrievable by `trace_id`. Forwarded so future frontend surfaces
    /// (e.g. trace detail modal) can embed the ID without an extra
    /// round-trip; the current web UI ignores unknown events gracefully.
    TraceReady { trace_id: String },
    /// Binary attachment (image, document, etc.) produced by a tool.
    ///
    /// Bytes are transported as standard (non URL-safe) base64 so the
    /// browser can reconstruct a blob or data URL and render the file
    /// inline alongside the matching tool call.
    Attachment {
        /// LLM-assigned tool call id, when the attachment was emitted from
        /// within a tool invocation.
        tool_call_id: Option<String>,
        /// IANA media type of the attachment bytes.
        mime_type:    String,
        /// Optional original filename.
        filename:     Option<String>,
        /// Base64-encoded payload (standard alphabet, with padding).
        data_base64:  String,
    },
    /// A new approval request has been submitted for this session. The
    /// browser should refresh its pending-approval view (e.g. invalidate
    /// the `kernel-approvals` query) so the user sees the request without
    /// waiting for the next poll.
    ApprovalRequested {
        id:           String,
        tool_name:    String,
        summary:      String,
        risk_level:   String,
        requested_at: String,
        timeout_secs: u64,
    },
    /// A previously-pending approval has been resolved (approved, denied,
    /// or timed out) — possibly by another surface such as Telegram. The
    /// browser should refresh its pending-approval view to drop the
    /// cleared entry.
    ApprovalResolved { id: String, decision: String },
    /// Stream completed (no more deltas).
    Done,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Query parameters for WebSocket and SSE endpoints.
///
/// Identity is **not** carried here. After owner-token auth passes in
/// `ws_handler` / `sse_handler`, the authenticated identity is taken
/// from the adapter's server-trusted `owner_user_id` (config-validated
/// at boot). Any `user_id` the client might append to the query string
/// is silently ignored by serde.
#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    pub session_key: String,
    /// Owner token fallback for browser WebSocket upgrades that cannot
    /// set an `Authorization` header. The header is preferred when both
    /// are present.
    #[serde(default)]
    pub token:       Option<String>,
}

/// Map an egress [`PlatformOutbound`] into the [`WebEvent`] frame the
/// browser consumes. Kept pure so adapter behaviour is unit-testable
/// without spinning up the broadcast channel.
fn platform_outbound_to_web_event(msg: PlatformOutbound) -> WebEvent {
    match msg {
        PlatformOutbound::Reply { content, .. } => WebEvent::Message { content },
        PlatformOutbound::StreamChunk { delta, .. } => WebEvent::TextDelta { text: delta },
        PlatformOutbound::Progress { text } => WebEvent::Progress { stage: text },
        PlatformOutbound::Error { message, .. } => WebEvent::Error { message },
    }
}

fn stream_event_to_web_event(event: StreamEvent) -> Option<WebEvent> {
    match event {
        StreamEvent::TextDelta { text } => Some(WebEvent::TextDelta { text }),
        StreamEvent::ReasoningDelta { .. } => None,
        StreamEvent::TextClear => Some(WebEvent::TextClear),
        StreamEvent::TurnRationale { text } => Some(WebEvent::TurnRationale { text }),
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
            rara_message_id: _,
            context_window_tokens: _,
        } => Some(WebEvent::TurnMetrics {
            duration_ms,
            iterations,
            tool_calls,
            model,
        }),
        StreamEvent::PlanCreated {
            goal,
            total_steps,
            compact_summary,
            estimated_duration_secs,
        } => Some(WebEvent::PlanCreated {
            goal,
            total_steps,
            compact_summary,
            estimated_duration_secs,
        }),
        StreamEvent::PlanProgress {
            current_step,
            total_steps,
            step_status,
            status_text,
        } => Some(WebEvent::PlanProgress {
            current_step,
            total_steps,
            step_status,
            status_text,
        }),
        StreamEvent::PlanReplan { reason } => Some(WebEvent::PlanReplan { reason }),
        StreamEvent::PlanCompleted { summary } => Some(WebEvent::PlanCompleted { summary }),
        StreamEvent::UsageUpdate { .. } => None,
        StreamEvent::TurnStarted { .. } => None,
        StreamEvent::TurnUsage {
            input_tokens,
            output_tokens,
            total_tokens,
            model,
        } => Some(WebEvent::Usage {
            input: input_tokens,
            output: output_tokens,
            cache_read: 0,
            cache_write: 0,
            total_tokens,
            cost: 0.0,
            model,
        }),
        StreamEvent::BackgroundTaskStarted {
            task_id,
            agent_name,
            description,
        } => Some(WebEvent::BackgroundTaskStarted {
            task_id,
            agent_name,
            description,
        }),
        StreamEvent::BackgroundTaskDone { task_id, status } => {
            Some(WebEvent::BackgroundTaskDone { task_id, status })
        }
        StreamEvent::ToolCallLimit { .. } => None, // handled by dedicated channel listener
        StreamEvent::ToolCallLimitResolved { .. } => None, // informational only
        StreamEvent::LoopBreakerTriggered { .. } => None, // informational only
        StreamEvent::ToolOutput { .. } => None,    // live preview, not persisted
        StreamEvent::TraceReady { trace_id } => Some(WebEvent::TraceReady { trace_id }),
        StreamEvent::Attachment {
            tool_call_id,
            mime_type,
            filename,
            data,
        } => {
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            Some(WebEvent::Attachment {
                tool_call_id,
                mime_type,
                filename,
                data_base64: STANDARD.encode(&data),
            })
        }
        // Terminal marker from StreamHub::close — surface as per-turn Done.
        // The session-level bus itself stays open across turns.
        StreamEvent::StreamClosed { .. } => Some(WebEvent::Done),
    }
}

/// Parsed inbound WebSocket text frame.
#[derive(Debug, Deserialize)]
struct InboundPayload {
    content: MessageContent,
}

fn parse_inbound_text_frame(text: &str) -> InboundPayload {
    serde_json::from_str(text).unwrap_or_else(|err| {
        if text.starts_with('{') {
            tracing::debug!(error = %err, "WebSocket frame looks like JSON but failed to parse; treating as plain text");
        }
        InboundPayload {
            content: MessageContent::Text(text.to_owned()),
        }
    })
}

/// JSON body for POST /messages.
///
/// The identity is **not** carried in the body — the `POST /messages`
/// handler derives the user id from the adapter's server-trusted
/// `owner_user_id` after owner-token auth. Extra fields (notably a
/// legacy `user_id`) are ignored by serde so older clients continue
/// to deserialize without error.
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub session_key: String,
    pub content:     MessageContent,
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
/// let adapter = WebAdapter::new(owner_token, owner_user_id);
/// let router = adapter.router();
/// // Mount into your axum app:
/// // app.nest("/chat", router)
/// ```
pub struct WebAdapter {
    /// Per-session broadcast sender for adapter-local events (Typing, Error,
    /// Phase, outbound replies from `ChannelAdapter::send`). Kernel stream
    /// events bypass this map — they flow directly from
    /// [`StreamHub::subscribe_session_events`] into each WS/SSE task.
    adapter_events:    Arc<DashMap<SessionKey, broadcast::Sender<WebEvent>>>,
    /// KernelHandle for dispatching inbound messages (set during `start`).
    sink:              Arc<RwLock<Option<KernelHandle>>>,
    /// StreamHub for subscribing to real-time token deltas.
    stream_hub:        Arc<RwLock<Option<Arc<StreamHub>>>>,
    /// EndpointRegistry for tracking connected users (set during startup).
    endpoint_registry: Arc<RwLock<Option<Arc<EndpointRegistry>>>>,
    /// Owner token for verifying WebSocket auth tokens.
    ///
    /// Always present: the boot layer (`rara_app::validate_owner_auth`)
    /// guarantees a non-empty token before constructing the adapter, so
    /// the WS handler always enforces auth.
    owner_token:       String,
    /// Authenticated owner's kernel `user_id`.
    ///
    /// After the owner-token check, this is the identity attached to
    /// every inbound web message. The client's query/body `user_id`
    /// field is ignored — auth establishes "you are the owner", so the
    /// server, not the client, names the identity. Validated at boot by
    /// `rara_app::validate_owner_auth` to match a configured user.
    owner_user_id:     String,
    /// Shutdown signal sender.
    shutdown_tx:       watch::Sender<bool>,
    /// Shutdown signal receiver (cloneable).
    shutdown_rx:       watch::Receiver<bool>,
    /// Optional STT service for transcribing voice messages to text.
    stt_service:       Option<rara_stt::SttService>,
    /// Per-session ring buffer for "important" `WebEvent`s. Adapter
    /// publishes matching [`ReplyBuffer::should_buffer`] are appended so
    /// that a later WS / SSE reconnect can drain them and recover
    /// task-completion replies that fired while no listener was attached
    /// (issue #1804). The buffer is always wired in production — see the
    /// `web_reply_buffer` module for why this is a mechanism, not a knob.
    reply_buffer:      Arc<ReplyBuffer>,
}

impl WebAdapter {
    /// Create a new `WebAdapter`.
    ///
    /// Both `owner_token` and `owner_user_id` are required — invalid
    /// "no auth" / "anonymous caller" states are unrepresentable.
    /// Boot-time validation (`validate_owner_auth`) guarantees a
    /// non-empty token and that `owner_user_id` matches a configured
    /// user before reaching this constructor.
    pub fn new(owner_token: String, owner_user_id: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            adapter_events: Arc::new(DashMap::new()),
            sink: Arc::new(RwLock::new(None)),
            stream_hub: Arc::new(RwLock::new(None)),
            endpoint_registry: Arc::new(RwLock::new(None)),
            owner_token,
            owner_user_id,
            shutdown_tx,
            shutdown_rx,
            stt_service: None,
            reply_buffer: ReplyBuffer::new(),
        }
    }

    /// Attach an STT service for voice message transcription.
    #[must_use]
    pub fn with_stt_service(mut self, stt: Option<rara_stt::SttService>) -> Self {
        self.stt_service = stt;
        self
    }

    /// Override the default per-session [`ReplyBuffer`]. Production code
    /// uses the buffer constructed by [`WebAdapter::new`] and only needs
    /// this hook in tests that want to inspect the buffer directly.
    #[must_use]
    pub fn with_reply_buffer(mut self, buffer: Arc<ReplyBuffer>) -> Self {
        self.reply_buffer = buffer;
        self
    }

    /// Returns an [`axum::Router`] with WebSocket, SSE, and message endpoints.
    ///
    /// Mount this into your application:
    /// ```rust,ignore
    /// app.nest("/chat", adapter.router())
    /// ```
    pub fn router(&self) -> Router {
        let state = WebAdapterState {
            adapter_events:    Arc::clone(&self.adapter_events),
            sink:              Arc::clone(&self.sink),
            stream_hub:        Arc::clone(&self.stream_hub),
            endpoint_registry: Arc::clone(&self.endpoint_registry),
            owner_token:       self.owner_token.clone(),
            owner_user_id:     self.owner_user_id.clone(),
            shutdown_rx:       self.shutdown_rx.clone(),
            stt_service:       self.stt_service.clone(),
            reply_buffer:      self.reply_buffer.clone(),
        };

        let events_state = crate::web_session_events::SessionEventsState {
            owner_token: self.owner_token.clone(),
            handle:      Arc::clone(&self.sink),
        };
        let events_router = Router::new()
            .route(
                "/events/{session_key}",
                get(crate::web_session_events::events_ws_handler),
            )
            .with_state(events_state);

        Router::new()
            .route("/ws", get(ws_handler))
            .route("/events", get(sse_handler))
            .route("/messages", post(send_message_handler))
            .route("/signals/{session_id}/interrupt", post(interrupt_handler))
            .with_state(state)
            .merge(events_router)
    }

    /// Test-only entry point that mirrors the inbound code path exercised by
    /// the WebSocket and `POST /messages` handlers, without requiring an HTTP
    /// round-trip. The adapter must have been `start`ed first so `sink` is
    /// populated.
    ///
    /// Runs audio transcription (if any), constructs the `RawPlatformMessage`,
    /// resolves identity + session, and submits the resulting message into
    /// the kernel's event queue — mirroring the WebSocket / `POST /messages`
    /// code path without requiring an HTTP round-trip.
    ///
    /// On first contact from a new `session_key` the kernel auto-creates a
    /// session; callers can discover the resulting `SessionKey` by polling
    /// `KernelHandle::list_processes` after this returns.
    #[doc(hidden)]
    pub async fn handle_inbound_for_test(
        &self,
        session_key: &str,
        user_id: &str,
        content: MessageContent,
    ) -> Result<(), String> {
        let content = transcribe_audio_blocks(content, &self.stt_service).await;
        let raw = build_raw_platform_message(session_key, user_id, content);

        let handle = {
            let guard = self.sink.read().await;
            guard
                .as_ref()
                .cloned()
                .ok_or_else(|| "adapter not started".to_owned())?
        };

        let msg = handle.resolve(raw).await.map_err(|e| e.to_string())?;
        handle.submit_message(msg).map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Test-only: subscribe to the per-session adapter event bus,
    /// creating it lazily if needed. Mirrors what the WS / SSE
    /// handlers do at connect time.
    #[doc(hidden)]
    pub fn subscribe_for_test(&self, session_key: &SessionKey) -> broadcast::Receiver<WebEvent> {
        Self::get_or_create_adapter_bus(&self.adapter_events, *session_key).subscribe()
    }

    /// Get or create the per-session adapter-event broadcast sender.
    ///
    /// Kept deliberately minimal — the heavy fan-out path (kernel stream
    /// events) runs directly off [`StreamHub::subscribe_session_events`], so
    /// this bus only carries adapter-local events (Typing, Error, Phase,
    /// outbound replies).
    fn get_or_create_adapter_bus(
        buses: &DashMap<SessionKey, broadcast::Sender<WebEvent>>,
        session_key: SessionKey,
    ) -> broadcast::Sender<WebEvent> {
        buses
            .entry(session_key)
            .or_insert_with(|| broadcast::channel(ADAPTER_EVENT_CAPACITY).0)
            .clone()
    }

    /// Publish an adapter-local event to all WS/SSE tasks subscribed to
    /// `session_key`, and append it to the per-session [`ReplyBuffer`]
    /// when [`ReplyBuffer::should_buffer`] returns `true`.
    ///
    /// The broadcast send always runs first so a connected receiver sees
    /// the event with no extra latency; the buffer append happens after,
    /// gated by `should_buffer` to avoid hoarding streaming token deltas.
    /// Buffering is unconditional on the publish side: it does not check
    /// `receiver_count`. Trade-off: a tab that read an event live will
    /// see it again if it disconnects + reconnects within the TTL window.
    /// See `web_reply_buffer.rs` module docs.
    fn publish_adapter_event(
        buses: &DashMap<SessionKey, broadcast::Sender<WebEvent>>,
        reply_buffer: &Arc<ReplyBuffer>,
        session_key: &SessionKey,
        event: WebEvent,
    ) {
        let event_kind: &'static str = (&event).into();
        // Only clone the event when it actually needs buffering — keeps
        // streaming hot paths (TextDelta / ReasoningDelta / …) at zero
        // extra allocations.
        let needs_buffer = ReplyBuffer::should_buffer(&event);
        if let Some(tx) = buses.get(session_key) {
            let receiver_count = tx.receiver_count();
            tracing::debug!(
                session_key = %session_key,
                receiver_count,
                event_kind,
                "web publish_adapter_event"
            );
            let send_result = if needs_buffer {
                let for_buf = event.clone();
                let r = tx.send(event);
                reply_buffer.append(session_key, for_buf);
                r
            } else {
                tx.send(event)
            };
            if send_result.is_err() {
                tracing::warn!(
                    session_key = %session_key,
                    event_kind,
                    "web publish: no active receivers"
                );
            }
        } else {
            tracing::debug!(
                session_key = %session_key,
                event_kind,
                "web publish_adapter_event: no bus yet"
            );
            if needs_buffer {
                reply_buffer.append(session_key, event);
            }
        }
    }
}

/// Listen for approval lifecycle events and fan them out to the originating
/// session's adapter bus.
///
/// Maps `ApprovalRequest` → [`WebEvent::ApprovalRequested`] keyed by
/// `session_key`. Resolutions carry only `request_id`, so the listener
/// maintains a short-lived `request_id → session_key` map populated from
/// the request stream and drained on resolution. Entries that never fire
/// (lost due to broadcast lag) leak until the adapter stops — bounded by
/// the per-agent pending limit enforced by `ApprovalManager`, so the map
/// never grows unboundedly in practice.
#[tracing::instrument(skip_all, name = "web.approval_listener")]
async fn approval_listener(
    mut request_rx: tokio::sync::broadcast::Receiver<ApprovalRequest>,
    mut resolution_rx: tokio::sync::broadcast::Receiver<ApprovalResponse>,
    adapter_events: Arc<DashMap<SessionKey, broadcast::Sender<WebEvent>>>,
    reply_buffer: Arc<ReplyBuffer>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let session_by_request: Arc<DashMap<uuid::Uuid, SessionKey>> = Arc::new(DashMap::new());

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                info!("web approval listener: shutting down");
                return;
            }
            result = request_rx.recv() => {
                match result {
                    Ok(req) => {
                        session_by_request.insert(req.id, req.session_key.clone());
                        let event = WebEvent::ApprovalRequested {
                            id:           req.id.to_string(),
                            tool_name:    req.tool_name.clone(),
                            summary:      req.summary.clone(),
                            risk_level:   risk_level_str(req.risk_level).to_owned(),
                            requested_at: req.requested_at.to_string(),
                            timeout_secs: req.timeout_secs,
                        };
                        WebAdapter::publish_adapter_event(
                            &adapter_events,
                            &reply_buffer,
                            &req.session_key,
                            event,
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "web approval listener: request stream lagged");
                    }
                }
            }
            result = resolution_rx.recv() => {
                match result {
                    Ok(resp) => {
                        let Some((_, session_key)) = session_by_request.remove(&resp.request_id) else {
                            // Request originated before this listener was
                            // subscribed, or the session already went away.
                            continue;
                        };
                        let decision = decision_str(resp.decision).to_owned();
                        let event = WebEvent::ApprovalResolved {
                            id: resp.request_id.to_string(),
                            decision,
                        };
                        WebAdapter::publish_adapter_event(
                            &adapter_events,
                            &reply_buffer,
                            &session_key,
                            event,
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "web approval listener: resolution stream lagged");
                    }
                }
            }
        }
    }
}

fn risk_level_str(level: rara_kernel::security::RiskLevel) -> &'static str {
    use rara_kernel::security::RiskLevel;
    match level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
        RiskLevel::Critical => "critical",
    }
}

fn decision_str(decision: rara_kernel::security::ApprovalDecision) -> &'static str {
    use rara_kernel::security::ApprovalDecision;
    match decision {
        ApprovalDecision::Approved => "approved",
        ApprovalDecision::Denied => "denied",
        ApprovalDecision::TimedOut => "timed_out",
    }
}

// ---------------------------------------------------------------------------
// Shared state for axum handlers
// ---------------------------------------------------------------------------

/// Shared state passed to axum route handlers.
#[derive(Clone)]
struct WebAdapterState {
    adapter_events:    Arc<DashMap<SessionKey, broadcast::Sender<WebEvent>>>,
    sink:              Arc<RwLock<Option<KernelHandle>>>,
    stream_hub:        Arc<RwLock<Option<Arc<StreamHub>>>>,
    endpoint_registry: Arc<RwLock<Option<Arc<EndpointRegistry>>>>,
    owner_token:       String,
    /// Authenticated owner identity — see [`WebAdapter::owner_user_id`].
    /// Used after auth passes to stamp inbound messages with a
    /// server-trusted user id instead of trusting client input.
    owner_user_id:     String,
    shutdown_rx:       watch::Receiver<bool>,
    stt_service:       Option<rara_stt::SttService>,
    /// Always-on per-session ring buffer; see [`WebAdapter::reply_buffer`].
    reply_buffer:      Arc<ReplyBuffer>,
}

// ---------------------------------------------------------------------------
// Helper: build endpoint for a Web connection
// ---------------------------------------------------------------------------

/// Extract a Bearer token from an `Authorization` header, if present and
/// well-formed.
///
/// Returns `Some(token)` only for strict `Bearer <token>` values with a
/// non-empty token after the prefix; case in the scheme keyword is ignored
/// per [RFC 6750]. Anything else — missing header, malformed scheme,
/// non-UTF-8 bytes — returns `None`, which the caller treats as "no header
/// provided" and falls back to the query-string token.
///
/// [RFC 6750]: https://datatracker.ietf.org/doc/html/rfc6750#section-2.1
fn bearer_token_from_headers(headers: &axum::http::HeaderMap) -> Option<&str> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?;
    (!token.is_empty()).then_some(token)
}

/// Build a Web endpoint and its associated UserId for endpoint registration.
///
/// The `UserId` format matches the app identity resolver (`"web:{user_id}"`).
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
    content: MessageContent,
) -> RawPlatformMessage {
    RawPlatformMessage {
        channel_type: ChannelType::Web,
        platform_message_id: Some(ulid::Ulid::new().to_string()),
        platform_user_id: user_id.to_owned(),
        platform_chat_id: Some(session_key.to_owned()),
        content,
        reply_context: Some(ReplyContext {
            thread_id:                None,
            reply_to_platform_msg_id: None,
            interaction_type:         InteractionType::Message,
        }),
        metadata: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Audio transcription helpers
// ---------------------------------------------------------------------------

use rara_kernel::channel::types::ContentBlock;

/// Transcribe any `AudioBase64` blocks in the message content, replacing them
/// with `Text` blocks containing the transcribed text.
async fn transcribe_audio_blocks(
    content: MessageContent,
    stt: &Option<rara_stt::SttService>,
) -> MessageContent {
    let blocks = match content {
        MessageContent::Text(_) => return content,
        MessageContent::Multimodal(blocks) => blocks,
    };

    if !blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::AudioBase64 { .. }))
    {
        return MessageContent::Multimodal(blocks);
    }

    let mut result: Vec<ContentBlock> = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block {
            ContentBlock::AudioBase64 { data, media_type } => {
                let text = transcribe_single_audio(&data, &media_type, stt).await;
                let text = if text.is_empty() {
                    "[voice message]".to_owned()
                } else {
                    text
                };
                result.push(ContentBlock::Text { text });
            }
            other => result.push(other),
        }
    }

    // Simplify: if only one text block remains, unwrap to plain text.
    if result.len() == 1 {
        if let ContentBlock::Text { text } = &result[0] {
            return MessageContent::Text(text.clone());
        }
    }
    MessageContent::Multimodal(result)
}

/// Transcribe a single base64-encoded audio clip via the STT service.
async fn transcribe_single_audio(
    data_b64: &str,
    media_type: &str,
    stt: &Option<rara_stt::SttService>,
) -> String {
    use base64::Engine;

    let Some(stt) = stt else {
        warn!("voice message received but STT not configured");
        return "[voice message]".to_owned();
    };

    let audio_bytes = match base64::engine::general_purpose::STANDARD.decode(data_b64) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "failed to decode audio base64");
            return "[voice message]".to_owned();
        }
    };

    match stt.transcribe(audio_bytes, media_type).await {
        Ok(text) => {
            info!(len = text.len(), "voice message transcribed");
            text
        }
        Err(e) => {
            warn!(error = %e, "STT transcription failed");
            "[voice message \u{2014} transcription failed]".to_owned()
        }
    }
}

// ---------------------------------------------------------------------------
// WebSocket handler
// ---------------------------------------------------------------------------

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<SessionQuery>,
    headers: axum::http::HeaderMap,
    State(state): State<WebAdapterState>,
) -> Response {
    // Prefer `Authorization: Bearer <token>` (browsers can set this via
    // `Sec-WebSocket-Protocol` shims or native clients), fall back to the
    // legacy `?token=` query parameter for browser WebSocket upgrades.
    //
    // Auth is always enforced: `owner_token` is a required `String`
    // guaranteed non-empty by startup validation, so "no token = no auth"
    // is not representable.
    let header_token = bearer_token_from_headers(&headers);
    let query_token = params.token.as_deref().filter(|t| !t.is_empty());
    let provided = header_token.or(query_token);
    match provided {
        Some(tok) if rara_kernel::auth::verify_owner_token(&state.owner_token, tok) => {
            info!(session_key = %params.session_key, "WebSocket auth via owner token");
        }
        Some(_) => {
            warn!(session_key = %params.session_key, "invalid owner token, rejecting");
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("invalid token"))
                .expect("static unauthorized response");
        }
        None => {
            warn!(
                session_key = %params.session_key,
                "owner token not provided, rejecting"
            );
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("missing token"))
                .expect("static unauthorized response");
        }
    }

    info!(
        session_key = %params.session_key,
        user_id = %state.owner_user_id,
        "WebSocket upgrade request"
    );
    ws.on_upgrade(move |socket| handle_ws(socket, params, state))
}

async fn handle_ws(socket: WebSocket, params: SessionQuery, state: WebAdapterState) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    let session_key_str = params.session_key.clone();
    let session_key = match SessionKey::try_from_raw(&session_key_str) {
        Ok(k) => k,
        Err(e) => {
            warn!(session_key = %session_key_str, error = %e, "invalid session key on WS");
            return;
        }
    };

    // Register this connection in the EndpointRegistry under the
    // server-trusted owner id — we must NOT trust any client-provided
    // identifier here, or an attacker with a valid owner token could
    // impersonate an arbitrary user.
    register_endpoint(
        &state.endpoint_registry,
        &state.owner_user_id,
        &session_key_str,
    )
    .await;

    // Per-WS event sink: both the kernel session bus forwarder and adapter
    // event forwarder push into this single channel; the send task drains it
    // and writes to the socket. One consumer → mpsc (no fan-out needed).
    let (ws_event_tx, mut ws_event_rx) = mpsc::unbounded_channel::<WebEvent>();

    // Subscribe to the adapter-local event bus (Typing / Error / Phase /
    // outbound replies). Created lazily so we can publish from other tasks
    // (e.g. POST /messages) even before the first WS subscriber shows up —
    // get_or_create ensures the sender exists before publishers try to emit.
    let adapter_bus = WebAdapter::get_or_create_adapter_bus(&state.adapter_events, session_key);
    let mut adapter_rx = adapter_bus.subscribe();

    // Drain any "important" events buffered during a no-listener window
    // BEFORE the live forwarder starts pushing into the same mpsc, so the
    // socket sees buffered events first in publish order. The buffer is
    // not removed on drain — see `web_reply_buffer` module docs for why.
    let backlog = state.reply_buffer.snapshot(&session_key);
    if !backlog.is_empty() {
        debug!(
            session_key = %session_key_str,
            count = backlog.len(),
            "draining web reply buffer to new WS"
        );
        for ev in backlog {
            if ws_event_tx.send(ev).is_err() {
                break;
            }
        }
    }

    // Forwarder: adapter bus → per-WS mpsc.
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
                        warn!(session_key = %skey, skipped = n, "adapter bus lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    // Forwarder: kernel session event bus → per-WS mpsc. The session bus
    // outlives individual streams so this subscription survives mid-turn
    // interrupt + reinject, which is exactly the #1647 bug fix.
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
                        warn!(session_key = %skey, skipped = n, "session event bus lagged");
                    }
                    // The session bus is never closed by the hub (intentional
                    // — it outlives streams). Reaching Closed means every
                    // sender was dropped, which can only happen during hub
                    // teardown.
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    // Drop the extra sender held here so that when both forwarders exit, the
    // mpsc receiver sees a clean close and the send task can terminate.
    drop(ws_event_tx);

    let mut shutdown_rx = state.shutdown_rx.clone();

    // Send task: drain ws_event_rx → socket.
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
                                error!(session_key = %session_key_str, error = %e, "serialize web event");
                                continue;
                            }
                        };
                        if ws_tx.send(Message::Text(json.into())).await.is_err() {
                            debug!(session_key = %session_key_str, "WebSocket send failed, closing");
                            break;
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        debug!(session_key = %session_key_str, "shutdown signal received");
                        break;
                    }
                }
            }
        })
    };

    // Recv task: read client frames, dispatch to kernel.
    let recv_task = {
        let sink = Arc::clone(&state.sink);
        let adapter_events = Arc::clone(&state.adapter_events);
        let session_key_str = session_key_str.clone();
        let user_id = state.owner_user_id.clone();
        let stt_service = state.stt_service.clone();
        let reply_buffer = state.reply_buffer.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_rx.next().await {
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => {
                        debug!(session_key = %session_key_str, "client closed WS");
                        break;
                    }
                    _ => continue,
                };

                if text.trim().is_empty() {
                    continue;
                }

                let payload = parse_inbound_text_frame(&text);
                let content = transcribe_audio_blocks(payload.content, &stt_service).await;
                let raw = build_raw_platform_message(&session_key_str, &user_id, content);

                let guard = sink.read().await;
                let Some(ref s) = *guard else {
                    warn!(session_key = %session_key_str, "sink not set");
                    WebAdapter::publish_adapter_event(
                        &adapter_events,
                        &reply_buffer,
                        &session_key,
                        WebEvent::Error {
                            message: "adapter not started".to_owned(),
                        },
                    );
                    continue;
                };

                WebAdapter::publish_adapter_event(
                    &adapter_events,
                    &reply_buffer,
                    &session_key,
                    WebEvent::Typing,
                );

                // When no channel binding exists yet (first message),
                // resolve() returns session_key = None. Patch it with the
                // URL-provided key so the kernel reuses the existing session.
                match s.resolve(raw).await {
                    Ok(mut msg) => {
                        if msg.session_key_opt().is_none() {
                            msg.set_session_key(session_key);
                        }
                        if let Err(e) = s.submit_message(msg) {
                            error!(session_key = %session_key_str, error = %e, "submit_message failed");
                            WebAdapter::publish_adapter_event(
                                &adapter_events,
                                &reply_buffer,
                                &session_key,
                                WebEvent::Error {
                                    message: e.to_string(),
                                },
                            );
                        }
                        // No per-message forwarder spawn needed — the
                        // per-WS stream_forwarder above is already
                        // subscribed to the session-level bus and will see
                        // events from every stream opened on this session.
                    }
                    Err(e) => {
                        error!(session_key = %session_key_str, error = %e, "resolve failed");
                        WebAdapter::publish_adapter_event(
                            &adapter_events,
                            &reply_buffer,
                            &session_key,
                            WebEvent::Error {
                                message: e.to_string(),
                            },
                        );
                    }
                }
            }
        })
    };

    // Wait for recv or send to finish, then tear down the others.
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
    adapter_forwarder.abort();
    stream_forwarder.abort();

    unregister_endpoint(
        &state.endpoint_registry,
        &state.owner_user_id,
        &session_key_str,
    )
    .await;
    info!(session_key = %session_key_str, "WebSocket connection closed");
}

// ---------------------------------------------------------------------------
// SSE handler
// ---------------------------------------------------------------------------

async fn sse_handler(
    Query(params): Query<SessionQuery>,
    State(state): State<WebAdapterState>,
) -> Response {
    info!(session_key = %params.session_key, "SSE connection opened");

    let session_key = match SessionKey::try_from_raw(&params.session_key) {
        Ok(k) => k,
        Err(e) => {
            warn!(session_key = %params.session_key, error = %e, "invalid session key on SSE");
            return (axum::http::StatusCode::BAD_REQUEST, "invalid session key").into_response();
        }
    };

    register_endpoint(
        &state.endpoint_registry,
        &state.owner_user_id,
        &params.session_key,
    )
    .await;

    // Same two-source merge as WS: adapter bus + kernel session bus → mpsc.
    let (ev_tx, ev_rx) = mpsc::unbounded_channel::<WebEvent>();

    let adapter_bus = WebAdapter::get_or_create_adapter_bus(&state.adapter_events, session_key);
    let mut adapter_rx = adapter_bus.subscribe();

    // Drain any buffered "important" events before live tail starts.
    let backlog = state.reply_buffer.snapshot(&session_key);
    if !backlog.is_empty() {
        debug!(
            session_key = %params.session_key,
            count = backlog.len(),
            "draining web reply buffer to new SSE"
        );
        for ev in backlog {
            if ev_tx.send(ev).is_err() {
                break;
            }
        }
    }

    {
        let ev_tx = ev_tx.clone();
        let skey = params.session_key.clone();
        tokio::spawn(async move {
            loop {
                match adapter_rx.recv().await {
                    Ok(ev) => {
                        if ev_tx.send(ev).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(session_key = %skey, skipped = n, "SSE adapter bus lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    {
        let ev_tx = ev_tx.clone();
        let stream_hub = Arc::clone(&state.stream_hub);
        let skey = params.session_key.clone();
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
                        let Some(we) = stream_event_to_web_event(event) else {
                            continue;
                        };
                        if ev_tx.send(we).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(session_key = %skey, skipped = n, "SSE stream bus lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    drop(ev_tx);

    let shutdown_rx = state.shutdown_rx.clone();
    let registry_for_cleanup = Arc::clone(&state.endpoint_registry);
    let user_for_cleanup = state.owner_user_id.clone();
    let key_for_cleanup = params.session_key.clone();

    let stream = futures::stream::unfold(
        (ev_rx, shutdown_rx, params.session_key.clone()),
        |(mut rx, mut shutdown_rx, session_key)| async move {
            tokio::select! {
                msg = rx.recv() => {
                    let event = msg?;
                    let json = match serde_json::to_string(&event) {
                        Ok(j) => j,
                        Err(e) => {
                            error!(session_key, error = %e, "serialize SSE event");
                            return None;
                        }
                    };
                    let out: Result<Event, std::convert::Infallible> =
                        Ok(Event::default().data(json));
                    Some((out, (rx, shutdown_rx, session_key)))
                }
                _ = shutdown_rx.changed() => {
                    debug!(session_key, "shutdown signal, ending SSE stream");
                    None
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

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
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
        user_id = %state.owner_user_id,
        "POST /messages"
    );

    let SendMessageRequest {
        session_key: session_key_str,
        content,
    } = body;
    // Identity comes from the server-trusted owner id, not the client
    // body — auth proves "you're the owner", so the server names the
    // identity.
    let user_id = state.owner_user_id.clone();

    let session_key = match SessionKey::try_from_raw(&session_key_str) {
        Ok(k) => k,
        Err(_) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "invalid session_key".to_owned(),
            )
                .into_response();
        }
    };

    // Ensure the adapter bus exists so the Typing indicator below reaches
    // currently-connected WS/SSE subscribers (if any).
    WebAdapter::get_or_create_adapter_bus(&state.adapter_events, session_key);

    let raw = build_raw_platform_message(&session_key_str, &user_id, content);

    let guard = state.sink.read().await;
    match &*guard {
        Some(sink) => {
            WebAdapter::publish_adapter_event(
                &state.adapter_events,
                &state.reply_buffer,
                &session_key,
                WebEvent::Typing,
            );
            match sink.ingest(raw).await {
                Ok(()) => {
                    // No forwarder to spawn — each WS/SSE is permanently
                    // subscribed to the session's event bus via
                    // StreamHub::subscribe_session_events.
                    axum::Json(SendMessageResponse { accepted: true }).into_response()
                }
                Err(e) => {
                    error!(session_key = %session_key_str, error = %e, "ingest failed");
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
        let connection_id = match &endpoint.address {
            EndpointAddress::Web { connection_id } => connection_id.as_str(),
            _ => return Ok(()),
        };
        let Ok(session_key) = SessionKey::try_from_raw(connection_id) else {
            warn!(
                connection_id,
                "web endpoint has non-UUID connection_id; dropping outbound"
            );
            return Ok(());
        };

        let event = platform_outbound_to_web_event(msg);
        WebAdapter::publish_adapter_event(
            &self.adapter_events,
            &self.reply_buffer,
            &session_key,
            event,
        );
        Ok(())
    }

    async fn start(&self, handle: KernelHandle) -> Result<(), KernelError> {
        *self.stream_hub.write().await = Some(handle.stream_hub().clone());
        *self.endpoint_registry.write().await = Some(handle.endpoint_registry().clone());

        // Subscribe to approval events and fan them out to the originating
        // session's adapter bus so the browser learns about
        // requests/resolutions without polling. Mirrors the Telegram
        // adapter's `approval_listener` (see
        // `crates/channels/src/telegram/adapter.rs`).
        {
            let request_rx = handle.security().approval().subscribe_requests();
            let resolution_rx = handle.security().approval().subscribe_resolutions();
            let events = Arc::clone(&self.adapter_events);
            let shutdown_rx = self.shutdown_rx.clone();
            tokio::spawn(approval_listener(
                request_rx,
                resolution_rx,
                events,
                self.reply_buffer.clone(),
                shutdown_rx,
            ));
        }

        let mut guard = self.sink.write().await;
        *guard = Some(handle);
        info!("WebAdapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<(), KernelError> {
        info!("WebAdapter stopping — clearing adapter_events");
        let _ = self.shutdown_tx.send(true);
        let mut guard = self.sink.write().await;
        *guard = None;
        self.adapter_events.clear();
        Ok(())
    }

    async fn typing_indicator(&self, session_key: &str) -> Result<(), KernelError> {
        let Ok(key) = SessionKey::try_from_raw(session_key) else {
            return Ok(());
        };
        WebAdapter::publish_adapter_event(
            &self.adapter_events,
            &self.reply_buffer,
            &key,
            WebEvent::Typing,
        );
        Ok(())
    }

    async fn set_phase(&self, session_key: &str, phase: AgentPhase) -> Result<(), KernelError> {
        let Ok(key) = SessionKey::try_from_raw(session_key) else {
            return Ok(());
        };
        WebAdapter::publish_adapter_event(
            &self.adapter_events,
            &self.reply_buffer,
            &key,
            WebEvent::Phase {
                phase: phase.to_string(),
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rara_kernel::{
        channel::types::{ContentBlock, MessageContent},
        io::{PlatformOutbound, StreamEvent},
    };

    use super::{
        SendMessageRequest, SessionQuery, WebEvent, parse_inbound_text_frame,
        platform_outbound_to_web_event, stream_event_to_web_event,
    };

    #[test]
    fn reasoning_deltas_are_not_forwarded_to_web_clients() {
        let event = StreamEvent::ReasoningDelta {
            text: "internal".to_owned(),
        };

        assert!(stream_event_to_web_event(event).is_none());
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

    #[test]
    fn parses_legacy_text_frame_as_plain_text_message() {
        let payload = parse_inbound_text_frame("hello world");

        assert!(matches!(payload.content, MessageContent::Text(text) if text == "hello world"));
    }

    #[test]
    fn parses_multimodal_json_frame() {
        let raw = serde_json::json!({
            "content": [
                { "type": "text", "text": "look at this" },
                {
                    "type": "image_base64",
                    "media_type": "image/png",
                    "data": "AAAA"
                }
            ]
        })
        .to_string();

        let payload = parse_inbound_text_frame(&raw);

        assert!(matches!(
            payload.content,
            MessageContent::Multimodal(blocks)
                if matches!(
                    blocks.as_slice(),
                    [
                        ContentBlock::Text { text },
                        ContentBlock::ImageBase64 { media_type, data }
                    ] if text == "look at this"
                        && media_type == "image/png"
                        && data == "AAAA"
                )
        ));
    }

    #[test]
    fn turn_usage_is_mapped_to_web_usage_event() {
        let event = StreamEvent::TurnUsage {
            input_tokens:  1_234,
            output_tokens: 56,
            total_tokens:  1_290,
            model:         "gpt-5".to_owned(),
        };

        let mapped = stream_event_to_web_event(event).expect("usage event");
        match mapped {
            WebEvent::Usage {
                input,
                output,
                cache_read,
                cache_write,
                total_tokens,
                cost,
                model,
            } => {
                assert_eq!(input, 1_234);
                assert_eq!(output, 56);
                assert_eq!(cache_read, 0);
                assert_eq!(cache_write, 0);
                assert_eq!(total_tokens, 1_290);
                assert!((cost - 0.0).abs() < f64::EPSILON);
                assert_eq!(model, "gpt-5");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parses_document_attachment_frame() {
        let raw = serde_json::json!({
            "content": [
                { "type": "text", "text": "summarize this" },
                {
                    "type": "file_base64",
                    "media_type": "application/pdf",
                    "data": "JVBERi0x",
                    "filename": "spec.pdf"
                }
            ]
        })
        .to_string();

        let payload = parse_inbound_text_frame(&raw);

        assert!(matches!(
            payload.content,
            MessageContent::Multimodal(blocks)
                if matches!(
                    blocks.as_slice(),
                    [
                        ContentBlock::Text { text },
                        ContentBlock::FileBase64 { media_type, data, filename }
                    ] if text == "summarize this"
                        && media_type == "application/pdf"
                        && data == "JVBERi0x"
                        && filename.as_deref() == Some("spec.pdf")
                )
        ));
    }

    #[test]
    fn platform_error_maps_to_web_error_frame() {
        let event = platform_outbound_to_web_event(PlatformOutbound::Error {
            code:    "agent_error".to_owned(),
            message: "model rejected reasoning=minimal".to_owned(),
        });

        match &event {
            WebEvent::Error { message } => {
                assert_eq!(message, "model rejected reasoning=minimal");
            }
            other => panic!("expected WebEvent::Error, got {other:?}"),
        }

        // The wire format is what the frontend actually parses — lock it
        // down so a future serde rename can't silently break rara-stream.ts.
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["type"], "error");
        assert_eq!(json["message"], "model rejected reasoning=minimal");
    }

    #[test]
    fn approval_requested_serializes_as_snake_case_tagged_frame() {
        let event = WebEvent::ApprovalRequested {
            id:           "11111111-1111-1111-1111-111111111111".to_owned(),
            tool_name:    "bash".to_owned(),
            summary:      "rm -rf /tmp/x".to_owned(),
            risk_level:   "critical".to_owned(),
            requested_at: "2025-01-01T00:00:00Z".to_owned(),
            timeout_secs: 120,
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["type"], "approval_requested");
        assert_eq!(json["tool_name"], "bash");
        assert_eq!(json["risk_level"], "critical");
        assert_eq!(json["timeout_secs"], 120);
    }

    #[test]
    fn approval_resolved_serializes_as_snake_case_tagged_frame() {
        let event = WebEvent::ApprovalResolved {
            id:       "22222222-2222-2222-2222-222222222222".to_owned(),
            decision: "approved".to_owned(),
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["type"], "approval_resolved");
        assert_eq!(json["decision"], "approved");
    }

    #[test]
    fn turn_rationale_is_forwarded_to_web_clients() {
        let event = StreamEvent::TurnRationale {
            text: "checking logs".to_owned(),
        };

        assert!(matches!(
            stream_event_to_web_event(event),
            Some(WebEvent::TurnRationale { text }) if text == "checking logs"
        ));
    }

    #[test]
    fn deserializes_legacy_post_body_with_plain_string_content() {
        // Legacy clients still send `user_id` in the body; serde must
        // ignore that extra field instead of rejecting the request, and
        // the handler takes identity from `state.owner_user_id`.
        let request: SendMessageRequest = serde_json::from_value(serde_json::json!({
            "session_key": "session-123",
            "user_id": "user-123",
            "content": "hello world"
        }))
        .expect("request");

        assert!(matches!(request.content, MessageContent::Text(text) if text == "hello world"));
    }

    #[test]
    fn session_query_has_no_user_id_field() {
        // A client attempting to impersonate via `?user_id=attacker`
        // must be deserialized as if the field did not exist. We
        // verify this by round-tripping through a JSON object that
        // contains the hostile `user_id` key — serde drops unknown
        // fields by default, and the struct no longer has a `user_id`
        // to accept. Identity is established server-side from
        // `WebAdapterState::owner_user_id` after owner-token auth.
        let params: SessionQuery = serde_json::from_value(serde_json::json!({
            "session_key": "s1",
            "user_id": "attacker",
            "token": "tok"
        }))
        .expect("deserialize");
        assert_eq!(params.session_key, "s1");
        assert_eq!(params.token.as_deref(), Some("tok"));
    }
}
