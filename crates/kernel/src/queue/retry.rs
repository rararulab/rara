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

//! Backpressure retry helper for the ingress pipeline.
//!
//! Channel adapters publish messages into the kernel's bounded
//! [`ShardedEventQueue`](crate::queue::ShardedEventQueue). When the target
//! shard is at capacity the queue returns [`IOError::Full`]. Without retry
//! the adapters would silently drop user messages — a latent reliability
//! bug (see issue #1148).
//!
//! [`push_with_retry`] applies a small bounded exponential-backoff retry,
//! and on exhaustion either appends the dropped envelope as a JSON line to
//! a configured dead-letter file or emits a structured error log.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{event::KernelEventEnvelope, io::IOError, queue::ShardedQueueRef};

/// Maximum number of `Full` retries before giving up.
const MAX_RETRIES: u32 = 3;

/// Initial backoff between retries (doubled on each attempt).
const BASE_DELAY: Duration = Duration::from_millis(100);

/// Configuration for the ingress retry/dead-letter behaviour.
///
/// `dead_letter_path` is intentionally an [`Option`] — defaults come from the
/// YAML config file, never from Rust. When unset, exhausted messages are only
/// recorded in the structured error log.
#[derive(Debug, Clone, Default)]
pub struct IngressConfig {
    /// Optional path to a dead-letter file. Dropped envelopes are appended as
    /// one JSON object per line. Parent directories must already exist.
    pub dead_letter_path: Option<PathBuf>,
}

/// Push an event into the sharded queue with bounded exponential-backoff
/// retry on [`IOError::Full`].
///
/// The retry budget is intentionally tiny — the queue is local and in-memory,
/// so persistent fullness indicates a stuck consumer rather than a transient
/// hiccup. The goal is to absorb micro-bursts (a few hundred ms), not to
/// paper over a real outage.
///
/// On exhaustion:
/// - If `cfg.dead_letter_path` is set, the dropped envelope is appended as a
///   JSON line to that file. Failure to write the dead letter is logged but
///   does not change the returned error — the caller still sees `Full`.
/// - Otherwise, a structured `error!` log is emitted.
///
/// Other [`IOError`] variants are returned immediately without retry; only
/// `Full` is retryable.
#[tracing::instrument(skip_all, fields(retries = MAX_RETRIES))]
pub async fn push_with_retry(
    queue: &ShardedQueueRef,
    event: KernelEventEnvelope,
    cfg: &IngressConfig,
) -> Result<(), IOError> {
    let mut delay = BASE_DELAY;
    let mut last_event = event;
    for attempt in 0..MAX_RETRIES {
        match queue.push_returning(last_event) {
            Ok(()) => return Ok(()),
            Err((IOError::Full, returned)) => {
                tracing::warn!(
                    attempt = attempt + 1,
                    backoff_ms = delay.as_millis() as u64,
                    "ingress queue full, backing off",
                );
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
                last_event = returned;
            }
            Err((other, _)) => return Err(other),
        }
    }

    handle_dead_letter(&last_event, cfg);
    Err(IOError::Full)
}

/// Either append the dropped envelope to the dead-letter file or emit a
/// structured error log when no path is configured.
fn handle_dead_letter(event: &KernelEventEnvelope, cfg: &IngressConfig) {
    let kind = event.event_type();
    if let Some(path) = cfg.dead_letter_path.as_deref() {
        match write_dead_letter(path, event) {
            Ok(()) => tracing::error!(
                event_kind = kind,
                dead_letter_path = %path.display(),
                "ingress queue full after retries — wrote dropped event to dead-letter file",
            ),
            Err(e) => tracing::error!(
                event_kind = kind,
                dead_letter_path = %path.display(),
                error = %e,
                "ingress queue full after retries — FAILED to write dead-letter file; event dropped",
            ),
        }
    } else {
        tracing::error!(
            event_kind = kind,
            "ingress queue full after retries — no dead-letter path configured; event dropped",
        );
    }
}

/// Append a JSON-encoded envelope as a single line to `path`.
fn write_dead_letter(path: &Path, event: &KernelEventEnvelope) -> std::io::Result<()> {
    use std::{
        fs::OpenOptions,
        io::{BufWriter, Write},
    };

    let payload = serde_json::to_string(event).map_err(std::io::Error::other)?;

    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(payload.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::queue::{ShardedEventQueue, ShardedEventQueueConfig};

    fn build_queue(global_capacity: usize) -> ShardedQueueRef {
        Arc::new(ShardedEventQueue::new(ShardedEventQueueConfig {
            num_shards: 0,
            shard_capacity: 1,
            global_capacity,
        }))
    }

    #[tokio::test(start_paused = true)]
    async fn push_with_retry_succeeds_when_space_frees_up() {
        let queue = build_queue(1);
        // Pre-fill the global queue.
        queue
            .push(KernelEventEnvelope::shutdown())
            .expect("first push fits");

        // Spawn a draining task that frees up space after a short delay.
        let drain_queue = queue.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            // Drop one event from the global queue to make room.
            let _ = drain_queue.global().drain(1).next();
        });

        let cfg = IngressConfig::default();
        push_with_retry(&queue, KernelEventEnvelope::shutdown(), &cfg)
            .await
            .expect("retry should eventually succeed once the queue drains");
    }

    #[tokio::test(start_paused = true)]
    async fn push_with_retry_returns_full_after_exhaustion() {
        let queue = build_queue(1);
        queue
            .push(KernelEventEnvelope::shutdown())
            .expect("first push fits");

        let cfg = IngressConfig::default();
        let err = push_with_retry(&queue, KernelEventEnvelope::shutdown(), &cfg)
            .await
            .expect_err("queue stays full so retry must surface IOError::Full");
        assert!(matches!(err, IOError::Full));
    }

    #[tokio::test(start_paused = true)]
    async fn dead_letter_file_is_appended_on_exhaustion() {
        let queue = build_queue(1);
        queue
            .push(KernelEventEnvelope::shutdown())
            .expect("first push fits");

        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let cfg = IngressConfig {
            dead_letter_path: Some(tmp.path().to_path_buf()),
        };

        let err = push_with_retry(&queue, KernelEventEnvelope::shutdown(), &cfg).await;
        assert!(matches!(err, Err(IOError::Full)));

        let contents = std::fs::read_to_string(tmp.path()).expect("read dead-letter file");
        assert!(
            contents.lines().count() >= 1,
            "dead-letter file should contain at least one JSON line, got: {contents:?}",
        );
    }
}
