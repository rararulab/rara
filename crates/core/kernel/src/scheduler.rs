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

//! LLM call scheduler — priority queue + token budget rate limiting.
//!
//! The [`PriorityScheduler`] sits between the [`InboundBus`] and the dispatch
//! logic in [`TickLoop`]. Instead of processing messages in FIFO order, it
//! reorders them by [`Priority`] and enforces per-agent and per-user token
//! budgets via a sliding-window [`UsageTracker`].
//!
//! # Design
//!
//! - Messages are wrapped in [`PrioritizedMessage`] and inserted into a
//!   [`BinaryHeap`] (max-heap by priority, then by arrival order).
//! - Before dispatching, the scheduler checks the [`UsageTracker`] to see if
//!   the agent or user has exceeded their token budget for the current window.
//! - [`Priority::Critical`] messages bypass rate limiting entirely.
//! - Deferred messages are kept in the heap and retried on the next tick.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use tracing::{debug, warn};

use crate::io::types::InboundMessage;
use crate::process::Priority;

// ---------------------------------------------------------------------------
// TokenBudget
// ---------------------------------------------------------------------------

/// Token budget configuration for rate limiting.
///
/// Budgets are best-effort — we use estimated token counts since exact
/// counts are only available after the LLM call completes. The scheduler
/// records usage reported by callers via [`UsageTracker::record`].
#[derive(Debug, Clone)]
pub struct TokenBudget {
    /// Maximum tokens per agent per window (None = unlimited).
    pub per_agent_limit: Option<u64>,
    /// Maximum tokens per user per window (None = unlimited).
    pub per_user_limit:  Option<u64>,
    /// Global token limit across all agents/users (None = unlimited).
    pub global_limit:    Option<u64>,
    /// Sliding window duration for rate limiting.
    pub window:          Duration,
}

impl Default for TokenBudget {
    fn default() -> Self {
        Self {
            per_agent_limit: None,
            per_user_limit:  None,
            global_limit:    None,
            window:          Duration::from_secs(60),
        }
    }
}

// ---------------------------------------------------------------------------
// UsageRecord
// ---------------------------------------------------------------------------

/// A single token usage record with timestamp.
#[derive(Debug, Clone)]
struct UsageRecord {
    tokens: u64,
    at:     Instant,
}

// ---------------------------------------------------------------------------
// UsageTracker
// ---------------------------------------------------------------------------

/// Sliding-window token usage tracker.
///
/// Tracks token consumption per agent, per user, and globally. Old records
/// outside the window are lazily pruned on each query.
#[derive(Debug)]
pub struct UsageTracker {
    /// Per-agent token usage history.
    agent_usage:  HashMap<String, VecDeque<UsageRecord>>,
    /// Per-user token usage history.
    user_usage:   HashMap<String, VecDeque<UsageRecord>>,
    /// Global token usage history.
    global_usage: VecDeque<UsageRecord>,
    /// Sliding window duration.
    window:       Duration,
}

impl UsageTracker {
    /// Create a new tracker with the given window duration.
    pub fn new(window: Duration) -> Self {
        Self {
            agent_usage:  HashMap::new(),
            user_usage:   HashMap::new(),
            global_usage: VecDeque::new(),
            window,
        }
    }

    /// Record token usage for an agent and user.
    pub fn record(&mut self, agent_name: &str, user_id: &str, tokens: u64) {
        let now = Instant::now();
        let record = UsageRecord { tokens, at: now };

        self.agent_usage
            .entry(agent_name.to_string())
            .or_default()
            .push_back(record.clone());

        self.user_usage
            .entry(user_id.to_string())
            .or_default()
            .push_back(record.clone());

        self.global_usage.push_back(record);
    }

    /// Get current token usage for an agent within the window.
    pub fn agent_usage(&mut self, agent_name: &str) -> u64 {
        self.prune_old_records();
        self.agent_usage
            .get(agent_name)
            .map(|records| records.iter().map(|r| r.tokens).sum())
            .unwrap_or(0)
    }

