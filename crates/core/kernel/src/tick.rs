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

//! TickLoop — the kernel's main event loop that drains the [`InboundBus`]
//! and dispatches messages through the [`SessionScheduler`] to the
//! [`AgentExecutor`].
//!
//! The tick loop is woken by the bus's `wait_for_messages()` mechanism
//! (no polling fallback). On each tick, it drains up to a configurable
//! batch size and dispatches each message through the scheduler:
//!
//! - `Ready` messages are spawned into the executor immediately.
//! - `Queued` messages wait until their session's current execution completes.
//! - `Rejected` messages are dropped with a warning (session queue full).

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::executor::AgentExecutor;
use crate::io::bus::InboundBus;
use crate::io::types::InboundMessage;
use crate::scheduler::{ScheduleResult, SessionScheduler};

// ---------------------------------------------------------------------------
// TickLoop
// ---------------------------------------------------------------------------

/// The kernel's main event loop.
///
/// Drains the [`InboundBus`] in batches, schedules messages through the
/// [`SessionScheduler`], and dispatches ready messages to the
/// [`AgentExecutor`] as concurrent tasks.
///
/// Stops gracefully when the [`CancellationToken`] is cancelled.
pub struct TickLoop {
    /// The inbound message bus to drain.
    inbound_bus: Arc<dyn InboundBus>,
    /// Per-session serial execution scheduler.
    session_scheduler: Arc<SessionScheduler>,
    /// The executor that processes individual messages.
    executor: Arc<AgentExecutor>,
    /// Maximum number of messages to drain per tick.
    batch_size: usize,
}

impl TickLoop {
    /// Create a new tick loop.
    pub fn new(
        inbound_bus: Arc<dyn InboundBus>,
        session_scheduler: Arc<SessionScheduler>,
        executor: Arc<AgentExecutor>,
    ) -> Self {
        Self {
            inbound_bus,
            session_scheduler,
            executor,
            batch_size: 32,
        }
    }

    /// Create a new tick loop with a custom batch size.
    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Main loop -- woken by the InboundBus, no polling.
    ///
    /// Runs until the shutdown token is cancelled.
    pub async fn run(&self, shutdown: CancellationToken) {
        info!("tick loop started");
        loop {
            tokio::select! {
                _ = self.inbound_bus.wait_for_messages() => {
                    self.tick().await;
                }
                _ = shutdown.cancelled() => {
                    info!("tick loop shutting down");
                    break;
                }
            }
        }
    }

    /// Process one tick: drain messages and dispatch.
    pub async fn tick(&self) {
        let messages = self.inbound_bus.drain(self.batch_size).await;
        for msg in messages {
            self.dispatch(msg);
        }
    }

