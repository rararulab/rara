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

//! Per-session ring buffer for "important" `WebEvent`s so that task-completion
//! replies survive periods where no WS / SSE listener is attached.
//!
//! # Why this exists
//!
//! `WebAdapter` publishes outbound replies through a `tokio::broadcast`
//! channel. When a long-running background task finishes while the user has
//! closed their browser tab, `broadcast::Sender::send` returns `Err` because
//! `receiver_count == 0`, and the `Reply` envelope is silently dropped. When
//! the user reconnects, the kernel has no record of "you owe this socket a
//! reply" — task output is lost forever (see issue #1804).
//!
//! # What is buffered
//!
//! Only events whose loss is user-visible:
//!
//! - [`WebEvent::Message`]      — final agent reply (the original #1804 case)
//! - [`WebEvent::Error`]        — surfaced error notifications
//! - [`WebEvent::BackgroundTaskDone`] — terminal status of a bg task
//! - [`WebEvent::Progress`]     — terminal stage signals
//!
//! Streaming deltas (`TextDelta`, `ReasoningDelta`, `ToolCall*`, …) are
//! intentionally **not** buffered: replaying a partial token stream after the
//! fact has no useful semantics for the UI.
//!
//! # Replay semantics & trade-off
//!
//! On connect the WS / SSE handler drains the buffer into the new socket
//! before forwarding live events. The buffer is **not** removed on drain —
//! a session may have multiple concurrent tabs, and a brand-new tab opening
//! mid-turn should still see the catch-up history. The cost is that an
//! already-connected tab which read an event live will see it *again* if it
//! reconnects (e.g. WS drop + retry inside the TTL window). Callers that
//! cannot tolerate duplicate `WebEvent::Message` rows must dedupe by
//! payload — there is no per-event sequence number on the wire today.
//!
//! Bounded capacity (oldest event is dropped on overflow) and a TTL sweep
//! task keep memory bounded; both are configured from YAML — no defaults
//! are hard-coded in Rust.

use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use rara_kernel::session::SessionKey;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, watch};

use crate::web::WebEvent;

/// Configuration for the per-session reply buffer.
///
/// Both fields are sourced from the YAML config file
/// (`web.reply_buffer.*`) — see `config.example.yaml`.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct ReplyBufferConfig {
    /// Maximum number of "important" events retained per session.
    /// On overflow, the oldest event is evicted (FIFO).
    pub capacity:       usize,
    /// How long after the last write a session's buffer is kept
    /// before the sweeper drops it.
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub ttl:            Duration,
    /// Sweeper tick interval. The sweeper runs every `sweep_interval` and
    /// removes any session whose buffer hasn't been written to within
    /// the last `ttl`.
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub sweep_interval: Duration,
}

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
    config:   ReplyBufferConfig,
}

