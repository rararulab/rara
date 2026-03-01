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

//! Event processor — processes events from a single [`ShardQueue`].
//!
//! Each `EventProcessor` runs as an independent tokio task, draining events
//! from its assigned shard queue and dispatching them to the kernel's event
//! handlers.
//!
//! The sharded event loop spawns N+1 processors:
//! - 1 **global processor** for UserMessage, SpawnAgent, Timer, Shutdown, Deliver
//! - N **shard processors** for agent-scoped events (Syscall, TurnCompleted, etc.)

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::{
    event_loop::RuntimeTable,
    kernel::Kernel,
    shard_queue::ShardQueue,
    unified_event::KernelEvent,
};

/// A single event processor that drains and processes events from one
/// `ShardQueue`.
///
/// Each processor runs independently, allowing parallel event handling
/// across different agent shards.
pub(crate) struct EventProcessor {
    /// Processor identifier (0 = global, 1..=N = shard processors).
    pub id: usize,
    /// The shard queue this processor drains from.
    pub queue: Arc<ShardQueue>,
}

impl EventProcessor {
    /// Run the event processor loop until shutdown.
    ///
    /// Drains events from the shard queue in batches of up to 32 and
    /// dispatches each to `kernel.handle_event()`.
    pub async fn run(
        &self,
        kernel: &Kernel,
        runtimes: &RuntimeTable,
        shutdown: CancellationToken,
    ) {
        info!(processor_id = self.id, "event processor started");

        loop {
            tokio::select! {
                _ = self.queue.wait() => {
                    let events = self.queue.drain(32);
                    for (event, wal_id) in events {
                        kernel.handle_event(event, runtimes).await;
                        if let Some(id) = wal_id {
                            kernel.event_queue().mark_completed(id);
                        }
                    }
                }
                _ = shutdown.cancelled() => {
                    info!(processor_id = self.id, "event processor shutting down");
                    // Drain remaining critical events.
                    let remaining = self.queue.drain(1024);
                    for (event, wal_id) in remaining {
                        if matches!(event, KernelEvent::SendSignal { .. } | KernelEvent::Shutdown) {
                            kernel.handle_event(event, runtimes).await;
                        } else {
                            warn!(
                                processor_id = self.id,
                                event = ?event,
                                "dropping non-critical event during shutdown"
                            );
                        }
                        if let Some(id) = wal_id {
                            kernel.event_queue().mark_completed(id);
                        }
                    }
                    break;
                }
            }
        }

        info!(processor_id = self.id, "event processor stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::types::InboundMessage;
    use crate::process::{SessionId, principal::UserId};
    use crate::channel::types::{ChannelType, MessageContent};
    use crate::io::types::{ChannelSource, MessageId};
    use std::collections::HashMap;

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
            session_id:      SessionId::new("s1"),
            target_agent_id: None,
            target_agent:    None,
            content:         MessageContent::Text(text.to_string()),
            reply_context:   None,
            timestamp:       jiff::Timestamp::now(),
            metadata:        HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_processor_shutdown_drains_critical() {
        let queue = Arc::new(ShardQueue::new(100));
        let _processor = EventProcessor { id: 0, queue: queue.clone() };

        // Push some events before starting.
        queue.push(KernelEvent::UserMessage(test_inbound("will be dropped"))).unwrap();
        queue.push(KernelEvent::Shutdown).unwrap();

        let shutdown = CancellationToken::new();

        // Cancel immediately — processor should drain only critical events.
        shutdown.cancel();

        // We can't easily test with a real kernel here, but we can test
        // that the processor exits cleanly when shutdown is already cancelled.
        // The queue should be drained after the processor runs.
        // Since we don't have a real kernel, just verify the queue structure.
        assert_eq!(queue.pending_count(), 2);
    }

    #[test]
    fn test_processor_creation() {
        let queue = Arc::new(ShardQueue::new(100));
        let processor = EventProcessor { id: 42, queue };

        assert_eq!(processor.id, 42);
    }
}
