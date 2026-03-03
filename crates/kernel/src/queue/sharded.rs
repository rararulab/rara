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

//! Sharded event queue — routes events to N agent-sharded queues + 1 global
//! queue for parallel processing by
//! [`EventProcessor`](crate::processor::EventProcessor)s.
//!
//! Event classification:
//! - **Global**: `UserMessage`, `SpawnAgent`, `Shutdown`, `Deliver`
//! - **Sharded by agent_id**: `Syscall`, `TurnCompleted`, `ChildCompleted`,
//!   `SendSignal`
//!
//! Shard index is computed as `agent_id.0.as_u128() as usize % num_shards`.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;

use super::{in_memory::EventQueue, shard::ShardQueue};
use crate::{event::KernelEvent, io::types::BusError};

/// Shared reference to the [`ShardedEventQueue`].
pub type ShardedQueueRef = Arc<ShardedEventQueue>;

// ---------------------------------------------------------------------------
// ShardTarget — classification result
// ---------------------------------------------------------------------------

/// Where an event should be routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShardTarget {
    /// Route to the global queue (processed by the global EventProcessor).
    Global,
    /// Route to a specific shard (identified by shard index).
    Shard(usize),
}

// ---------------------------------------------------------------------------
// ShardedEventQueue
// ---------------------------------------------------------------------------

/// Configuration for the sharded event queue.
#[derive(Debug, Clone)]
pub struct ShardedEventQueueConfig {
    /// Number of agent shards. Each shard gets its own `EventProcessor`.
    pub num_shards:      usize,
    /// Per-shard capacity (total across all tiers within one shard).
    pub shard_capacity:  usize,
    /// Global queue capacity.
    pub global_capacity: usize,
}

impl ShardedEventQueueConfig {
    /// Single-queue (non-sharded) configuration.
    ///
    /// All events are routed to the global queue and processed by a single
    /// `EventProcessor`. Equivalent to the old `InMemoryEventQueue` path.
    pub fn single() -> Self {
        Self {
            num_shards:      0,
            shard_capacity:  0,
            global_capacity: 4096,
        }
    }
}

impl Default for ShardedEventQueueConfig {
    fn default() -> Self {
        let num_shards = num_cpus().max(2) / 2;
        Self {
            num_shards:      num_shards.max(1),
            shard_capacity:  2048,
            global_capacity: 2048,
        }
    }
}

/// Returns the number of available CPUs (logical cores).
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
}

/// Sharded event queue with N agent shards + 1 global queue.
///
/// Implements [`EventQueue`] and provides internal access to individual
/// shards for the multi-processor event loop.
///
/// All shard queues are stored as `Arc<ShardQueue>` so that
/// [`EventProcessor`](crate::processor::EventProcessor) tasks can
/// hold references to them independently.
pub struct ShardedEventQueue {
    /// Per-agent shards. Events are routed by `agent_id % num_shards`.
    shards:        Vec<Arc<ShardQueue>>,
    /// Global queue for non-agent-scoped events.
    global:        Arc<ShardQueue>,
    /// Total pending events across all shards + global (aggregated).
    total_pending: AtomicUsize,
}

impl ShardedEventQueue {
    /// Create a new sharded event queue with the given configuration.
    pub fn new(config: ShardedEventQueueConfig) -> Self {
        let shards = (0..config.num_shards)
            .map(|_| Arc::new(ShardQueue::new(config.shard_capacity)))
            .collect();
        Self {
            shards,
            global: Arc::new(ShardQueue::new(config.global_capacity)),
            total_pending: AtomicUsize::new(0),
        }
    }

    /// Classify a kernel event into its routing target.
    ///
    /// When `num_shards == 0` (single-queue mode), all events are routed to
    /// the global queue regardless of `agent_id`.
    pub(crate) fn classify(&self, event: &KernelEvent) -> ShardTarget {
        if self.shards.is_empty() {
            return ShardTarget::Global;
        }
        match event.agent_id() {
            Some(agent_id) => {
                let shard_idx = agent_id.0.as_u128() as usize % self.shards.len();
                ShardTarget::Shard(shard_idx)
            }
            None => ShardTarget::Global,
        }
    }

    /// Access a specific shard by index (Arc-wrapped for task sharing).
    pub(crate) fn shard(&self, idx: usize) -> &Arc<ShardQueue> { &self.shards[idx] }

    /// Access the global queue (Arc-wrapped for task sharing).
    pub(crate) fn global(&self) -> &Arc<ShardQueue> { &self.global }

    /// Number of agent shards.
    pub(crate) fn num_shards(&self) -> usize { self.shards.len() }
}

#[async_trait]
impl EventQueue for ShardedEventQueue {
    fn push(&self, event: KernelEvent) -> Result<(), BusError> { self.try_push(event) }

    fn try_push(&self, event: KernelEvent) -> Result<(), BusError> {
        let target = self.classify(&event);
        let result = match target {
            ShardTarget::Global => self.global.push(event),
            ShardTarget::Shard(idx) => self.shards[idx].push(event),
        };
        if result.is_ok() {
            self.total_pending.fetch_add(1, Ordering::Release);
        }
        result
    }

