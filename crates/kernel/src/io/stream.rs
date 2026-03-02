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

//! StreamHub — ephemeral real-time event streaming.
//!
//! Provides incremental token deltas and tool progress events to connected
//! frontends while an agent is executing. This is the ephemeral complement
//! to the durable [`OutboundBus`](super::bus::OutboundBus).
//!
//! Key design points:
//! - Streams are keyed by [`StreamId`] (ULID), not `SessionId` — supports
//!   concurrent runs on the same session.
//! - [`StreamEvent`] has no `Done`/`Error` variants — those go through the
//!   `OutboundBus` for durability.
//! - [`StreamHandle`] is held by the agent executor; dropping it does NOT
//!   auto-close (use explicit `close` on `StreamHub`).

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::process::SessionId;

// ---------------------------------------------------------------------------
// StreamId
// ---------------------------------------------------------------------------

/// Unique identifier for a stream (ULID string).
///
/// Each agent execution run gets its own `StreamId`, allowing multiple
/// concurrent streams on the same session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamId(pub String);

impl StreamId {
    /// Generate a new ULID-based stream ID.
    pub fn new() -> Self { Self(ulid::Ulid::new().to_string()) }
}

impl Default for StreamId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}

// ---------------------------------------------------------------------------
// StreamEvent
// ---------------------------------------------------------------------------

/// Incremental events emitted during agent execution.
///
/// These are ephemeral — not stored durably. Final results and errors
/// are published through the `OutboundBus`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Incremental text output from the LLM.
    TextDelta { text: String },
    /// Incremental reasoning/thinking text.
    ReasoningDelta { text: String },
    /// A tool call has started executing.
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
    /// Turn metrics summary (emitted before stream close).
    TurnMetrics {
        duration_ms: u64,
        iterations:  usize,
        tool_calls:  usize,
        model:       String,
    },
}

// ---------------------------------------------------------------------------
// StreamEntry (internal)
// ---------------------------------------------------------------------------

/// Internal entry in the stream table.
struct StreamEntry {
    session_id: SessionId,
    tx:         broadcast::Sender<StreamEvent>,
}

// ---------------------------------------------------------------------------
// StreamHandle
// ---------------------------------------------------------------------------

/// Handle held by the agent executor to emit stream events.
///
/// Created by [`StreamHub::open`]. The agent emits events via
/// [`emit`](Self::emit).
pub struct StreamHandle {
    stream_id: StreamId,
    tx:        broadcast::Sender<StreamEvent>,
}

impl StreamHandle {
    /// Get the stream ID.
    pub fn stream_id(&self) -> &StreamId { &self.stream_id }

    /// Emit a stream event. Silently drops if no subscribers.
    pub fn emit(&self, event: StreamEvent) { let _ = self.tx.send(event); }
}

// ---------------------------------------------------------------------------
// StreamHub
// ---------------------------------------------------------------------------

/// Central registry for active ephemeral streams.
///
/// Manages the lifecycle of per-execution streams and provides
/// subscription endpoints for egress/frontends.
pub struct StreamHub {
    streams:  DashMap<StreamId, StreamEntry>,
    capacity: usize,
}

impl StreamHub {
    /// Create a new hub with the given per-stream broadcast capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            streams: DashMap::new(),
            capacity,
        }
    }

    /// Open a new stream for an agent execution run.
    ///
    /// Returns a [`StreamHandle`] that the executor uses to emit events.
    #[tracing::instrument(skip(self), fields(stream_id = tracing::field::Empty))]
    pub fn open(&self, session_id: SessionId) -> StreamHandle {
        let stream_id = StreamId::new();
        tracing::Span::current().record("stream_id", stream_id.0.as_str());
        let (tx, _) = broadcast::channel(self.capacity);
        let entry = StreamEntry {
            session_id,
            tx: tx.clone(),
        };
        self.streams.insert(stream_id.clone(), entry);
        StreamHandle { stream_id, tx }
    }

    /// Close a stream by its ID.
    ///
    /// This is precise — only the specified stream is removed, not other
    /// streams on the same session.
    #[tracing::instrument(skip(self))]
    pub fn close(&self, stream_id: &StreamId) { self.streams.remove(stream_id); }

    /// Subscribe to all active streams for a given session.
    ///
    /// Returns a list of `(StreamId, Receiver)` pairs. Multiple streams
    /// may exist if the session has concurrent agent runs.
    pub fn subscribe_session(
        &self,
        session_id: &SessionId,
    ) -> Vec<(StreamId, broadcast::Receiver<StreamEvent>)> {
        self.streams
            .iter()
            .filter(|entry| &entry.value().session_id == session_id)
            .map(|entry| (entry.key().clone(), entry.value().tx.subscribe()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stream_open_close() {
        let hub = StreamHub::new(16);
        let session = SessionId::new("s1");

        let handle = hub.open(session.clone());
        let sid = handle.stream_id().clone();

        // Stream should exist.
        assert!(hub.streams.contains_key(&sid));

        hub.close(&sid);

        // Stream should be gone.
        assert!(!hub.streams.contains_key(&sid));
    }

    #[tokio::test]
    async fn test_stream_concurrent_sessions() {
        let hub = StreamHub::new(16);
        let session = SessionId::new("s1");

        let h1 = hub.open(session.clone());
        let h2 = hub.open(session.clone());

        // Two separate streams on the same session.
        assert_ne!(h1.stream_id(), h2.stream_id());
        assert_eq!(hub.streams.len(), 2);

        // Both should appear in subscribe_session.
        let subs = hub.subscribe_session(&session);
        assert_eq!(subs.len(), 2);

        hub.close(h1.stream_id());
        assert_eq!(hub.streams.len(), 1);

        hub.close(h2.stream_id());
        assert_eq!(hub.streams.len(), 0);
    }

    #[tokio::test]
    async fn test_stream_subscribe_receives() {
        let hub = StreamHub::new(16);
        let session = SessionId::new("s1");

        let handle = hub.open(session.clone());

        let subs = hub.subscribe_session(&session);
        assert_eq!(subs.len(), 1);

        let (_, mut rx) = subs.into_iter().next().unwrap();

        handle.emit(StreamEvent::TextDelta {
            text: "hello".to_string(),
        });
        handle.emit(StreamEvent::Progress {
            stage: "thinking".to_string(),
        });

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, StreamEvent::TextDelta { ref text } if text == "hello"));

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, StreamEvent::Progress { ref stage } if stage == "thinking"));
    }

    #[tokio::test]
    async fn test_stream_no_subscriber_no_error() {
        let hub = StreamHub::new(16);
        let session = SessionId::new("s1");

        let handle = hub.open(session);

        // Emit without any subscriber — should not panic.
        handle.emit(StreamEvent::TextDelta {
            text: "ignored".to_string(),
        });
        handle.emit(StreamEvent::ToolCallStart {
            name:      "read_file".to_string(),
            id:        "tc-1".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test"}),
        });
        handle.emit(StreamEvent::ToolCallEnd {
            id:             "tc-1".to_string(),
            result_preview: "ok".to_string(),
            success:        true,
            error:          None,
        });
    }
}
