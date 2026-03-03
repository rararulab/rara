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

//! Unified event loop — parallel multi-processor loop that processes all
//! [`KernelEvent`](crate::event::KernelEvent) variants.
//!
//! Always backed by a
//! [`ShardedEventQueue`](crate::queue::ShardedEventQueue). When
//! `num_shards == 0` (single-queue mode), only a global
//! [`EventProcessor`](processor::EventProcessor) is spawned. When
//! `num_shards > 0`, N additional shard processors run in parallel for
//! agent-scoped events. The kernel directly manages process state
//! (conversation, turn cancellation, pause buffer) instead of delegating
//! to per-process tokio tasks.

mod lifecycle;
mod message;
pub(crate) mod processor;
pub(crate) mod runtime;
mod syscall;
mod turn;

use std::sync::Arc;

pub(crate) use runtime::RuntimeTable;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{event::KernelEvent, kernel::Kernel};

impl Kernel {
    /// Agent name for admin/root users.
    const ADMIN_AGENT_NAME: &'static str = "rara";
    /// Agent name for regular users.
    const USER_AGENT_NAME: &'static str = "nana";

    /// Run the unified event loop, spawning 1 global + N shard
    /// [`EventProcessor`] tasks.
    ///
    /// When `num_shards == 0` (single-queue mode), only the global processor
    /// is spawned — functionally identical to the former single-queue path
    /// but using the same code path for both modes.
    ///
    /// Called from [`start()`](Kernel::start) which already wraps Kernel in
    /// Arc.
    pub(crate) async fn run_event_loop_arc(kernel: Arc<Kernel>, shutdown: CancellationToken) {
        use crate::event_loop::processor::EventProcessor;

        let runtimes: Arc<RuntimeTable> = Arc::new(dashmap::DashMap::new());
        let sq = kernel.sharded_queue().clone();
        let num_shards = sq.num_shards();

        info!(
            num_shards = num_shards,
            total_processors = num_shards + 1,
            "kernel event loop started"
        );

        let mut handles = Vec::with_capacity(num_shards + 1);

        // Global processor (id=0) — always present.
        {
            let proc = EventProcessor {
                id:    0,
                queue: Arc::clone(sq.global()),
            };
            let k = Arc::clone(&kernel);
            let rt = Arc::clone(&runtimes);
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                proc.run(&k, &rt, sd).await;
            }));
        }

        // Shard processors (id=1..=N) — only when sharding is enabled.
        for i in 0..num_shards {
            let proc = EventProcessor {
                id:    i + 1,
                queue: Arc::clone(sq.shard(i)),
            };
            let k = Arc::clone(&kernel);
            let rt = Arc::clone(&runtimes);
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                proc.run(&k, &rt, sd).await;
            }));
        }

        // Wait for all processors to finish.
        for handle in handles {
            if let Err(e) = handle.await {
                error!("event processor panicked: {e}");
            }
        }

        info!("kernel event loop stopped");
    }

    /// Dispatch a single event to its handler.
    pub(crate) async fn handle_event(&self, event: KernelEvent, runtimes: &RuntimeTable) {
        let event_type: &'static str = (&event).into();
        crate::metrics::EVENT_PROCESSED
            .with_label_values(&[event_type])
            .inc();

        match event {
            KernelEvent::UserMessage(msg) => {
                self.handle_user_message(msg, runtimes).await;
            }
            KernelEvent::SpawnAgent {
                manifest,
                input,
                principal,
                parent_id,
                reply_tx,
            } => {
                // SpawnAgent from ProcessHandle::spawn() — subagent, no
                // channel binding.
                let result = self
                    .handle_spawn_agent(manifest, input, principal, None, parent_id, runtimes)
                    .await;
                let _ = reply_tx.send(result);
            }
            KernelEvent::SendSignal { target, signal } => {
                self.handle_signal(target, signal, runtimes).await;
            }
            KernelEvent::TurnCompleted {
                agent_id,
                session_id,
                result,
                in_reply_to,
                user,
            } => {
                self.handle_turn_completed(
                    agent_id,
                    session_id,
                    result,
                    in_reply_to,
                    user,
                    runtimes,
                )
                .await;
            }
            KernelEvent::ChildCompleted {
                parent_id,
                child_id,
                result,
            } => {
                self.handle_child_completed(parent_id, child_id, result, runtimes)
                    .await;
            }
            KernelEvent::Deliver(envelope) => {
                self.spawn_deliver(envelope);
            }
            KernelEvent::Syscall(syscall) => {
                self.handle_syscall(syscall, runtimes).await;
            }
            KernelEvent::Shutdown => {
                info!("shutdown event received");
            }
        }
    }
}