    /// Get current token usage for a user within the window.
    pub fn user_usage(&mut self, user_id: &str) -> u64 {
        self.prune_old_records();
        self.user_usage
            .get(user_id)
            .map(|records| records.iter().map(|r| r.tokens).sum())
            .unwrap_or(0)
    }

    /// Get current global token usage within the window.
    pub fn global_usage(&mut self) -> u64 {
        self.prune_old_records();
        self.global_usage.iter().map(|r| r.tokens).sum()
    }

    /// Check if an agent is within budget.
    pub fn agent_within_budget(&mut self, agent_name: &str, limit: u64) -> bool {
        self.agent_usage(agent_name) < limit
    }

    /// Check if a user is within budget.
    pub fn user_within_budget(&mut self, user_id: &str, limit: u64) -> bool {
        self.user_usage(user_id) < limit
    }

    /// Check if global usage is within budget.
    pub fn global_within_budget(&mut self, limit: u64) -> bool {
        self.global_usage() < limit
    }

    /// Remove records older than the window from all tracking maps.
    fn prune_old_records(&mut self) {
        let cutoff = Instant::now() - self.window;

        for records in self.agent_usage.values_mut() {
            while records.front().is_some_and(|r| r.at < cutoff) {
                records.pop_front();
            }
        }
        // Remove empty entries
        self.agent_usage.retain(|_, v| !v.is_empty());

        for records in self.user_usage.values_mut() {
            while records.front().is_some_and(|r| r.at < cutoff) {
                records.pop_front();
            }
        }
        self.user_usage.retain(|_, v| !v.is_empty());

        while self.global_usage.front().is_some_and(|r| r.at < cutoff) {
            self.global_usage.pop_front();
        }
    }
}

// ---------------------------------------------------------------------------
// PrioritizedMessage
// ---------------------------------------------------------------------------

/// An inbound message wrapped with scheduling metadata.
///
/// The `Ord` implementation sorts by priority (higher first), then by
/// sequence number (lower first = FIFO within same priority).
#[derive(Debug)]
pub struct PrioritizedMessage {
    /// The wrapped inbound message.
    pub message:  InboundMessage,
    /// Dispatch priority.
    pub priority: Priority,
    /// Monotonically increasing sequence number for FIFO ordering within
    /// the same priority level.
    pub seq:      u64,
}

impl PartialEq for PrioritizedMessage {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}

impl Eq for PrioritizedMessage {}

impl PartialOrd for PrioritizedMessage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

impl Ord for PrioritizedMessage {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority first; within same priority, lower seq first (FIFO).
        match self.priority.cmp(&other.priority) {
            Ordering::Equal => other.seq.cmp(&self.seq), // reversed: lower seq = higher in heap
            ord => ord,
        }
    }
}

// ---------------------------------------------------------------------------
// SchedulerConfig
// ---------------------------------------------------------------------------

/// Configuration for the priority scheduler.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Token budget configuration.
    pub budget:           TokenBudget,
    /// Default priority for messages without an explicit priority.
    pub default_priority: Priority,
    /// Maximum number of deferred messages to keep in the queue.
    /// Messages beyond this limit are dropped with a warning.
    pub max_deferred:     usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            budget:           TokenBudget::default(),
            default_priority: Priority::Normal,
            max_deferred:     1024,
        }
    }
}

// ---------------------------------------------------------------------------
// PriorityScheduler
// ---------------------------------------------------------------------------

/// LLM call scheduler with priority queue and token budget enforcement.
///
/// The scheduler accepts inbound messages, orders them by priority, and
/// yields them for dispatch while respecting rate limits. Messages that
/// exceed their budget are deferred and retried on the next tick.
pub struct PriorityScheduler {
    /// Priority queue of pending messages.
    queue:   BinaryHeap<PrioritizedMessage>,
    /// Token usage tracker.
    usage:   UsageTracker,
    /// Configuration.
    config:  SchedulerConfig,
    /// Monotonically increasing sequence counter.
    next_seq: u64,
}

