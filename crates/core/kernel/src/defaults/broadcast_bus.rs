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
        // TODO: apply filter in a wrapper stream
        self.sender.subscribe()
    }
}
