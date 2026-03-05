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
use crate::{event::KernelEventCommonFields, io::IOError};

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
    fn push(&self, event: KernelEventEnvelope) -> Result<(), IOError> {
        let observed = ObservableKernelEvent::from_event(&event);
        self.inner.push(event)?;
        if let Some(observed) = observed {
            let _ = self.sender.send(observed);
        }
        Ok(())
    }

    fn try_push(&self, event: KernelEventEnvelope) -> Result<(), IOError> {
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

impl ObservableKernelEvent {
    fn from_event(event: &KernelEventEnvelope) -> Option<Self> {
        let payload = serde_json::to_value(&event.kind).ok()?;
        Some(Self {
            common: event.common_fields(),
            event:  payload,
        })
    }
}