impl PriorityScheduler {
    /// Create a new scheduler with the given configuration.
    pub fn new(config: SchedulerConfig) -> Self {
        let window = config.budget.window;
        Self {
            queue:    BinaryHeap::new(),
            usage:    UsageTracker::new(window),
            config,
            next_seq: 0,
        }
    }

    /// Enqueue a batch of inbound messages for scheduling.
    ///
    /// Each message gets the default priority unless overridden.
    pub fn enqueue(&mut self, messages: Vec<InboundMessage>, priority: Priority) {
        for msg in messages {
            if self.queue.len() >= self.config.max_deferred {
                warn!(
                    user = %msg.user.0,
                    "scheduler queue full, dropping message"
                );
                continue;
            }
            let seq = self.next_seq;
            self.next_seq += 1;
            self.queue.push(PrioritizedMessage {
                message: msg,
                priority,
                seq,
            });
        }
    }

    /// Enqueue a single message with a specific priority.
    pub fn enqueue_one(&mut self, msg: InboundMessage, priority: Priority) {
        if self.queue.len() >= self.config.max_deferred {
            warn!(
                user = %msg.user.0,
                "scheduler queue full, dropping message"
            );
            return;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.queue.push(PrioritizedMessage {
            message: msg,
            priority,
            seq,
        });
    }

    /// Drain up to `limit` messages that are within budget.
    ///
    /// Messages that exceed their budget are kept in the queue for the
    /// next tick. Critical-priority messages always pass.
    ///
    /// Returns the messages ready for dispatch.
    pub fn drain_ready(&mut self, limit: usize) -> Vec<InboundMessage> {
        let mut ready = Vec::with_capacity(limit);
        let mut deferred = Vec::new();

        while ready.len() < limit {
            let Some(item) = self.queue.pop() else {
                break;
            };

            // Critical messages always pass.
            if item.priority == Priority::Critical {
                ready.push(item.message);
                continue;
            }

            // Check budgets.
            if self.is_within_budget(&item.message) {
                ready.push(item.message);
            } else {
                debug!(
                    user = %item.message.user.0,
                    priority = %item.priority,
                    "message deferred: budget exceeded"
                );
                deferred.push(item);
            }
        }

        // Put deferred messages back.
        for item in deferred {
            self.queue.push(item);
        }

        ready
    }

    /// Record token usage after an LLM call completes.
    ///
    /// This is called by the kernel process loop after receiving the LLM
    /// response, so the scheduler can track consumption for rate limiting.
    pub fn record_usage(&mut self, agent_name: &str, user_id: &str, tokens: u64) {
        self.usage.record(agent_name, user_id, tokens);
    }

    /// Access the scheduler configuration.
    pub fn config(&self) -> &SchedulerConfig { &self.config }

    /// Number of messages currently queued (including deferred).
    pub fn pending_count(&self) -> usize { self.queue.len() }

    /// Access the usage tracker for querying current usage.
    pub fn usage_tracker(&mut self) -> &mut UsageTracker { &mut self.usage }

    /// Check if a message is within all configured budgets.
    fn is_within_budget(&mut self, msg: &InboundMessage) -> bool {
        let user_id = &msg.user.0;

        // Check global budget.
        if let Some(limit) = self.config.budget.global_limit {
            if !self.usage.global_within_budget(limit) {
                return false;
            }
        }

        // Check per-user budget.
        if let Some(limit) = self.config.budget.per_user_limit {
            if !self.usage.user_within_budget(user_id, limit) {
                return false;
            }
        }

        // Per-agent budget is checked at dispatch time since we don't know
        // which agent will handle this message yet. For now, we check global
        // and per-user only.

        true
    }
}

impl std::fmt::Debug for PriorityScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PriorityScheduler")
            .field("pending", &self.queue.len())
            .field("next_seq", &self.next_seq)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// KernelConfig extensions
// ---------------------------------------------------------------------------

// Note: SchedulerConfig is integrated into KernelConfig in kernel.rs.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::*;
    use crate::{
        channel::types::{ChannelType, MessageContent},
        io::types::{ChannelSource, MessageId},
        process::{SessionId, principal::UserId},
    };

