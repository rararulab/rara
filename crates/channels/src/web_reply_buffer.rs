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

//! Always-on per-session reply buffer for the Web channel.
//!
//! When a long-running task completes while the user has closed their tab,
//! the WS broadcast has zero receivers and the event would otherwise be
//! silently dropped (issues #1804 / #1882). This buffer holds "important"
//! events for a TTL window so a reattaching client can replay them in
//! order before resuming live publish.
//!
//! ## Critical invariants
//!
//! - **Per-session isolation**: each [`SessionKey`] has its own ring; one
//!   session's buffer cannot be drained by another session's reattach.
//! - **No double-deliver**: publish (`broadcast::send` + buffer append) and
//!   reattach (`subscribe` + buffer drain) are serialised by a per-session
//!   `parking_lot::Mutex`. Any publish strictly before drain lands only in the
//!   snapshot; any publish strictly after drain lands only on the broadcast.
//!   There is no window where a single event reaches both paths.
//! - **Hard memory bound**: bounded by both event count and total bytes —
//!   whichever cap fills first triggers FIFO eviction. Buffering can never OOM
//!   regardless of producer rate.
//! - **TTL**: events older than `TTL` are evicted lazily on every publish/drain
//!   (so no idle session needs a sweep), and a low-frequency background sweeper
//!   drops fully-empty session entries.
//!
//! Caps live as `const` next to the buffer — they are mechanism tuning for
//! the always-on bug fix, not deployment configuration. See
//! `docs/guides/anti-patterns.md` "Mechanism constants are not config".

use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use parking_lot::Mutex;
use rara_kernel::session::SessionKey;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::web::WebEvent;

/// Maximum number of buffered events per session before FIFO eviction.
const CAPACITY_EVENTS: usize = 256;

/// Maximum total serialized bytes per session before FIFO eviction.
/// Bytes are estimated from `serde_json::to_string(&event).len()`.
const CAPACITY_BYTES: usize = 2 * 1024 * 1024;

/// How long an individual event survives in the buffer before being
/// evicted on the next publish or drain.
const TTL: Duration = Duration::from_mins(5);

/// Sweeper interval for dropping fully-empty session entries. The
/// per-event TTL is enforced inline on publish/drain; the sweeper only
/// reclaims map slots whose buffer has been empty long enough that no
/// reattach is realistically going to need them.
const SESSION_SWEEP_INTERVAL: Duration = Duration::from_mins(1);

/// One buffered event with the timestamp used for TTL eviction and the
/// pre-computed serialized size used for the byte-cap accounting.
struct Buffered {
    event:     WebEvent,
    queued_at: Instant,
    bytes:     usize,
}

/// Per-session bounded ring with byte accounting and a TTL cursor.
///
/// The mutex around this struct is the serialisation point for the
/// "no double deliver" invariant: see module docs.
struct SessionBuffer {
    events:     VecDeque<Buffered>,
    bytes:      usize,
    last_write: Instant,
}

impl SessionBuffer {
    fn new() -> Self {
        Self {
            events:     VecDeque::new(),
            bytes:      0,
            last_write: Instant::now(),
        }
    }

    /// Drop events older than `TTL`. Called on every publish/drain so an
    /// abandoned session stops returning stale data even if no sweeper
    /// has run yet.
    fn evict_expired(&mut self, now: Instant) {
        while let Some(front) = self.events.front() {
            if now.duration_since(front.queued_at) <= TTL {
                break;
            }
            let dropped = self
                .events
                .pop_front()
                .expect("front exists per peek above");
            self.bytes = self.bytes.saturating_sub(dropped.bytes);
        }
    }

    /// Append `event`, FIFO-evicting until both the event-count and
    /// byte-count caps are satisfied.
    fn push(&mut self, event: WebEvent, bytes: usize) {
        // Evict by count first so a flood of tiny events still leaves
        // room for the new entry without blowing the byte cap.
        while self.events.len() >= CAPACITY_EVENTS
            || (self.bytes + bytes > CAPACITY_BYTES && !self.events.is_empty())
        {
            let Some(dropped) = self.events.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(dropped.bytes);
        }
        self.events.push_back(Buffered {
            event,
            queued_at: Instant::now(),
            bytes,
        });
        self.bytes += bytes;
        self.last_write = Instant::now();
    }
}

/// Per-session reply buffer registry shared across all WS / SSE handlers
/// and the [`crate::web::WebAdapter`] publish path.
pub struct ReplyBuffer {
    sessions: DashMap<SessionKey, Arc<Mutex<SessionBuffer>>>,
}

