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

//! Message types and on-disk format definitions.
//!
//! This module defines the core message types used throughout the queue:
//! - [`Message`]: The public type returned to consumers with sequence,
//!   timestamp, and payload
//! - `WriteEvent`: Internal type sent from Appender to `IOWorker` via channel
//!
//! ## On-Disk Message Format
//!
//! Messages are stored contiguously in data files with the following binary
//! layout:
//!
//! ```text
//! ┌─────────────────┬──────────────────────┬─────────────────┐
//! │  Length (4B)    │   Payload (variable) │   CRC32 (4B)    │
//! │  little-endian  │   raw bytes          │   little-endian │
//! └─────────────────┴──────────────────────┴─────────────────┘
//! ```
//!
//! - **Length**: 4-byte little-endian u32 containing the payload size
//! - **Payload**: Variable-length raw bytes (the actual message data)
//! - **CRC32**: 4-byte little-endian checksum over the payload for integrity
//!   verification
//!
//! This format enables:
//! - Sequential scanning (length prefix allows skipping to next message)
//! - Corruption detection (CRC64 validates payload integrity)
//! - Zero-copy reads (payload can be sliced directly from mmap)

use bytes::Bytes;

/// A message read from the queue.
///
/// This is the public type returned by [`Tailer`](crate::Tailer) when reading
/// messages. It contains all metadata needed for message processing and
/// ordering.
#[derive(Debug, Clone)]
pub struct Message {
    /// Monotonically increasing sequence number assigned at write time.
    /// Sequences are unique within a queue and never reused.
    pub sequence: u64,

    /// Unix timestamp in nanoseconds when the message was written.
    /// Useful for time-based queries and debugging.
    pub timestamp: u64,

    /// The message payload as zero-copy bytes.
    /// This is a view into the mmap'd file, avoiding allocation on read.
    pub payload: Bytes,
}

/// Internal event sent from Appender to `IOWorker`.
///
/// This is a simplified representation used in the write path. The `IOWorker`
/// receives these events via a crossbeam channel and writes them to disk.
/// The timestamp is captured by the `IOWorker` at write time, not by the
/// Appender.
#[derive(Debug, Clone)]
pub(crate) struct WriteEvent {
    /// Sequence number pre-assigned by the Appender.
    pub sequence: u64,

    /// Payload bytes to write to disk.
    pub data: Bytes,
}

/// Size of the length prefix in bytes (4 bytes = u32).
pub(crate) const MESSAGE_LENGTH_SIZE: usize = 4;

/// Size of the CRC32 checksum in bytes.
pub(crate) const MESSAGE_CRC_SIZE: usize = 4;

/// Calculate the total on-disk size of a message given its payload length.
///
/// This includes the length prefix, payload, and CRC checksum.
///
/// # Examples
///
/// ```ignore
/// // A 100-byte payload requires 108 bytes on disk:
/// // 4 (length) + 100 (payload) + 4 (crc) = 108
/// assert_eq!(message_disk_size(100), 108);
/// ```
#[inline]
pub(crate) const fn message_disk_size(payload_len: usize) -> usize {
    MESSAGE_LENGTH_SIZE + payload_len + MESSAGE_CRC_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_disk_size() {
        assert_eq!(message_disk_size(0), 8);
        assert_eq!(message_disk_size(10), 18);
        assert_eq!(message_disk_size(100), 108);
    }

    #[test]
    fn test_write_event_clone() {
        let event = WriteEvent {
            sequence: 42,
            data:     Bytes::from("test data"),
        };

        let cloned = event;
        assert_eq!(cloned.sequence, 42);
        assert_eq!(cloned.data, Bytes::from("test data"));
    }
}