    /// Helper: build a test InboundMessage.
    fn test_msg(user: &str, session: &str) -> InboundMessage {
        InboundMessage {
            id:            MessageId::new(),
            source:        ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    user.to_string(),
                platform_chat_id:    None,
            },
            user:          UserId(user.to_string()),
            session_id:    SessionId::new(session),
            content:       MessageContent::Text("hello".to_string()),
            reply_context: None,
            timestamp:     jiff::Timestamp::now(),
            metadata:      HashMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Priority ordering tests
    // -----------------------------------------------------------------------

    #[test]
    fn priority_ordering() {
        assert!(Priority::Critical > Priority::High);
        assert!(Priority::High > Priority::Normal);
        assert!(Priority::Normal > Priority::Low);
    }

    #[test]
    fn priority_default_is_normal() {
        assert_eq!(Priority::default(), Priority::Normal);
    }

    #[test]
    fn priority_display() {
        assert_eq!(Priority::Low.to_string(), "low");
        assert_eq!(Priority::Normal.to_string(), "normal");
        assert_eq!(Priority::High.to_string(), "high");
        assert_eq!(Priority::Critical.to_string(), "critical");
    }

    // -----------------------------------------------------------------------
    // PrioritizedMessage ordering tests
    // -----------------------------------------------------------------------

    #[test]
    fn prioritized_message_higher_priority_first() {
        let mut heap = BinaryHeap::new();

        heap.push(PrioritizedMessage {
            message:  test_msg("u1", "s1"),
            priority: Priority::Low,
            seq:      0,
        });
        heap.push(PrioritizedMessage {
            message:  test_msg("u2", "s2"),
            priority: Priority::High,
            seq:      1,
        });
        heap.push(PrioritizedMessage {
            message:  test_msg("u3", "s3"),
            priority: Priority::Normal,
            seq:      2,
        });

        let first = heap.pop().unwrap();
        assert_eq!(first.priority, Priority::High);

        let second = heap.pop().unwrap();
        assert_eq!(second.priority, Priority::Normal);

        let third = heap.pop().unwrap();
        assert_eq!(third.priority, Priority::Low);
    }

    #[test]
    fn prioritized_message_fifo_within_same_priority() {
        let mut heap = BinaryHeap::new();

        heap.push(PrioritizedMessage {
            message:  test_msg("u1", "s1"),
            priority: Priority::Normal,
            seq:      0,
        });
        heap.push(PrioritizedMessage {
            message:  test_msg("u2", "s2"),
            priority: Priority::Normal,
            seq:      1,
        });
        heap.push(PrioritizedMessage {
            message:  test_msg("u3", "s3"),
            priority: Priority::Normal,
            seq:      2,
        });

        // Within same priority, lower seq (earlier arrival) comes first.
        let first = heap.pop().unwrap();
        assert_eq!(first.seq, 0);
        let second = heap.pop().unwrap();
        assert_eq!(second.seq, 1);
        let third = heap.pop().unwrap();
        assert_eq!(third.seq, 2);
    }

    // -----------------------------------------------------------------------
    // UsageTracker tests
    // -----------------------------------------------------------------------

    #[test]
    fn usage_tracker_records_and_queries() {
        let mut tracker = UsageTracker::new(Duration::from_secs(60));

        tracker.record("agent-a", "user-1", 100);
        tracker.record("agent-a", "user-1", 200);
        tracker.record("agent-b", "user-2", 50);

        assert_eq!(tracker.agent_usage("agent-a"), 300);
        assert_eq!(tracker.agent_usage("agent-b"), 50);
        assert_eq!(tracker.agent_usage("agent-c"), 0);

        assert_eq!(tracker.user_usage("user-1"), 300);
        assert_eq!(tracker.user_usage("user-2"), 50);

        assert_eq!(tracker.global_usage(), 350);
    }

    #[test]
    fn usage_tracker_budget_checks() {
        let mut tracker = UsageTracker::new(Duration::from_secs(60));

        tracker.record("agent-a", "user-1", 100);

        assert!(tracker.agent_within_budget("agent-a", 200));
        assert!(!tracker.agent_within_budget("agent-a", 50));

        assert!(tracker.user_within_budget("user-1", 200));
        assert!(!tracker.user_within_budget("user-1", 50));

        assert!(tracker.global_within_budget(200));
        assert!(!tracker.global_within_budget(50));
    }

    #[test]
    fn usage_tracker_prunes_old_records() {
        // Use a very short window so we can test pruning.
        let mut tracker = UsageTracker::new(Duration::from_millis(1));

        tracker.record("agent-a", "user-1", 1000);

        // Wait for the window to expire.
        std::thread::sleep(Duration::from_millis(5));

        // After pruning, usage should be 0.
        assert_eq!(tracker.agent_usage("agent-a"), 0);
        assert_eq!(tracker.user_usage("user-1"), 0);
        assert_eq!(tracker.global_usage(), 0);
    }

    // -----------------------------------------------------------------------
    // PriorityScheduler tests
    // -----------------------------------------------------------------------

    #[test]
    fn scheduler_drains_in_priority_order() {
        let config = SchedulerConfig::default();
        let mut scheduler = PriorityScheduler::new(config);

        scheduler.enqueue_one(test_msg("u1", "s1"), Priority::Low);
        scheduler.enqueue_one(test_msg("u2", "s2"), Priority::Critical);
        scheduler.enqueue_one(test_msg("u3", "s3"), Priority::Normal);
        scheduler.enqueue_one(test_msg("u4", "s4"), Priority::High);

        let ready = scheduler.drain_ready(10);
        assert_eq!(ready.len(), 4);

        // Verify order: Critical, High, Normal, Low
        assert_eq!(ready[0].user.0, "u2"); // Critical
        assert_eq!(ready[1].user.0, "u4"); // High
        assert_eq!(ready[2].user.0, "u3"); // Normal
        assert_eq!(ready[3].user.0, "u1"); // Low
    }

    #[test]
    fn scheduler_respects_drain_limit() {
        let config = SchedulerConfig::default();
        let mut scheduler = PriorityScheduler::new(config);

        for i in 0..10 {
            scheduler.enqueue_one(
                test_msg(&format!("u{i}"), &format!("s{i}")),
                Priority::Normal,
            );
        }

        let ready = scheduler.drain_ready(3);
        assert_eq!(ready.len(), 3);

        // Remaining should still be in queue
        assert_eq!(scheduler.pending_count(), 7);
    }

    #[test]
    fn scheduler_defers_over_budget_messages() {
        let config = SchedulerConfig {
            budget: TokenBudget {
                per_user_limit: Some(100),
                per_agent_limit: None,
                global_limit: None,
                window: Duration::from_secs(60),
            },
            ..Default::default()
        };
        let mut scheduler = PriorityScheduler::new(config);

        // Record some usage for user-1 that exceeds budget.
        scheduler.record_usage("agent-a", "user-1", 150);

        // Enqueue messages from user-1 (over budget) and user-2 (within).
        scheduler.enqueue_one(test_msg("user-1", "s1"), Priority::Normal);
        scheduler.enqueue_one(test_msg("user-2", "s2"), Priority::Normal);

        let ready = scheduler.drain_ready(10);

        // Only user-2's message should be ready.
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].user.0, "user-2");

