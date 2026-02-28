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

//! Terminal channel adapter — CLI interactive chat implementation.
//!
//! Unlike the Telegram and Web adapters, the `TerminalAdapter` does NOT
//! implement [`ChannelAdapter`] (no start/stop lifecycle needed). It only
//! implements [`EgressAdapter`] to receive outbound messages from the
//! egress pipeline and forward them as [`CliEvent`]s through an mpsc
//! channel for the REPL loop to render.

use async_trait::async_trait;
use rara_kernel::{
    channel::types::ChannelType,
    io::egress::{EgressAdapter, EgressError, Endpoint, EndpointAddress, PlatformOutbound},
};
use tokio::sync::mpsc;
use tracing::debug;

// ---------------------------------------------------------------------------
// CliEvent
// ---------------------------------------------------------------------------

/// Events emitted by the terminal adapter for the CLI REPL to render.
#[derive(Debug, Clone)]
pub enum CliEvent {
    /// A complete reply from the agent.
    Reply { content: String },
    /// Incremental text output from LLM.
    TextDelta { text: String },
    /// Incremental reasoning/thinking text.
    ReasoningDelta { text: String },
    /// A tool call has started.
    ToolCallStart { name: String },
    /// A tool call has finished.
    ToolCallEnd,
    /// Progress stage update.
    Progress { text: String },
    /// Error notification.
    Error { message: String },
    /// Stream completed.
    Done,
}

// ---------------------------------------------------------------------------
// TerminalAdapter
// ---------------------------------------------------------------------------

/// Terminal channel adapter for CLI interactive chat.
///
/// Converts [`PlatformOutbound`] messages into [`CliEvent`]s and sends
/// them through an mpsc channel for the REPL loop to consume.
///
/// # Usage
///
/// ```rust,ignore
/// let (adapter, event_rx) = TerminalAdapter::new();
/// // Register adapter with egress pipeline
/// // Spawn REPL loop consuming event_rx
/// ```
pub struct TerminalAdapter {
    event_tx: mpsc::UnboundedSender<CliEvent>,
}

impl TerminalAdapter {
    /// Create a new `TerminalAdapter` and its event receiver.
    ///
    /// Returns the adapter (to be registered with the egress pipeline)
    /// and an unbounded receiver for the REPL loop to consume events.
    pub fn new() -> (Self, mpsc::UnboundedReceiver<CliEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        (Self { event_tx }, event_rx)
    }

    /// Send a [`CliEvent`] to the REPL loop.
    ///
    /// Returns `Ok(())` if the event was sent, `Err(CliEvent)` if the
    /// receiver has been dropped.
    pub fn send_cli_event(&self, event: CliEvent) -> Result<(), CliEvent> {
        self.event_tx.send(event).map_err(|e| e.0)
    }

    /// Send a [`CliEvent`] to the REPL loop (internal, ignores errors).
    fn send_event(&self, event: CliEvent) {
        if self.event_tx.send(event).is_err() {
            debug!("CLI event receiver dropped, ignoring event");
        }
    }
}

// ---------------------------------------------------------------------------
// EgressAdapter trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl EgressAdapter for TerminalAdapter {
    fn channel_type(&self) -> ChannelType { ChannelType::Cli }

    async fn send(&self, endpoint: &Endpoint, msg: PlatformOutbound) -> Result<(), EgressError> {
        // Only handle CLI endpoints.
        match &endpoint.address {
            EndpointAddress::Cli { .. } => {}
            _ => return Ok(()),
        }

        let event = match msg {
            PlatformOutbound::Reply { content, .. } => CliEvent::Reply { content },
            PlatformOutbound::StreamChunk { delta, .. } => CliEvent::TextDelta { text: delta },
            PlatformOutbound::Progress { text, .. } => CliEvent::Progress { text },
        };

        self.send_event(event);
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
    fn terminal_adapter_channel_type_is_cli() {
        let (adapter, _rx) = TerminalAdapter::new();
        assert_eq!(EgressAdapter::channel_type(&adapter), ChannelType::Cli);
    }

    #[tokio::test]
    async fn egress_send_reply_produces_cli_event() {
        let (adapter, mut rx) = TerminalAdapter::new();

        let endpoint = Endpoint {
            channel_type: ChannelType::Cli,
            address:      EndpointAddress::Cli {
                session_id: "cli-sess".to_owned(),
            },
        };

        let msg = PlatformOutbound::Reply {
            session_key:   "cli:cli-sess".to_owned(),
            content:       "hello from agent".to_owned(),
            attachments:   vec![],
            reply_context: None,
        };

        EgressAdapter::send(&adapter, &endpoint, msg).await.unwrap();

        let event = rx.try_recv().unwrap();
        match event {
            CliEvent::Reply { content } => assert_eq!(content, "hello from agent"),
            other => panic!("expected Reply, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn egress_send_stream_chunk_produces_text_delta() {
        let (adapter, mut rx) = TerminalAdapter::new();

        let endpoint = Endpoint {
            channel_type: ChannelType::Cli,
            address:      EndpointAddress::Cli {
                session_id: "cli-sess".to_owned(),
            },
        };

        let msg = PlatformOutbound::StreamChunk {
            session_key: "cli:cli-sess".to_owned(),
            delta:       "token".to_owned(),
            edit_target: None,
        };

        EgressAdapter::send(&adapter, &endpoint, msg).await.unwrap();

        let event = rx.try_recv().unwrap();
        match event {
            CliEvent::TextDelta { text } => assert_eq!(text, "token"),
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn egress_send_progress_produces_progress_event() {
        let (adapter, mut rx) = TerminalAdapter::new();

        let endpoint = Endpoint {
            channel_type: ChannelType::Cli,
            address:      EndpointAddress::Cli {
                session_id: "cli-sess".to_owned(),
            },
        };

        let msg = PlatformOutbound::Progress {
            session_key: "cli:cli-sess".to_owned(),
            text:        "thinking...".to_owned(),
        };

        EgressAdapter::send(&adapter, &endpoint, msg).await.unwrap();

        let event = rx.try_recv().unwrap();
        match event {
            CliEvent::Progress { text } => assert_eq!(text, "thinking..."),
            other => panic!("expected Progress, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn egress_send_ignores_non_cli_endpoint() {
        let (adapter, mut rx) = TerminalAdapter::new();

        let endpoint = Endpoint {
            channel_type: ChannelType::Web,
            address:      EndpointAddress::Web {
                connection_id: "web-conn".to_owned(),
            },
        };

        let msg = PlatformOutbound::Reply {
            session_key:   "web:web-conn".to_owned(),
            content:       "should be ignored".to_owned(),
            attachments:   vec![],
            reply_context: None,
        };

        EgressAdapter::send(&adapter, &endpoint, msg).await.unwrap();

        // No event should be produced for non-CLI endpoints.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn send_event_after_receiver_drop_does_not_panic() {
        let (adapter, rx) = TerminalAdapter::new();
        drop(rx);

        // Should not panic — just silently drops the event.
        adapter.send_event(CliEvent::Done);
    }
}
