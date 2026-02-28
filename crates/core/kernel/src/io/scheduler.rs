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

//! SessionScheduler — per-session serial execution.
//!
//! Ensures that only one agent execution runs per session at a time.
//! Additional messages for the same session are queued (up to a limit),
//! and excess messages are rejected.
//!
//! When an agent finishes, [`release_and_next`](SessionScheduler::release_and_next)
//! returns the next queued message (if any) and cleans up empty slots to
//! prevent memory leaks.

use std::collections::VecDeque;

use dashmap::DashMap;

use crate::io::types::InboundMessage;
use crate::process::SessionId;

// ---------------------------------------------------------------------------
// ScheduleResult
// ---------------------------------------------------------------------------

/// Result of attempting to schedule a message for execution.
#[derive(Debug)]
pub enum ScheduleResult {
    /// Message is ready for immediate execution.
    Ready(InboundMessage),
    /// Message was queued behind a running execution.
    Queued,
    /// Message was rejected because the session queue is full.
    Rejected,
}

// ---------------------------------------------------------------------------
// SessionSlot (internal)
// ---------------------------------------------------------------------------

/// Per-session execution slot.
struct SessionSlot {
    /// Whether an agent is currently running for this session.
    running: bool,
    /// Messages waiting for execution.
    pending: VecDeque<InboundMessage>,
}

// ---------------------------------------------------------------------------
// SessionScheduler
// ---------------------------------------------------------------------------

/// Ensures per-session serial execution of agent processes.
///
/// The scheduler maintains a slot per active session. The first message
/// for an idle session is immediately [`Ready`](ScheduleResult::Ready).
/// Subsequent messages are [`Queued`](ScheduleResult::Queued) until the
/// current execution completes. If the queue exceeds `max_pending_per_session`,
/// the message is [`Rejected`](ScheduleResult::Rejected).
pub struct SessionScheduler {
    slots: DashMap<SessionId, SessionSlot>,
    max_pending_per_session: usize,
}

impl SessionScheduler {
    /// Create a new scheduler with the given per-session queue limit.
    pub fn new(max_pending_per_session: usize) -> Self {
        Self {
            slots: DashMap::new(),
            max_pending_per_session,
        }
    }

    /// Attempt to schedule a message for execution.
    ///
    /// - If no execution is running for this session: marks running and returns `Ready`.
    /// - If running but queue not full: enqueues and returns `Queued`.
    /// - If running and queue full: returns `Rejected`.
    pub fn schedule(&self, msg: InboundMessage) -> ScheduleResult {
        let session_id = msg.session_id.clone();

        let mut entry = self.slots.entry(session_id).or_insert_with(|| SessionSlot {
            running: false,
            pending: VecDeque::new(),
        });

        let slot = entry.value_mut();

        if !slot.running {
            slot.running = true;
            ScheduleResult::Ready(msg)
        } else if slot.pending.len() < self.max_pending_per_session {
            slot.pending.push_back(msg);
            ScheduleResult::Queued
        } else {
            ScheduleResult::Rejected
        }
    }

    /// Release the current execution and return the next queued message.
    ///
    /// If there is a pending message, it is returned and the slot stays
    /// `running = true`. If no pending messages remain, the slot is cleaned
    /// up (removed from the DashMap) to prevent memory leaks.
    pub fn release_and_next(&self, session_id: &SessionId) -> Option<InboundMessage> {
        // We need to handle removal atomically. Use `entry` API for safe access.
        let mut entry = match self.slots.get_mut(session_id) {
            Some(e) => e,
            None => return None,
        };

        let slot = entry.value_mut();

        if let Some(next) = slot.pending.pop_front() {
            // Keep running for the next message.
            return Some(next);
        }

        // No more pending — mark not running.
        slot.running = false;
        drop(entry);

        // Clean up empty slots.
        self.slots
            .remove_if(session_id, |_, slot| !slot.running && slot.pending.is_empty());

        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::channel::types::{ChannelType, MessageContent};
    use crate::io::types::{ChannelSource, MessageId};
    use crate::process::principal::UserId;

    use super::*;

    /// Helper: build a test InboundMessage for a given session.
    fn test_msg(session: &str, text: &str) -> InboundMessage {
        InboundMessage {
            id: MessageId::new(),
            source: ChannelSource {
                channel_type: ChannelType::Telegram,
                platform_message_id: None,
                platform_user_id: "tg-user".to_string(),
                platform_chat_id: None,
            },
            user: UserId("u1".to_string()),
            session_id: SessionId::new(session),
            content: MessageContent::Text(text.to_string()),
            reply_context: None,
            timestamp: jiff::Timestamp::now(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_schedule_first_message_ready() {
        let sched = SessionScheduler::new(5);
        let result = sched.schedule(test_msg("s1", "hello"));
        assert!(matches!(result, ScheduleResult::Ready(_)));
    }

    #[test]
    fn test_schedule_while_running_queued() {
        let sched = SessionScheduler::new(5);

        let r1 = sched.schedule(test_msg("s1", "first"));
        assert!(matches!(r1, ScheduleResult::Ready(_)));

        let r2 = sched.schedule(test_msg("s1", "second"));
        assert!(matches!(r2, ScheduleResult::Queued));
    }

    #[test]
    fn test_schedule_queue_full_rejected() {
        let sched = SessionScheduler::new(1);

        let r1 = sched.schedule(test_msg("s1", "first"));
        assert!(matches!(r1, ScheduleResult::Ready(_)));

        let r2 = sched.schedule(test_msg("s1", "second"));
        assert!(matches!(r2, ScheduleResult::Queued));

        let r3 = sched.schedule(test_msg("s1", "third"));
        assert!(matches!(r3, ScheduleResult::Rejected));
    }

    #[test]
    fn test_release_and_next() {
        let sched = SessionScheduler::new(5);
        let sid = SessionId::new("s1");

        let _r1 = sched.schedule(test_msg("s1", "first"));
        let _r2 = sched.schedule(test_msg("s1", "second"));

        let next = sched.release_and_next(&sid);
        assert!(next.is_some());
        assert_eq!(next.unwrap().content.as_text(), "second");
    }

    #[test]
    fn test_release_empty_cleans_slot() {
        let sched = SessionScheduler::new(5);
        let sid = SessionId::new("s1");

        let _r1 = sched.schedule(test_msg("s1", "only"));

        let next = sched.release_and_next(&sid);
        assert!(next.is_none());

        // Slot should be cleaned up.
        assert!(!sched.slots.contains_key(&sid));
    }

    #[test]
    fn test_release_keeps_running() {
        let sched = SessionScheduler::new(5);
        let sid = SessionId::new("s1");

        let _r1 = sched.schedule(test_msg("s1", "first"));
        let _r2 = sched.schedule(test_msg("s1", "second"));
        let _r3 = sched.schedule(test_msg("s1", "third"));

        // Release first — should return second, keep running.
        let next = sched.release_and_next(&sid);
        assert!(next.is_some());
        assert_eq!(next.unwrap().content.as_text(), "second");

        // Slot should still exist and be running.
        let slot = sched.slots.get(&sid).unwrap();
        assert!(slot.running);
        assert_eq!(slot.pending.len(), 1);
    }
}
