// Copyright 2025 Crrow
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

//! Message writer (appender) for producing to the queue.
//!
//! The [`Appender`] provides a thread-safe interface for writing messages
//! to the queue. It:
//! - Atomically assigns sequence numbers
//! - Sends messages to the `IOWorker` via a channel
//! - Returns immediately (writes are asynchronous)
//!
//! ## Concurrency
//!
//! Appenders can be cloned and used from multiple threads. Each `append`
//! call atomically increments the global sequence counter, ensuring unique
//! sequence numbers even under concurrent writes.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use bytes::Bytes;
use crossbeam::channel::Sender;
use snafu::ensure;

use crate::{Result, error::ChannelSendSnafu, message::WriteEvent};

/// A writer for appending messages to the queue.
///
/// Appenders are cheap to clone and thread-safe. Multiple appenders can
/// write concurrently to the same queue.
#[derive(Clone)]
pub struct Appender {
    /// Channel sender to the `IOWorker`.
    tx:       Sender<WriteEvent>,
    /// Shared global sequence counter.
    sequence: Arc<AtomicU64>,
}

impl Appender {
    /// Create a new appender with the given channel and sequence counter.
    pub(crate) const fn new(tx: Sender<WriteEvent>, sequence: Arc<AtomicU64>) -> Self {
        Self { tx, sequence }
    }

    /// Append a message to the queue.
    ///
    /// Atomically assigns a sequence number and sends the message to the
    /// `IOWorker` for persistence. Returns the assigned sequence number.
    ///
    /// This method returns as soon as the message is enqueued to the channel,
    /// before it is persisted to disk. Use the queue's flush mode to control
    /// durability guarantees.
    ///
    /// # Errors
    ///
    /// Returns an error if the IO worker has shut down and can no longer
    /// receive messages.
    pub fn append(&self, data: impl Into<Bytes>) -> Result<u64> {
        let seq = self.sequence.fetch_add(1, Ordering::Relaxed);

        let event = WriteEvent {
            sequence: seq,
            data:     data.into(),
        };

        ensure!(self.tx.send(event).is_ok(), ChannelSendSnafu);

        Ok(seq)
    }

    /// Append multiple messages to the queue in a batch.
    ///
    /// Returns the sequence numbers of all appended messages.
    /// Note: This is not atomic - each message gets its own sequence number
    /// and is sent individually.
    ///
    /// # Errors
    ///
    /// Returns an error if the IO worker has shut down and can no longer
    /// receive messages.
    pub fn append_batch<I>(&self, items: I) -> Result<Vec<u64>>
    where
        I: IntoIterator<Item = Bytes>,
    {
        let mut sequences = Vec::new();

        for data in items {
            let seq = self.append(data)?;
            sequences.push(seq);
        }

        Ok(sequences)
    }

    /// Get the current global sequence number.
    ///
    /// This is the sequence that will be assigned to the next appended message.
    #[must_use]
    pub fn current_sequence(&self) -> u64 { self.sequence.load(Ordering::Relaxed) }
}

#[cfg(test)]
mod tests {
    use crossbeam::channel::unbounded;

    use super::*;

    #[test]
    fn test_appender_append() {
        let (tx, rx) = unbounded();
        let sequence = Arc::new(AtomicU64::new(0));
        let appender = Appender::new(tx, sequence);

        let seq1 = appender.append(b"message 1".as_slice()).unwrap();
        assert_eq!(seq1, 0);

        let seq2 = appender.append(Bytes::from("message 2")).unwrap();
        assert_eq!(seq2, 1);

        let event1 = rx.try_recv().unwrap();
        assert_eq!(event1.sequence, 0);
        assert_eq!(event1.data, Bytes::from("message 1"));

        let event2 = rx.try_recv().unwrap();
        assert_eq!(event2.sequence, 1);
        assert_eq!(event2.data, Bytes::from("message 2"));
    }

    #[test]
    fn test_appender_append_batch() {
        let (tx, rx) = unbounded();
        let sequence = Arc::new(AtomicU64::new(100));
        let appender = Appender::new(tx, sequence);

        let items = vec![
            Bytes::from("msg1"),
            Bytes::from("msg2"),
            Bytes::from("msg3"),
        ];

        let sequences = appender.append_batch(items).unwrap();
        assert_eq!(sequences, vec![100, 101, 102]);

        assert_eq!(rx.len(), 3);
    }

    #[test]
    fn test_appender_current_sequence() {
        let (tx, _rx) = unbounded();
        let sequence = Arc::new(AtomicU64::new(42));
        let appender = Appender::new(tx, sequence);

        assert_eq!(appender.current_sequence(), 42);
        appender.append(b"test".as_slice()).unwrap();
        assert_eq!(appender.current_sequence(), 43);
    }

    #[test]
    fn test_appender_clone() {
        let (tx, rx) = unbounded();
        let sequence = Arc::new(AtomicU64::new(0));
        let appender1 = Appender::new(tx, sequence);
        let appender2 = appender1.clone();

        appender1.append(b"from appender1".as_slice()).unwrap();
        appender2.append(b"from appender2".as_slice()).unwrap();

        assert_eq!(rx.len(), 2);

        let event1 = rx.try_recv().unwrap();
        let event2 = rx.try_recv().unwrap();

        assert_eq!(event1.sequence, 0);
        assert_eq!(event2.sequence, 1);
    }
}
