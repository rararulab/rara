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
use serde::{Deserialize, Serialize};
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

use crate::{session::SessionKey, task_report::TaskReportStatus};

// ---------------------------------------------------------------------------
// Task notification types
// ---------------------------------------------------------------------------

/// Lightweight notification broadcast when a TaskReport is written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNotification {
    /// Unique task identifier.
    pub task_id:      Uuid,
    /// Fixed category (e.g. "pr_review").
    pub task_type:    String,
    /// Routing tags for subscription matching.
    pub tags:         Vec<String>,
    /// Completion status.
    pub status:       TaskReportStatus,
    /// Human-readable one-line summary.
    pub summary:      String,
    /// Task-type-specific structured result from the report.
    pub result:       serde_json::Value,
    /// Action already taken by the task agent, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_taken: Option<String>,
    /// Pointer to the full TaskReport in the source session's tape.
    pub report_ref:   TapeEntryRef,
}

/// Pointer to a specific tape entry in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapeEntryRef {
    /// Session that holds the tape entry.
    pub session_key: SessionKey,
    /// Entry ID within the tape.
    pub entry_id:    u64,
}

/// Action to take when a matching notification arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyAction {
    /// Trigger a proactive LLM turn on the subscriber with the notification
    /// as directive.
    ProactiveTurn,
    /// Silently append a TaskReport entry to the subscriber's tape.
    SilentAppend,
}

/// A session's subscription to task notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// Unique subscription ID.
    pub id:         Uuid,
    /// Session that will receive matching notifications.
    pub subscriber: SessionKey,
    /// Any matching tag triggers delivery.
    pub match_tags: Vec<String>,
    /// What to do when a notification matches.
    pub on_receive: NotifyAction,
}

// ---------------------------------------------------------------------------
// SubscriptionRegistry
// ---------------------------------------------------------------------------

/// In-memory registry of tag-based notification subscriptions.
///
/// Thread-safe: guarded by a tokio `RwLock` for concurrent read access
/// during notification fan-out.
pub struct SubscriptionRegistry {
    subs: tokio::sync::RwLock<std::collections::HashMap<Uuid, Subscription>>,
}

impl SubscriptionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            subs: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Register a new subscription. Returns the subscription ID.
    pub async fn subscribe(
        &self,
        subscriber: SessionKey,
        match_tags: Vec<String>,
        on_receive: NotifyAction,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let sub = Subscription {
            id,
            subscriber,
            match_tags,
            on_receive,
        };
        self.subs.write().await.insert(id, sub);
        id
    }

    /// Remove a subscription by ID. Returns true if it existed.
    pub async fn unsubscribe(&self, subscription_id: Uuid) -> bool {
        self.subs.write().await.remove(&subscription_id).is_some()
    }

    /// Find all subscriptions matching any of the given tags.
    pub async fn match_tags(&self, tags: &[String]) -> Vec<Subscription> {
        let subs = self.subs.read().await;
        subs.values()
            .filter(|sub| sub.match_tags.iter().any(|t| tags.contains(t)))
            .cloned()
            .collect()
    }

    /// Remove all subscriptions for a given session (cleanup on session end).
    pub async fn remove_session(&self, session_key: &SessionKey) {
        let mut subs = self.subs.write().await;
        subs.retain(|_, sub| &sub.subscriber != session_key);
    }
}

impl Default for SubscriptionRegistry {
    fn default() -> Self { Self::new() }
}

/// Shared reference to a subscription registry.
pub type SubscriptionRegistryRef = Arc<SubscriptionRegistry>;