impl ReplyBuffer {
    /// Construct an empty buffer.
    ///
    /// Returned as `Arc` because every consumer (`WebAdapter`, the WS
    /// handler, the sweeper task) holds its own reference.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: DashMap::new(),
        })
    }

    /// Decide whether an event must survive a "no listeners" publish.
    ///
    /// Streaming chunks (`TextDelta`, `ReasoningDelta`, …) intentionally
    /// fall through to `false` — replaying a partial token stream after
    /// the fact has no useful UX.
    #[must_use]
    pub fn should_buffer(event: &WebEvent) -> bool {
        matches!(
            event,
            WebEvent::Message { .. }
                | WebEvent::Error { .. }
                | WebEvent::BackgroundTaskDone { .. }
                | WebEvent::Progress { .. }
        )
    }

    /// Atomic publish: under the per-session mutex, optionally append
    /// `event` to the buffer (when [`Self::should_buffer`] is true) and
    /// then broadcast it via `tx`. Holding the lock across both steps is
    /// what guarantees the "no double-deliver" invariant relative to
    /// [`Self::subscribe_and_drain`]. Returns the result of `tx.send` so
    /// the caller can log "no receivers" the same way as before.
    pub fn publish(
        &self,
        session_key: &SessionKey,
        tx: &broadcast::Sender<WebEvent>,
        event: WebEvent,
    ) -> Result<usize, broadcast::error::SendError<WebEvent>> {
        let needs_buffer = Self::should_buffer(&event);
        let bytes = if needs_buffer {
            estimated_bytes(&event)
        } else {
            0
        };

        let entry = self.session_entry(session_key);
        let mut guard = entry.lock();
        let now = Instant::now();
        guard.evict_expired(now);
        if needs_buffer {
            guard.push(event.clone(), bytes);
        }
        // The send happens inside the lock so a concurrent
        // `subscribe_and_drain` cannot insert itself between the buffer
        // append and the broadcast emission.
        tx.send(event)
    }

    /// Atomically subscribe to `tx` and drain any buffered events that
    /// have not yet expired. Returns the receiver and the drained
    /// events in publish order. The buffer is cleared as part of this
    /// operation — re-subscribing later will not see the same events
    /// twice (per-connection idempotency).
    ///
    /// New events that arrive *after* this call return via the
    /// receiver only and are NOT re-buffered for this connection
    /// (publishes that race with drain block on the same mutex; see
    /// [`Self::publish`]).
    pub fn subscribe_and_drain(
        &self,
        session_key: &SessionKey,
        tx: &broadcast::Sender<WebEvent>,
    ) -> (broadcast::Receiver<WebEvent>, Vec<WebEvent>) {
        let entry = self.session_entry(session_key);
        let mut guard = entry.lock();
        // Subscribe BEFORE draining so any publish that arrives after we
        // release the lock is delivered live; we already hold the lock
        // so no concurrent publish can sneak between subscribe + drain.
        let rx = tx.subscribe();
        let now = Instant::now();
        guard.evict_expired(now);
        let drained: Vec<WebEvent> = guard.events.drain(..).map(|b| b.event).collect();
        guard.bytes = 0;
        (rx, drained)
    }

    /// Snapshot of currently buffered events for `session_key`, in
    /// publish order, with TTL eviction applied. Provided for tests and
    /// metrics; the production WS reattach path uses
    /// [`Self::subscribe_and_drain`] instead.
    #[must_use]
    pub fn snapshot(&self, session_key: &SessionKey) -> Vec<WebEvent> {
        let Some(entry) = self.sessions.get(session_key).map(|e| e.clone()) else {
            return Vec::new();
        };
        let mut guard = entry.lock();
        guard.evict_expired(Instant::now());
        guard.events.iter().map(|b| b.event.clone()).collect()
    }

    /// Number of currently tracked sessions — exposed for tests / metrics.
    #[doc(hidden)]
    #[must_use]
    pub fn session_count(&self) -> usize { self.sessions.len() }

    /// Spawn the background sweeper that drops fully-empty session
    /// entries. Returns immediately; the sweeper runs until `cancel`
    /// fires.
    pub fn spawn_sweeper(self: Arc<Self>, cancel: CancellationToken) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SESSION_SWEEP_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = ticker.tick() => {
                        self.sweep();
                    }
                }
            }
        });
    }

    /// Sweeper body, pulled out for direct unit testing. Drops session
    /// entries whose buffers are empty after TTL eviction.
    #[doc(hidden)]
    pub fn sweep(&self) {
        let now = Instant::now();
        let victims: Vec<SessionKey> = self
            .sessions
            .iter()
            .filter_map(|entry| {
                let mut guard = entry.value().lock();
                guard.evict_expired(now);
                guard.events.is_empty().then_some(*entry.key())
            })
            .collect();
        for k in victims {
            // Re-check under the per-entry lock so a publish racing with
            // sweep does not lose its append.
            if let Some((_, arc)) = self.sessions.remove_if(&k, |_, v| {
                let g = v.lock();
                g.events.is_empty()
            }) {
                drop(arc);
            }
        }
    }

    fn session_entry(&self, session_key: &SessionKey) -> Arc<Mutex<SessionBuffer>> {
        self.sessions
            .entry(*session_key)
            .or_insert_with(|| Arc::new(Mutex::new(SessionBuffer::new())))
            .clone()
    }
}

