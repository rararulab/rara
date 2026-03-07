use std::sync::Arc;
use std::time::Duration;

use crossbeam_queue::SegQueue;
use tokio::sync::Notify;

use crate::event::SymphonyEvent;

/// A lock-free, async-aware event queue for the symphony event loop.
#[derive(Debug, Clone)]
pub struct EventQueue {
    queue: Arc<SegQueue<SymphonyEvent>>,
    notify: Arc<Notify>,
}

impl EventQueue {
    /// Create a new empty event queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue: Arc::new(SegQueue::new()),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Push an event onto the queue and notify any waiting consumers.
    pub fn push(&self, event: SymphonyEvent) {
        self.queue.push(event);
        self.notify.notify_one();
    }

    /// Pop an event from the queue, waiting asynchronously if empty.
    pub async fn pop(&self) -> SymphonyEvent {
        loop {
            if let Some(event) = self.queue.pop() {
                return event;
            }
            self.notify.notified().await;
        }
    }

    /// Schedule an event to be pushed after a delay.
    pub fn schedule_after(&self, delay: Duration, event: SymphonyEvent) {
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            this.push(event);
        });
    }

    /// Return the number of events currently in the queue.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Check whether the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn push_and_pop() {
        let q = EventQueue::new();
        q.push(SymphonyEvent::Shutdown);
        q.push(SymphonyEvent::AgentStalled { issue_id: "test".into() });

        assert_eq!(q.len(), 2);

        let first = q.pop().await;
        assert!(matches!(first, SymphonyEvent::Shutdown));

        let second = q.pop().await;
        assert!(matches!(second, SymphonyEvent::AgentStalled { .. }));

        assert!(q.is_empty());
    }

    #[tokio::test]
    async fn pop_waits_for_push() {
        let q = EventQueue::new();
        let q2 = q.clone();

        let handle = tokio::spawn(async move { q2.pop().await });

        // Give the spawned task a moment to start waiting.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(q.is_empty());

        q.push(SymphonyEvent::Shutdown);

        let event = handle.await.unwrap();
        assert!(matches!(event, SymphonyEvent::Shutdown));
    }

    #[tokio::test]
    async fn schedule_after_delivers() {
        let q = EventQueue::new();
        q.schedule_after(Duration::from_millis(50), SymphonyEvent::Shutdown);

        // Should be empty immediately.
        assert!(q.is_empty());

        // Wait for the delayed event to arrive.
        let event = tokio::time::timeout(Duration::from_secs(2), q.pop())
            .await
            .expect("timed out waiting for scheduled event");
        assert!(matches!(event, SymphonyEvent::Shutdown));
    }
}
