//! Always-on per-session reply buffer for the Web channel.
//!
//! The buffer is a structural correctness fix, not a tunable feature: when a
//! long-running task completes while the user has closed their tab, the
//! WS broadcast has zero receivers and the event would otherwise be silently
//! dropped (issue #1804). Capacity, TTL, and sweeper interval are mechanism
//! parameters that live as `const` next to the code, **not** deployment
//! configuration — there is no YAML knob to disable buffering.

use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use parking_lot::Mutex;
use rara_kernel::session::SessionKey;
use tokio_util::sync::CancellationToken;

use crate::web::WebEvent;

/// Maximum number of "important" events retained per session before FIFO
/// eviction.
const REPLY_BUFFER_CAPACITY: usize = 64;

/// How long after the last write a session's buffer is kept before the
/// sweeper drops it.
const REPLY_BUFFER_TTL: Duration = Duration::from_mins(10);

/// How often the sweeper checks for expired sessions.
const REPLY_BUFFER_SWEEP_INTERVAL: Duration = Duration::from_mins(1);

/// One session's bounded ring of important events plus a last-write
/// timestamp used by the TTL sweeper.
struct SessionBuffer {
    events:     VecDeque<WebEvent>,
    last_write: Instant,
}

/// Per-session reply buffer registry. Shared across all WS / SSE
/// handlers and the `ChannelAdapter::send` path.
pub struct ReplyBuffer {
    sessions: DashMap<SessionKey, Arc<Mutex<SessionBuffer>>>,
}

impl ReplyBuffer {
    /// Construct a new empty buffer.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: DashMap::new(),
        })
    }

    /// Decide whether an event must survive a "no listeners" publish.
    ///
    /// Streaming chunks intentionally fall through to `false` — replaying
    /// a partial token stream after the fact has no useful UX.
    pub fn should_buffer(event: &WebEvent) -> bool {
        matches!(
            event,
            WebEvent::Message { .. }
                | WebEvent::Error { .. }
                | WebEvent::BackgroundTaskDone { .. }
                | WebEvent::Progress { .. }
        )
    }

    /// Append an event to the session's ring, evicting the oldest entry
    /// when capacity is exceeded.
    pub fn append(&self, session_key: &SessionKey, event: WebEvent) {
        let entry = self
            .sessions
            .entry(session_key.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(SessionBuffer {
                    events:     VecDeque::with_capacity(REPLY_BUFFER_CAPACITY),
                    last_write: Instant::now(),
                }))
            })
            .clone();
        let mut guard = entry.lock();
        if guard.events.len() == REPLY_BUFFER_CAPACITY {
            guard.events.pop_front();
        }
        guard.events.push_back(event);
        guard.last_write = Instant::now();
    }

    /// Snapshot of currently buffered events for `session_key`, in
    /// publish order (oldest first). Returns an empty vec if the
    /// session has no buffer.
    ///
    /// The buffer is **not** drained — see module-level docs for why.
    pub fn snapshot(&self, session_key: &SessionKey) -> Vec<WebEvent> {
        let Some(entry) = self.sessions.get(session_key).map(|e| e.clone()) else {
            return Vec::new();
        };
        entry.lock().events.iter().cloned().collect()
    }

    /// Number of currently tracked sessions — exposed for tests / metrics.
    #[doc(hidden)]
    pub fn session_count(&self) -> usize { self.sessions.len() }

    /// Spawn the TTL sweeper. Returns immediately; the sweeper runs in
    /// the background until `cancel` fires.
    pub fn spawn_sweeper(self: Arc<Self>, cancel: CancellationToken) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(REPLY_BUFFER_SWEEP_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = ticker.tick() => {
                        self.sweep_expired(REPLY_BUFFER_TTL);
                    }
                }
            }
        });
    }

    /// Remove sessions whose `last_write` is older than `ttl`. Pulled out
    /// for direct unit testing.
    #[doc(hidden)]
    pub fn sweep_expired(&self, ttl: Duration) {
        let now = Instant::now();
        let victims: Vec<SessionKey> = self
            .sessions
            .iter()
            .filter_map(|entry| {
                let guard = entry.value().lock();
                (now.duration_since(guard.last_write) > ttl).then(|| entry.key().clone())
            })
            .collect();
        for k in victims {
            self.sessions.remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rara_kernel::{io::BackgroundTaskStatus, session::SessionKey};

    use super::{REPLY_BUFFER_CAPACITY, ReplyBuffer, WebEvent};

    fn session() -> SessionKey { SessionKey::new() }

    #[test]
    fn streaming_events_are_not_buffered() {
        assert!(!ReplyBuffer::should_buffer(&WebEvent::TextDelta {
            text: "x".to_owned(),
        }));
        assert!(!ReplyBuffer::should_buffer(&WebEvent::ReasoningDelta {
            text: "x".to_owned(),
        }));
        assert!(!ReplyBuffer::should_buffer(&WebEvent::ToolCallStart {
            name:      "t".to_owned(),
            id:        "id".to_owned(),
            arguments: serde_json::json!({}),
        }));
    }

    #[test]
    fn important_events_are_buffered() {
        assert!(ReplyBuffer::should_buffer(&WebEvent::Message {
            content: "hi".to_owned(),
        }));
        assert!(ReplyBuffer::should_buffer(&WebEvent::Error {
            message: "bad".to_owned(),
        }));
        assert!(ReplyBuffer::should_buffer(&WebEvent::BackgroundTaskDone {
            task_id: "t".to_owned(),
            status:  BackgroundTaskStatus::Completed,
        }));
        assert!(ReplyBuffer::should_buffer(&WebEvent::Progress {
            stage: "done".to_owned(),
        }));
    }

    #[test]
    fn append_then_snapshot_returns_events_in_order() {
        let buf = ReplyBuffer::new();
        let s = session();
        buf.append(
            &s,
            WebEvent::Message {
                content: "first".to_owned(),
            },
        );
        buf.append(
            &s,
            WebEvent::Message {
                content: "second".to_owned(),
            },
        );

        let snap = buf.snapshot(&s);
        assert_eq!(snap.len(), 2);
        match (&snap[0], &snap[1]) {
            (WebEvent::Message { content: a }, WebEvent::Message { content: b }) => {
                assert_eq!(a, "first");
                assert_eq!(b, "second");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn snapshot_does_not_drain() {
        let buf = ReplyBuffer::new();
        let s = session();
        buf.append(
            &s,
            WebEvent::Message {
                content: "x".to_owned(),
            },
        );
        assert_eq!(buf.snapshot(&s).len(), 1);
        assert_eq!(buf.snapshot(&s).len(), 1);
    }

    #[test]
    fn capacity_overflow_evicts_oldest() {
        let buf = ReplyBuffer::new();
        let s = session();
        for i in 0..=REPLY_BUFFER_CAPACITY {
            buf.append(
                &s,
                WebEvent::Message {
                    content: format!("m{i}"),
                },
            );
        }
        let snap = buf.snapshot(&s);
        assert_eq!(snap.len(), REPLY_BUFFER_CAPACITY);
        // Oldest ("m0") should have been evicted; first remaining is "m1".
        match &snap[0] {
            WebEvent::Message { content } => assert_eq!(content, "m1"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn sweep_expired_drops_idle_sessions() {
        let buf = ReplyBuffer::new();
        let s = session();
        buf.append(
            &s,
            WebEvent::Message {
                content: "x".to_owned(),
            },
        );
        assert_eq!(buf.session_count(), 1);

        buf.sweep_expired(Duration::from_nanos(0));
        assert_eq!(buf.session_count(), 0);
    }
}
