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

//! Kernel notifications — inter-component event broadcasting.

use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use tokio::sync::broadcast;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// Notifications emitted by the kernel during agent execution.
#[derive(Debug, Clone, strum::IntoStaticStr)]
pub enum KernelNotification {
    /// A tool was executed.
    ToolExecuted {
        session_key: SessionKey,
        tool_name:   String,
        success:     bool,
        timestamp:   Timestamp,
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
pub struct NotificationFilter {
    /// Only receive events for this agent (None = all agents).
    pub agent_id:    Option<Uuid>,
    /// Only receive these event types (empty = all types).
    pub event_types: Vec<String>,
}

/// A stream of kernel notifications.
pub type NotificationStream = tokio::sync::broadcast::Receiver<KernelNotification>;

// ---------------------------------------------------------------------------
// NotificationBus trait
// ---------------------------------------------------------------------------

pub type NotificationBusRef = Arc<dyn NotificationBus>;

/// Inter-component notification broadcasting.
#[async_trait]
pub trait NotificationBus: Send + Sync {
    /// Publish a notification to all subscribers.
    async fn publish(&self, event: KernelNotification);

    /// Subscribe to notifications matching the given filter.
    async fn subscribe(&self, filter: NotificationFilter) -> NotificationStream;
}

// ---------------------------------------------------------------------------
// BroadcastNotificationBus
// ---------------------------------------------------------------------------

/// Notification bus backed by `tokio::sync::broadcast`.
pub struct BroadcastNotificationBus {
    sender: broadcast::Sender<KernelNotification>,
}

impl BroadcastNotificationBus {
    /// Create a new broadcast notification bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }
}

impl Default for BroadcastNotificationBus {
    fn default() -> Self { Self::new(256) }
}

#[async_trait]
impl NotificationBus for BroadcastNotificationBus {
    async fn publish(&self, event: KernelNotification) {
        // Ignore send errors (no active subscribers).
        let _ = self.sender.send(event);
    }

    async fn subscribe(&self, _filter: NotificationFilter) -> NotificationStream {
        self.sender.subscribe()
    }
}

use crate::session::SessionKey;