    /// Dispatch a single message through the session scheduler.
    fn dispatch(&self, msg: InboundMessage) {
        match self.session_scheduler.schedule(msg) {
            ScheduleResult::Ready(ready_msg) => {
                let executor = self.executor.clone();
                tokio::spawn(async move {
                    executor.run(*ready_msg).await;
                });
            }
            ScheduleResult::Queued => {
                // Message is waiting in the session queue; it will be
                // re-published to the InboundBus when the current execution
                // completes (via AgentExecutor::release_session).
            }
            ScheduleResult::Rejected => {
                warn!("message rejected: session queue full");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::Semaphore;

    use super::*;
    use crate::channel::types::{ChannelType, MessageContent};
    use crate::defaults::noop::{NoopOutboxStore, NoopSessionRepository};
    use crate::executor::AgentExecutor;
    use crate::io::memory_bus::{InMemoryInboundBus, InMemoryOutboundBus};
    use crate::io::stream::StreamHub;
    use crate::session_manager::SessionManager;
    use crate::io::types::{ChannelSource, MessageId};
    use crate::process::principal::UserId;
    use crate::process::{ProcessTable, SessionId};
    use crate::provider::EnvLlmProviderLoader;
    use crate::provider::LlmProviderLoaderRef;
    use crate::tool::ToolRegistry;

    /// Helper: build a test InboundMessage.
    fn test_inbound(session: &str, text: &str) -> InboundMessage {
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

    /// Helper: create test components and a TickLoop.
    fn make_test_tick_loop(
        inbound_bus: Arc<InMemoryInboundBus>,
    ) -> (TickLoop, Arc<SessionScheduler>) {
        let scheduler = Arc::new(SessionScheduler::new(5));
        let executor = Arc::new(AgentExecutor::new(
            ProcessTable::new(),
            Arc::new(Semaphore::new(16)),
            scheduler.clone(),
            inbound_bus.clone() as Arc<dyn InboundBus>,
            Arc::new(InMemoryOutboundBus::new(64)),
            Arc::new(NoopOutboxStore),
            Arc::new(StreamHub::new(64)),
            Arc::new(SessionManager::new(
                Arc::new(NoopSessionRepository),
            )),
            Arc::new(EnvLlmProviderLoader::default()) as LlmProviderLoaderRef,
            Arc::new(ToolRegistry::new()),
        ));

        let tick_loop = TickLoop::new(
            inbound_bus as Arc<dyn InboundBus>,
            scheduler.clone(),
            executor,
        );

        (tick_loop, scheduler)
    }

    #[tokio::test]
    async fn test_tick_loop_drain_and_dispatch() {
        let inbound_bus = Arc::new(InMemoryInboundBus::new(100));

        // Publish 3 messages for different sessions
        inbound_bus
            .publish(test_inbound("s1", "hello s1"))
            .await
            .unwrap();
        inbound_bus
            .publish(test_inbound("s2", "hello s2"))
            .await
            .unwrap();
        inbound_bus
            .publish(test_inbound("s3", "hello s3"))
            .await
            .unwrap();

        let (tick_loop, scheduler) = make_test_tick_loop(inbound_bus.clone());

        // Run one tick
        tick_loop.tick().await;

        // All 3 messages should have been drained
        assert_eq!(inbound_bus.pending_count(), 0);

        // The scheduler should have processed them. Since all are different
        // sessions, all become Ready and are dispatched. The slots should be
        // in running state (they won't complete since there's no real LLM).
        // We can verify this by trying to schedule another message for s1
        // which should be Queued (since s1 is still "running").
        let result = scheduler.schedule(test_inbound("s1", "second for s1"));
        assert!(matches!(result, ScheduleResult::Queued));
    }

    #[tokio::test]
    async fn test_tick_loop_shutdown() {
        let inbound_bus = Arc::new(InMemoryInboundBus::new(100));
        let (tick_loop, _scheduler) = make_test_tick_loop(inbound_bus);

        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();

        let handle = tokio::spawn(async move {
            tick_loop.run(shutdown_clone).await;
        });

        // Cancel after a short delay
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        shutdown.cancel();

        // The loop should exit within a reasonable time
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            handle,
        )
        .await;

        assert!(result.is_ok(), "tick loop should have exited on shutdown");
        assert!(
            result.unwrap().is_ok(),
            "tick loop task should complete successfully"
        );
    }

    #[tokio::test]
    async fn test_tick_loop_same_session_queued() {
        let inbound_bus = Arc::new(InMemoryInboundBus::new(100));

        // Publish 2 messages for the same session
        inbound_bus
            .publish(test_inbound("s1", "first"))
            .await
            .unwrap();
        inbound_bus
            .publish(test_inbound("s1", "second"))
            .await
            .unwrap();

        let (tick_loop, _scheduler) = make_test_tick_loop(inbound_bus.clone());

        // Run one tick
        tick_loop.tick().await;

        // Both messages should have been drained from the bus
        assert_eq!(inbound_bus.pending_count(), 0);

        // The first message for s1 is Ready (dispatched to executor).
        // The second message for s1 is Queued (waiting for the first to complete).
        // We can't easily inspect the scheduler's internal state, but the
        // test passes if no panics occur and the bus is drained.
    }
}
