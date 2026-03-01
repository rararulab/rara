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

//! Unified kernel event — all kernel interactions as a single enum.
//!
//! Replaces the separate message types (InboundMessage via bus,
//! ProcessMessage via mailbox, OutboundEnvelope via outbound bus) with
//! a single unified event type processed by `Kernel::run()`.

use tokio::sync::oneshot;

use crate::{
    agent_turn::AgentTurnResult,
    io::types::{InboundMessage, MessageId, OutboundEnvelope},
    process::{AgentId, AgentManifest, AgentResult, SessionId, Signal, principal::{Principal, UserId}},
};

// ---------------------------------------------------------------------------
// EventPriority
// ---------------------------------------------------------------------------

/// Auto-inferred priority tier for event queue ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventPriority {
    /// Signal, Shutdown — processed first.
    Critical = 0,
    /// TurnCompleted, ChildCompleted, Deliver — processed second.
    Normal = 1,
    /// UserMessage, SpawnAgent, Timer — processed last.
    Low = 2,
}

// ---------------------------------------------------------------------------
// KernelEvent
// ---------------------------------------------------------------------------

/// Unified event type for all kernel interactions.
///
/// Every interaction with the kernel — user messages, process control,
/// internal callbacks, output delivery — is represented as a `KernelEvent`
/// and processed by the single `Kernel::run()` event loop.
pub enum KernelEvent {
    // === Input: from external sources ===

    /// A new user message from a channel adapter (via IngressPipeline).
    UserMessage(InboundMessage),

    // === Process control ===

    /// Request to spawn a new agent process.
    SpawnAgent {
        manifest:   AgentManifest,
        input:      String,
        principal:  Principal,
        session_id: SessionId,
        parent_id:  Option<AgentId>,
        reply_tx:   oneshot::Sender<crate::error::Result<AgentId>>,
    },

    /// Send a control signal to an agent process.
    SendSignal {
        target: AgentId,
        signal: Signal,
    },

    // === Internal callbacks: from async task completion ===

    /// An LLM turn completed (success or failure).
    TurnCompleted {
        agent_id:    AgentId,
        session_id:  SessionId,
        result:      Result<AgentTurnResult, String>,
        in_reply_to: MessageId,
        user:        UserId,
    },

    /// A child agent process completed.
    ChildCompleted {
        parent_id: AgentId,
        child_id:  AgentId,
        result:    AgentResult,
    },

    // === Output ===

    /// Deliver an outbound envelope to egress.
    Deliver(OutboundEnvelope),

    // === System ===

    /// Timer event (reserved for future use).
    Timer {
        name:    String,
        payload: serde_json::Value,
    },

    /// Graceful shutdown request.
    Shutdown,
}

impl KernelEvent {
    /// Determine the priority tier for this event.
    ///
    /// Priority is auto-inferred from the event variant — callers never
    /// specify it manually.
    pub fn priority(&self) -> EventPriority {
        match self {
            Self::SendSignal { .. } | Self::Shutdown => EventPriority::Critical,
            Self::TurnCompleted { .. }
            | Self::ChildCompleted { .. }
            | Self::Deliver(_) => EventPriority::Normal,
            Self::UserMessage(_)
            | Self::SpawnAgent { .. }
            | Self::Timer { .. } => EventPriority::Low,
        }
    }
}

impl std::fmt::Debug for KernelEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserMessage(msg) => write!(f, "UserMessage(session={})", msg.session_id),
            Self::SpawnAgent { manifest, session_id, .. } => {
                write!(f, "SpawnAgent(name={}, session={})", manifest.name, session_id)
            }
            Self::SendSignal { target, signal } => {
                write!(f, "SendSignal(target={}, signal={:?})", target, signal)
            }
            Self::TurnCompleted { agent_id, .. } => {
                write!(f, "TurnCompleted(agent={})", agent_id)
            }
            Self::ChildCompleted { parent_id, child_id, .. } => {
                write!(f, "ChildCompleted(parent={}, child={})", parent_id, child_id)
            }
            Self::Deliver(env) => write!(f, "Deliver(session={})", env.session_id),
            Self::Timer { name, .. } => write!(f, "Timer(name={})", name),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}
