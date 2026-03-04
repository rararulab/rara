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

//! Multi-processor subsystem for kernel event dispatch.
//!
//! Owns the event processor workers that drain shard queues in parallel
//! when the kernel runs in sharded event-loop mode.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::{Instrument, info, info_span, warn};

use super::RuntimeTable;
use crate::{
    event::{KernelEvent, KernelEventEnvelope},
    kernel::Kernel,
    queue::shard::ShardQueue,
};

/// A single event processor that drains and processes events from one
/// `ShardQueue`.
///
/// Each processor runs independently, allowing parallel event handling
/// across different agent shards.
pub(crate) struct EventProcessor {
    /// Processor identifier (0 = global, 1..=N = shard processors).
    pub id:    usize,
    /// The shard queue this processor drains from.
    pub queue: Arc<ShardQueue>,
}

impl EventProcessor {
    /// Run the event processor loop until shutdown.
    ///
    /// Drains events from the shard queue in batches of up to 32 and
    /// dispatches each to `kernel.handle_event()`.
    pub async fn run(&self, kernel: &Kernel, runtimes: &RuntimeTable, shutdown: CancellationToken) {
        info!(processor_id = self.id, "event processor started");

        loop {
            tokio::select! {
                _ = self.queue.wait() => {
                    // Inner loop: keep draining while events are available
                    // to avoid re-entering select! unnecessarily.
                    loop {
                        let events = self.queue.drain(32);
                        if events.is_empty() { break; }
                        for event in events {
                            let event_type: &'static str = (&event).into();
                            let span = info_span!(
                                "handle_event",
                                processor_id = self.id,
                                event_type,
                            );
                            kernel.handle_event(event, runtimes)
                                .instrument(span)
                                .await;
                        }
                    }
                }
                _ = shutdown.cancelled() => {
                    info!(processor_id = self.id, "event processor shutting down");
                    let remaining = self.queue.drain(1024);
                    for event in remaining {
                        if matches!(event.kind, KernelEvent::SendSignal { .. } | KernelEvent::Shutdown) {
                            kernel.handle_event(event, runtimes).await;
                        } else {
                            warn!(
                                processor_id = self.id,
                                event = ?event,
                                "dropping non-critical event during shutdown"
                            );
                        }
                    }
                    break;
                }
            }
        }

        info!(processor_id = self.id, "event processor stopped");
    }
}
