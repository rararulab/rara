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

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::event::{EventBus, EventFilter, EventStream, KernelEvent};

/// Event bus backed by `tokio::sync::broadcast`.
pub struct BroadcastEventBus {
    sender: broadcast::Sender<KernelEvent>,
}

impl BroadcastEventBus {
    /// Create a new broadcast event bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }
}

impl Default for BroadcastEventBus {
    fn default() -> Self { Self::new(256) }
}

#[async_trait]
impl EventBus for BroadcastEventBus {
    async fn publish(&self, event: KernelEvent) {
        // Ignore send errors (no active subscribers).
        let _ = self.sender.send(event);
    }

    async fn subscribe(&self, _filter: EventFilter) -> EventStream {
        self.sender.subscribe()
    }
}
