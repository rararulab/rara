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

//! Terminal channel adapter — CLI interactive chat implementation.
//!
//! Unlike the Telegram and Web adapters, the `TerminalAdapter` is
//! egress-only — it implements [`ChannelAdapter`] with only `send()`,
//! forwarding outbound messages as [`CliEvent`]s through an mpsc
//! channel for the REPL loop to render.

use async_trait::async_trait;
use rara_kernel::{
    channel::{adapter::ChannelAdapter, types::ChannelType},
    io::{EgressError, Endpoint, EndpointAddress, PlatformOutbound},
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
    /// LLM's rationale for the current tool call batch.
    TurnRationale { text: String },
    /// A tool call has started.
    ToolCallStart { name: String, summary: String },
    /// A tool call has finished.
    ToolCallEnd {
        success:        bool,
        result_preview: String,
    },
    /// Progress stage update.
    Progress { text: String },
    /// Error notification.
    Error { message: String },
    /// Guard approval request — tool blocked by security policy.
    ApprovalRequest {
        id:         String,
        tool_name:  String,
        summary:    String,
        risk_level: String,
    },
    /// Agent question requiring user answer.
    UserQuestion { id: String, question: String },
    /// Agent paused on tool call limit — user must choose continue or stop.
    ToolCallLimitPaused {
        session_key:     String,
        limit_id:        u64,
        tool_calls_made: usize,
    },
    /// Plan created with goal, total steps, and step descriptions.
    PlanCreated {
        goal:              String,
        total_steps:       u32,
        step_descriptions: Vec<String>,
    },
    /// Plan step progress update.
    PlanProgress {
        current_step: u32,
        total_steps:  u32,
        status_text:  String,
    },
    /// Plan completed with summary.
    PlanCompleted { summary: String },
    /// Per-iteration token usage update (cumulative values from kernel).
    UsageUpdate {
        input_tokens:  u32,
        output_tokens: u32,
        thinking_ms:   u64,
    },
    /// End-of-turn metrics summary.
    TurnSummary {
        duration_ms: u64,
        iterations:  u32,
        tool_calls:  u32,
        model:       String,
    },
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
// ChannelAdapter trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ChannelAdapter for TerminalAdapter {
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
            PlatformOutbound::Progress { text } => CliEvent::Progress { text },
        };

        self.send_event(event);
        Ok(())
    }
}