impl ReplyBuffer {
    /// Construct a new empty buffer with the given configuration.
    pub fn new(config: ReplyBufferConfig) -> Arc<Self> {
        Arc::new(Self {
            sessions: DashMap::new(),
            config,
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
    pub async fn append(&self, session_key: &SessionKey, event: WebEvent) {
        let entry = self
            .sessions
            .entry(session_key.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(SessionBuffer {
                    events:     VecDeque::with_capacity(self.config.capacity.min(64)),
                    last_write: Instant::now(),
                }))
            })
            .clone();
        let mut guard = entry.lock().await;
        if guard.events.len() == self.config.capacity {
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
    pub async fn snapshot(&self, session_key: &SessionKey) -> Vec<WebEvent> {
        let Some(entry) = self.sessions.get(session_key).map(|e| e.clone()) else {
            return Vec::new();
        };
        entry.lock().await.events.iter().cloned().collect()
    }

    /// Number of buffered events for `session_key` — exposed for tests
    /// and metrics.
    #[doc(hidden)]
    pub async fn len(&self, session_key: &SessionKey) -> usize {
        match self.sessions.get(session_key).map(|e| e.clone()) {
            Some(entry) => entry.lock().await.events.len(),
            None => 0,
        }
    }

    /// Number of currently tracked sessions — exposed for tests / metrics.
    #[doc(hidden)]
    pub fn session_count(&self) -> usize { self.sessions.len() }

    /// Spawn the TTL sweeper. Returns immediately; the sweeper runs in
    /// the background until `shutdown_rx` flips to `true`.
    pub fn spawn_sweeper(self: Arc<Self>, mut shutdown_rx: watch::Receiver<bool>) {
        let interval = self.config.sweep_interval;
        let ttl = self.config.ttl;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => return,
                    _ = ticker.tick() => {
                        self.sweep_expired(ttl).await;
                    }
                }
            }
        });
    }

    /// Remove sessions whose `last_write` is older than `ttl`. Pulled out
    /// for direct unit testing.
    #[doc(hidden)]
    pub async fn sweep_expired(&self, ttl: Duration) {
        let now = Instant::now();
        let mut victims: Vec<SessionKey> = Vec::new();
        for entry in &self.sessions {
            let buf = entry.value().clone();
            let guard = buf.lock().await;
            if now.duration_since(guard.last_write) > ttl {
                victims.push(entry.key().clone());
            }
        }
        for k in victims {
            self.sessions.remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rara_kernel::{io::BackgroundTaskStatus, session::SessionKey};

    use super::{ReplyBuffer, ReplyBufferConfig, WebEvent};

    fn config(capacity: usize) -> ReplyBufferConfig {
        ReplyBufferConfig::builder()
            .capacity(capacity)
            .ttl(Duration::from_mins(1))
            .sweep_interval(Duration::from_secs(10))
            .build()
    }

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

    #[tokio::test]
    async fn append_then_snapshot_returns_events_in_order() {
        let buf = ReplyBuffer::new(config(8));
        let s = session();
        buf.append(
            &s,
            WebEvent::Message {
                content: "first".to_owned(),
            },
        )
        .await;
        buf.append(
            &s,
            WebEvent::Message {
                content: "second".to_owned(),
            },
        )
        .await;

        let snap = buf.snapshot(&s).await;
        assert_eq!(snap.len(), 2);
        match (&snap[0], &snap[1]) {
            (WebEvent::Message { content: a }, WebEvent::Message { content: b }) => {
                assert_eq!(a, "first");
                assert_eq!(b, "second");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn snapshot_does_not_drain() {
        let buf = ReplyBuffer::new(config(8));
        let s = session();
        buf.append(
            &s,
            WebEvent::Message {
                content: "x".to_owned(),
            },
        )
        .await;
        assert_eq!(buf.snapshot(&s).await.len(), 1);
        assert_eq!(buf.snapshot(&s).await.len(), 1);
    }

    #[tokio::test]
    async fn capacity_overflow_evicts_oldest() {
        let buf = ReplyBuffer::new(config(2));
        let s = session();
        for i in 0..3 {
            buf.append(
                &s,
                WebEvent::Message {
                    content: format!("m{i}"),
                },
            )
            .await;
        }
        let snap = buf.snapshot(&s).await;
        assert_eq!(snap.len(), 2);
        match (&snap[0], &snap[1]) {
            (WebEvent::Message { content: a }, WebEvent::Message { content: b }) => {
                assert_eq!(a, "m1");
                assert_eq!(b, "m2");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sweep_expired_drops_idle_sessions() {
        let buf = ReplyBuffer::new(config(4));
        let s = session();
        buf.append(
            &s,
            WebEvent::Message {
                content: "x".to_owned(),
            },
        )
        .await;
        assert_eq!(buf.session_count(), 1);

        // ttl=0 makes every entry immediately eligible.
        buf.sweep_expired(Duration::from_nanos(0)).await;
        assert_eq!(buf.session_count(), 0);
    }
}
