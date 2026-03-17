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

//! Tag-based task notification subscriptions.
//!
//! Manages per-user, per-tag subscriptions so that [`TaskNotification`]s
//! produced by background/scheduled tasks are routed to interested sessions.
//! Separate from the kernel event bus (`bus` module) which is a fire-and-forget
//! broadcast channel.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{identity::UserId, session::SessionKey, task_report::TaskReportStatus};

// ---------------------------------------------------------------------------
// Task notification types
// ---------------------------------------------------------------------------

/// Lightweight notification delivered when a TaskReport is published.
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
    /// Owner identity — only reports from the same user are delivered.
    pub owner:      UserId,
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
/// Uses an inverted index `(UserId, tag) → {sub_id}` so that `match_tags`
/// is O(M) hash lookups (M = number of report tags) instead of a full scan
/// over all subscriptions.
///
/// Thread-safe: guarded by a tokio `RwLock` for concurrent read access
/// during notification fan-out.
pub struct SubscriptionRegistry {
    inner: tokio::sync::RwLock<RegistryInner>,
}

/// Interior state behind the `RwLock`.
struct RegistryInner {
    /// Primary store: sub_id → Subscription.
    subs:      HashMap<Uuid, Subscription>,
    /// Inverted index: (owner, tag) → set of sub_ids.
    tag_index: HashMap<(UserId, String), HashSet<Uuid>>,
}

impl RegistryInner {
    fn new() -> Self {
        Self {
            subs:      HashMap::new(),
            tag_index: HashMap::new(),
        }
    }
}

impl SubscriptionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::RwLock::new(RegistryInner::new()),
        }
    }

    /// Register a new subscription. Returns the subscription ID.
    ///
    /// `owner` scopes the subscription — only reports published by the same
    /// user will be delivered, preventing cross-user data leakage.
    pub async fn subscribe(
        &self,
        subscriber: SessionKey,
        owner: UserId,
        match_tags: Vec<String>,
        on_receive: NotifyAction,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let sub = Subscription {
            id,
            subscriber,
            owner,
            match_tags,
            on_receive,
        };
        let mut inner = self.inner.write().await;
        for tag in &sub.match_tags {
            inner
                .tag_index
                .entry((sub.owner.clone(), tag.clone()))
                .or_default()
                .insert(id);
        }
        inner.subs.insert(id, sub);
        id
    }

    /// Remove a subscription by ID, only if owned by `caller`.
    /// Returns true if it existed and was removed.
    pub async fn unsubscribe(&self, subscription_id: Uuid, caller: &UserId) -> bool {
        let mut inner = self.inner.write().await;
        // Check ownership before removing.
        let owned = inner
            .subs
            .get(&subscription_id)
            .is_some_and(|sub| sub.owner == *caller);
        if !owned {
            return false;
        }
        if let Some(sub) = inner.subs.remove(&subscription_id) {
            for tag in &sub.match_tags {
                let key = (sub.owner.clone(), tag.clone());
                if let Some(set) = inner.tag_index.get_mut(&key) {
                    set.remove(&subscription_id);
                    if set.is_empty() {
                        inner.tag_index.remove(&key);
                    }
                }
            }
            true
        } else {
            false
        }
    }

    /// Find all subscriptions matching any of the given tags, scoped to the
    /// publisher's owner. O(M) hash lookups where M = number of report tags.
    pub async fn match_tags(&self, tags: &[String], publisher: &UserId) -> Vec<Subscription> {
        let inner = self.inner.read().await;
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for tag in tags {
            if let Some(ids) = inner.tag_index.get(&(publisher.clone(), tag.clone())) {
                for id in ids {
                    if seen.insert(*id) {
                        if let Some(sub) = inner.subs.get(id) {
                            result.push(sub.clone());
                        }
                    }
                }
            }
        }
        result
    }

    /// Remove all subscriptions for a given session (cleanup on session end).
    pub async fn remove_session(&self, session_key: &SessionKey) {
        let mut inner = self.inner.write().await;
        let to_remove: Vec<Uuid> = inner
            .subs
            .values()
            .filter(|sub| &sub.subscriber == session_key)
            .map(|sub| sub.id)
            .collect();
        for id in to_remove {
            if let Some(sub) = inner.subs.remove(&id) {
                for tag in &sub.match_tags {
                    let key = (sub.owner.clone(), tag.clone());
                    if let Some(set) = inner.tag_index.get_mut(&key) {
                        set.remove(&id);
                        if set.is_empty() {
                            inner.tag_index.remove(&key);
                        }
                    }
                }
            }
        }
    }
}

impl Default for SubscriptionRegistry {
    fn default() -> Self { Self::new() }
}

/// Shared reference to a subscription registry.
pub type SubscriptionRegistryRef = Arc<SubscriptionRegistry>;