    fn drain(&self, max: usize) -> Vec<KernelEvent> {
        // Drain from global first, then round-robin across shards.
        let mut result = Vec::with_capacity(max);
        let mut remaining = max;

        // Global first
        let global_events = self.global.drain(remaining);
        remaining -= global_events.len();
        result.extend(global_events);

        // Then shards
        for shard in &self.shards {
            if remaining == 0 {
                break;
            }
            let shard_events = shard.drain(remaining);
            remaining -= shard_events.len();
            result.extend(shard_events);
        }

        let drained = result.len();
        if drained > 0 {
            self.total_pending.fetch_sub(drained, Ordering::Release);
        }

        result
    }

    async fn wait(&self) {
        // Fast path: if anything is pending, return immediately.
        if self.total_pending.load(Ordering::Acquire) > 0 {
            return;
        }

        // Wait on the global queue's notify.
        // Each EventProcessor also waits on its own shard independently.
        self.global.wait().await;
    }

    fn pending_count(&self) -> usize { self.total_pending.load(Ordering::Acquire) }

    fn is_sharded(&self) -> bool { !self.shards.is_empty() }
}

impl std::fmt::Debug for ShardedEventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShardedEventQueue")
            .field("num_shards", &self.shards.len())
            .field("total_pending", &self.pending_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{
        channel::types::{ChannelType, MessageContent},
        io::types::{ChannelSource, InboundMessage, MessageId},
        process::{AgentId, SessionId, Signal, principal::UserId},
    };

    fn test_inbound(text: &str) -> InboundMessage {
        InboundMessage {
            id:              MessageId::new(),
            source:          ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    "test".to_string(),
                platform_chat_id:    None,
            },
            user:            UserId("u1".to_string()),
            session_id:      SessionId::new(),
            target_agent_id: None,
            target_agent:    None,
            content:         MessageContent::Text(text.to_string()),
            reply_context:   None,
            timestamp:       jiff::Timestamp::now(),
            metadata:        HashMap::new(),
        }
    }

    fn make_queue(num_shards: usize) -> ShardedEventQueue {
        ShardedEventQueue::new(ShardedEventQueueConfig {
            num_shards,
            shard_capacity: 100,
            global_capacity: 100,
        })
    }

    // -- classify() tests -------------------------------------------------

    #[test]
    fn classify_user_message_is_global() {
        let q = make_queue(4);
        let event = KernelEvent::UserMessage(test_inbound("hello"));
        assert_eq!(q.classify(&event), ShardTarget::Global);
    }

    #[test]
    fn classify_spawn_agent_is_global() {
        let q = make_queue(4);
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let event = KernelEvent::SpawnAgent {
            manifest:  crate::process::AgentManifest {
                name:               "test".to_string(),
                role:               None,
                description:        "test".to_string(),
                model:              None,
                system_prompt:      "test".to_string(),
                soul_prompt:        None,
                provider_hint:      None,
                max_iterations:     None,
                tools:              vec![],
                max_children:       None,
                max_context_tokens: None,
                priority:           Default::default(),
                metadata:           Default::default(),
                sandbox:            None,
            },
            input:     "hello".to_string(),
            principal: crate::process::principal::Principal::user("test"),
            parent_id: None,
            reply_tx:  tx,
        };
        assert_eq!(q.classify(&event), ShardTarget::Global);
    }

    #[test]
    fn classify_shutdown_is_global() {
        let q = make_queue(4);
        assert_eq!(q.classify(&KernelEvent::Shutdown), ShardTarget::Global);
    }

    #[test]
    fn classify_deliver_is_global() {
        let q = make_queue(4);
        let event = KernelEvent::Deliver(crate::io::types::OutboundEnvelope {
            id:          MessageId::new(),
            in_reply_to: MessageId::new(),
            user:        UserId("u1".to_string()),
            session_id:  SessionId::new(),
            routing:     crate::io::types::OutboundRouting::BroadcastAll,
            payload:     crate::io::types::OutboundPayload::Reply {
                content:     MessageContent::Text("reply".to_string()),
                attachments: vec![],
            },
            timestamp:   jiff::Timestamp::now(),
        });
        assert_eq!(q.classify(&event), ShardTarget::Global);
    }

    #[test]
    fn classify_send_signal_is_sharded() {
        let q = make_queue(4);
        let target = AgentId::new();
        let event = KernelEvent::SendSignal {
            target,
            signal: Signal::Interrupt,
        };
        let expected_shard = target.0.as_u128() as usize % 4;
        assert_eq!(q.classify(&event), ShardTarget::Shard(expected_shard));
    }

    #[test]
    fn classify_turn_completed_is_sharded() {
        let q = make_queue(4);
        let agent_id = AgentId::new();
        let event = KernelEvent::TurnCompleted {
            agent_id,
            session_id: SessionId::new(),
            result: Ok(crate::agent_turn::AgentTurnResult {
                text:       "done".to_string(),
                iterations: 1,
                tool_calls: 0,
                model:      "test".to_string(),
                trace:      crate::agent_turn::TurnTrace {
                    duration_ms:      0,
                    model:            "test".to_string(),
                    input_text:       None,
                    iterations:       vec![],
                    final_text_len:   4,
                    total_tool_calls: 0,
                    success:          true,
                    error:            None,
                },
            }),
            in_reply_to: MessageId::new(),
            user: UserId("u1".to_string()),
        };
        let expected_shard = agent_id.0.as_u128() as usize % 4;
        assert_eq!(q.classify(&event), ShardTarget::Shard(expected_shard));
    }

    #[test]
    fn classify_child_completed_is_sharded() {
        let q = make_queue(4);
        let parent_id = AgentId::new();
        let event = KernelEvent::ChildCompleted {
            parent_id,
            child_id: AgentId::new(),
            result: crate::process::AgentResult {
                output:     "done".to_string(),
                iterations: 1,
                tool_calls: 0,
            },
        };
        let expected_shard = parent_id.0.as_u128() as usize % 4;
        assert_eq!(q.classify(&event), ShardTarget::Shard(expected_shard));
    }

    // -- push routing tests -----------------------------------------------

    #[test]
    fn push_routes_global_to_global_queue() {
        let q = make_queue(4);
        q.push(KernelEvent::UserMessage(test_inbound("hello")))
            .unwrap();

        assert_eq!(q.global().pending_count(), 1);
        for i in 0..4 {
            assert_eq!(q.shard(i).pending_count(), 0);
        }
        assert_eq!(q.pending_count(), 1);
    }

    #[test]
    fn push_routes_signal_to_correct_shard() {
        let q = make_queue(4);
        let target = AgentId::new();
        let expected_shard = target.0.as_u128() as usize % 4;

        q.push(KernelEvent::SendSignal {
            target,
            signal: Signal::Interrupt,
        })
        .unwrap();

        assert_eq!(q.shard(expected_shard).pending_count(), 1);
        assert_eq!(q.global().pending_count(), 0);
        assert_eq!(q.pending_count(), 1);
    }

    #[test]
    fn pending_count_aggregates_across_shards() {
        let q = make_queue(4);

        // Push 2 global events
        q.push(KernelEvent::UserMessage(test_inbound("a"))).unwrap();
        q.push(KernelEvent::UserMessage(test_inbound("b"))).unwrap();

        // Push 1 sharded event
        q.push(KernelEvent::SendSignal {
            target: AgentId::new(),
            signal: Signal::Kill,
        })
        .unwrap();

        assert_eq!(q.pending_count(), 3);
    }

    #[test]
    fn drain_collects_from_all_queues() {
        let q = make_queue(4);

        // Push 1 global event
        q.push(KernelEvent::UserMessage(test_inbound("global")))
            .unwrap();

        // Push 1 sharded event
        q.push(KernelEvent::SendSignal {
            target: AgentId::new(),
            signal: Signal::Kill,
        })
        .unwrap();

        assert_eq!(q.pending_count(), 2);

        let events = q.drain(10);
        assert_eq!(events.len(), 2);
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn test_capacity_per_shard() {
        let q = ShardedEventQueue::new(ShardedEventQueueConfig {
            num_shards:      1,
            shard_capacity:  2,
            global_capacity: 100,
        });

        let target = AgentId::new();

        // Fill the single shard
        q.push(KernelEvent::SendSignal {
            target,
            signal: Signal::Interrupt,
        })
        .unwrap();
        q.push(KernelEvent::SendSignal {
            target,
            signal: Signal::Interrupt,
        })
        .unwrap();

        // Third should fail
        let result = q.push(KernelEvent::SendSignal {
            target,
            signal: Signal::Interrupt,
        });
        assert!(result.is_err());
    }

    // -- single-queue mode (num_shards=0) -----------------------------------

    #[test]
    fn single_mode_classify_routes_all_to_global() {
        let q = make_queue(0);
        // Agent-scoped event should still go to Global in single mode.
        let target = AgentId::new();
        let event = KernelEvent::SendSignal {
            target,
            signal: Signal::Interrupt,
        };
        assert_eq!(q.classify(&event), ShardTarget::Global);
    }

    #[test]
    fn single_mode_is_not_sharded() {
        let q = make_queue(0);
        assert!(!q.is_sharded());
    }

    #[test]
    fn single_mode_push_and_drain() {
        let q = make_queue(0);
        let target = AgentId::new();

        q.push(KernelEvent::UserMessage(test_inbound("hello")))
            .unwrap();
        q.push(KernelEvent::SendSignal {
            target,
            signal: Signal::Interrupt,
        })
        .unwrap();

        assert_eq!(q.pending_count(), 2);
        let events = q.drain(10);
        assert_eq!(events.len(), 2);
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn single_mode_config() {
        let config = ShardedEventQueueConfig::single();
        assert_eq!(config.num_shards, 0);
        assert_eq!(config.global_capacity, 4096);
    }

    #[test]
    fn test_debug_format() {
        let q = make_queue(4);
        let debug = format!("{:?}", q);
        assert!(debug.contains("ShardedEventQueue"));
        assert!(debug.contains("num_shards: 4"));
        assert!(debug.contains("total_pending: 0"));
    }
}