/// Best-effort serialised byte estimate used for the byte cap. We pay
/// this cost only when `should_buffer` is true, so the streaming hot
/// path is unaffected. Falls back to a small constant when the event
/// somehow fails to serialize (no `WebEvent` variant currently does).
fn estimated_bytes(event: &WebEvent) -> usize {
    serde_json::to_string(event).map(|s| s.len()).unwrap_or(64)
}

#[cfg(test)]
mod tests {
    use rara_kernel::{io::BackgroundTaskStatus, session::SessionKey};
    use tokio::sync::broadcast;

    use super::{ReplyBuffer, WebEvent};

    fn session() -> SessionKey { SessionKey::new() }

    fn msg(s: &str) -> WebEvent {
        WebEvent::Message {
            content: s.to_owned(),
        }
    }

    #[test]
    fn streaming_events_are_not_buffered() {
        assert!(!ReplyBuffer::should_buffer(&WebEvent::TextDelta {
            text: "x".to_owned(),
        }));
        assert!(!ReplyBuffer::should_buffer(&WebEvent::ReasoningDelta {
            text: "x".to_owned(),
        }));
    }

    #[test]
    fn important_events_are_buffered() {
        assert!(ReplyBuffer::should_buffer(&msg("hi")));
        assert!(ReplyBuffer::should_buffer(&WebEvent::Error {
            message:     "bad".to_owned(),
            category:    None,
            upgrade_url: None,
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
        let (tx, _rx) = broadcast::channel(16);
        buf.publish(&s, &tx, msg("first")).ok();
        buf.publish(&s, &tx, msg("second")).ok();
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
    fn subscribe_and_drain_clears_buffer() {
        let buf = ReplyBuffer::new();
        let s = session();
        let (tx, _rx) = broadcast::channel(16);
        buf.publish(&s, &tx, msg("a")).ok();
        buf.publish(&s, &tx, msg("b")).ok();

        let (_rx2, drained) = buf.subscribe_and_drain(&s, &tx);
        assert_eq!(drained.len(), 2);
        // Second drain returns nothing because the first cleared it.
        let (_rx3, drained2) = buf.subscribe_and_drain(&s, &tx);
        assert!(drained2.is_empty());
    }

    #[test]
    fn per_session_isolation() {
        let buf = ReplyBuffer::new();
        let s1 = session();
        let s2 = session();
        let (tx1, _rx1) = broadcast::channel(16);
        let (tx2, _rx2) = broadcast::channel(16);
        buf.publish(&s1, &tx1, msg("for-1")).ok();
        buf.publish(&s2, &tx2, msg("for-2")).ok();

        let (_, drained_s2) = buf.subscribe_and_drain(&s2, &tx2);
        assert_eq!(drained_s2.len(), 1);
        // s1's buffer must remain intact.
        assert_eq!(buf.snapshot(&s1).len(), 1);
    }

    #[test]
    fn sweep_drops_empty_sessions() {
        let buf = ReplyBuffer::new();
        let s = session();
        let (tx, _rx) = broadcast::channel(16);
        buf.publish(&s, &tx, msg("x")).ok();
        // Drain to leave the entry empty, then sweep.
        let _ = buf.subscribe_and_drain(&s, &tx);
        assert_eq!(buf.session_count(), 1);
        buf.sweep();
        assert_eq!(buf.session_count(), 0);
    }

    #[test]
    fn no_double_deliver_after_drain() {
        // Mid-flight scenario: subscribe_and_drain, then immediately
        // publish — the receiver must see the new event ONCE (live), and
        // a second drain must not re-emit it.
        let buf = ReplyBuffer::new();
        let s = session();
        let (tx, _rx) = broadcast::channel(16);
        buf.publish(&s, &tx, msg("buffered")).ok();

        let (mut rx, drained) = buf.subscribe_and_drain(&s, &tx);
        assert_eq!(drained.len(), 1);

        // New publish goes to the receiver and is also re-buffered for
        // any future reattach — but THIS receiver only sees it via
        // broadcast (no duplicate path).
        buf.publish(&s, &tx, msg("after-drain")).ok();
        let live = rx.try_recv().expect("receive new event");
        match live {
            WebEvent::Message { content } => assert_eq!(content, "after-drain"),
            other => panic!("unexpected: {other:?}"),
        }
        // No second copy queued for the same receiver.
        assert!(rx.try_recv().is_err());
    }
}
