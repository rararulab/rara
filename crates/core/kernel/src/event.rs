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

//! Event bus abstraction — inter-component event broadcasting.

use async_trait::async_trait;
use jiff::Timestamp;
use tokio::sync::broadcast;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// Events emitted by the kernel during agent execution.
#[derive(Debug, Clone)]
pub enum KernelEvent {
    /// A tool was executed.
    ToolExecuted {
        agent_id:  Uuid,
        tool_name: String,
        success:   bool,
        timestamp: Timestamp,
    },
    /// Memory was updated.
    MemoryUpdated {
        agent_id:  Uuid,
        layer:     String,
        timestamp: Timestamp,
    },
    /// Agent state changed (Idle → Running, etc.).
    AgentStateChanged {
        agent_id:  Uuid,
        old_state: String,
        new_state: String,
        timestamp: Timestamp,
    },
    /// A guard denied a tool call or output.
    GuardDenied {
        agent_id:  Uuid,
        tool_name: String,
        reason:    String,
        timestamp: Timestamp,
    },
}

/// Filter for subscribing to specific events.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Only receive events for this agent (None = all agents).
    pub agent_id:    Option<Uuid>,
    /// Only receive these event types (empty = all types).
    pub event_types: Vec<String>,
}

/// A stream of kernel events.
pub type EventStream = broadcast::Receiver<KernelEvent>;

// ---------------------------------------------------------------------------
// EventBus trait
// ---------------------------------------------------------------------------

/// Inter-component event broadcasting.
#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publish an event to all subscribers.
    async fn publish(&self, event: KernelEvent);

    /// Subscribe to events matching the given filter.
    async fn subscribe(&self, filter: EventFilter) -> EventStream;
}
