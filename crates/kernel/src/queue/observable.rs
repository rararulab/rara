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

//! Observable wrapper for [`EventQueue`] implementations.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::broadcast;

use super::{EventQueue, EventQueueRef, KernelEventEnvelope};
use crate::{event::KernelEventCommonFields, io::types::BusError};

/// Observable payload derived from the canonical [`KernelEvent`] definition.
#[derive(Debug, Clone, Serialize)]
pub struct ObservableKernelEvent {
    pub common: KernelEventCommonFields,
    pub event:  serde_json::Value,
}

/// Shared reference to an observable event queue wrapper.
pub type ObservableEventQueueRef = Arc<ObservableEventQueue>;

/// Wrapper that mirrors queue operations and broadcasts successful enqueues.
pub struct ObservableEventQueue {
    inner:  EventQueueRef,
    sender: broadcast::Sender<ObservableKernelEvent>,
}

impl ObservableEventQueue {
    /// Wrap an existing event queue with observable fanout.
    pub fn new(inner: EventQueueRef, capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { inner, sender }
    }
}

#[async_trait]
impl EventQueue for ObservableEventQueue {
    fn push(&self, event: KernelEventEnvelope) -> Result<(), BusError> {
        let observed = ObservableKernelEvent::from_event(&event);
        self.inner.push(event)?;
        if let Some(observed) = observed {
            let _ = self.sender.send(observed);
        }
        Ok(())
    }

    fn try_push(&self, event: KernelEventEnvelope) -> Result<(), BusError> {
        let observed = ObservableKernelEvent::from_event(&event);
        self.inner.try_push(event)?;
        if let Some(observed) = observed {
            let _ = self.sender.send(observed);
        }
        Ok(())
    }

    fn drain(&self, max: usize) -> Vec<KernelEventEnvelope> { self.inner.drain(max) }

    async fn wait(&self) { self.inner.wait().await; }

    fn pending_count(&self) -> usize { self.inner.pending_count() }

    fn is_sharded(&self) -> bool { self.inner.is_sharded() }

    fn subscribe(&self) -> Option<broadcast::Receiver<ObservableKernelEvent>> {
        Some(self.sender.subscribe())
    }
}

impl std::fmt::Debug for ObservableEventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservableEventQueue")
            .field("pending", &self.pending_count())
            .field("is_sharded", &self.is_sharded())
            .finish()
    }
}

impl ObservableKernelEvent {
    fn from_event(event: &KernelEventEnvelope) -> Option<Self> {
        let payload = serde_json::to_value(&event.kind).ok()?;
        Some(Self {
            common: event.common_fields(),
            event:  payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ObservableEventQueue, ObservableKernelEvent};
    use crate::{
        event::KernelEventEnvelope,
        process::{AgentId, Signal},
        queue::{EventQueue, InMemoryEventQueue},
    };

    #[test]
    fn wrapper_push_fans_out_to_subscribers() {
        let inner: Arc<dyn EventQueue> = Arc::new(InMemoryEventQueue::new(8));
        let queue = ObservableEventQueue::new(inner, 8);
        let mut rx = queue.subscribe().unwrap();

        queue.push(KernelEventEnvelope::shutdown()).unwrap();

        let received = rx.try_recv().unwrap();
        assert_eq!(received.common.event_type, "shutdown");
        assert_eq!(received.event, serde_json::json!("Shutdown"));
    }

    #[test]
    fn subscribed_event_exposes_common_fields() {
        let agent_id = AgentId::new();
        let event = KernelEventEnvelope::send_signal(agent_id, Signal::Pause);

        let fields = event.common_fields();

        assert_eq!(fields.event_type, "send_signal");
        assert_eq!(fields.priority, "critical");
        assert_eq!(fields.agent_id, Some(agent_id.to_string()));
        assert!(fields.summary.contains("Pause"));
    }

    #[test]
    fn observed_event_contains_payload_and_common_fields() {
        let observed = ObservableKernelEvent::from_event(&KernelEventEnvelope::shutdown()).unwrap();

        assert_eq!(observed.common.event_type, "shutdown");
        assert_eq!(observed.event, serde_json::json!("Shutdown"));
    }
}