        // user-1's message should be deferred (still in queue).
        assert_eq!(scheduler.pending_count(), 1);
    }

    #[test]
    fn scheduler_critical_bypasses_rate_limit() {
        let config = SchedulerConfig {
            budget: TokenBudget {
                per_user_limit: Some(100),
                per_agent_limit: None,
                global_limit: None,
                window: Duration::from_secs(60),
            },
            ..Default::default()
        };
        let mut scheduler = PriorityScheduler::new(config);

        // Exceed user-1's budget.
        scheduler.record_usage("agent-a", "user-1", 200);

        // Critical message from user-1 should still pass.
        scheduler.enqueue_one(test_msg("user-1", "s1"), Priority::Critical);

        let ready = scheduler.drain_ready(10);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].user.0, "user-1");
    }

    #[test]
    fn scheduler_global_budget_limit() {
        let config = SchedulerConfig {
            budget: TokenBudget {
                per_user_limit: None,
                per_agent_limit: None,
                global_limit: Some(500),
                window: Duration::from_secs(60),
            },
            ..Default::default()
        };
        let mut scheduler = PriorityScheduler::new(config);

        // Use up the global budget.
        scheduler.record_usage("agent-a", "user-1", 600);

        // Normal message should be deferred.
        scheduler.enqueue_one(test_msg("user-2", "s1"), Priority::Normal);
        let ready = scheduler.drain_ready(10);
        assert_eq!(ready.len(), 0);
        assert_eq!(scheduler.pending_count(), 1);

        // Critical should still pass.
        scheduler.enqueue_one(test_msg("user-3", "s2"), Priority::Critical);
        let ready = scheduler.drain_ready(10);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].user.0, "user-3");
    }

    #[test]
    fn scheduler_max_deferred_drops_excess() {
        let config = SchedulerConfig {
            max_deferred: 3,
            ..Default::default()
        };
        let mut scheduler = PriorityScheduler::new(config);

        for i in 0..5 {
            scheduler.enqueue_one(
                test_msg(&format!("u{i}"), &format!("s{i}")),
                Priority::Normal,
            );
        }

        // Only 3 should be queued (max_deferred).
        assert_eq!(scheduler.pending_count(), 3);
    }

    #[test]
    fn scheduler_fair_between_users() {
        let config = SchedulerConfig {
            budget: TokenBudget {
                per_user_limit: Some(1000),
                per_agent_limit: None,
                global_limit: None,
                window: Duration::from_secs(60),
            },
            ..Default::default()
        };
        let mut scheduler = PriorityScheduler::new(config);

        // user-1 is at 900/1000, user-2 is at 100/1000
        scheduler.record_usage("agent-a", "user-1", 900);
        scheduler.record_usage("agent-a", "user-2", 100);

        // Both within budget — both should be dispatched.
        scheduler.enqueue_one(test_msg("user-1", "s1"), Priority::Normal);
        scheduler.enqueue_one(test_msg("user-2", "s2"), Priority::Normal);

        let ready = scheduler.drain_ready(10);
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn scheduler_empty_drain() {
        let config = SchedulerConfig::default();
        let mut scheduler = PriorityScheduler::new(config);

        let ready = scheduler.drain_ready(10);
        assert!(ready.is_empty());
        assert_eq!(scheduler.pending_count(), 0);
    }

    #[test]
    fn scheduler_enqueue_batch() {
        let config = SchedulerConfig::default();
        let mut scheduler = PriorityScheduler::new(config);

        let messages = vec![
            test_msg("u1", "s1"),
            test_msg("u2", "s2"),
            test_msg("u3", "s3"),
        ];
        scheduler.enqueue(messages, Priority::High);

        assert_eq!(scheduler.pending_count(), 3);

        let ready = scheduler.drain_ready(10);
        assert_eq!(ready.len(), 3);
    }
}
