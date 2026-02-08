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

#![allow(clippy::cast_possible_truncation)]

//! CRC32 checksum utilities for message integrity verification.
//!
//! Uses CRC-32 (IEEE polynomial) via crc32fast for hardware-accelerated
//! checksums. The CRC covers both the length field and payload to detect
//! truncation and corruption.

use crc32fast::Hasher;

/// Calculates CRC32 checksum for a message.
///
/// The checksum covers both the length prefix and payload data to detect:
/// - Payload corruption
/// - Length field corruption
/// - Truncated writes
///
/// # Arguments
/// * `length` - The payload length (will be included in CRC calculation)
/// * `data` - The payload bytes
///
/// # Returns
/// 32-bit CRC checksum
#[inline]
pub(crate) fn calculate_message_crc(length: u32, data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(&length.to_le_bytes());
    hasher.update(data);
    hasher.finalize()
}

/// Verifies a message's CRC32 checksum.
///
/// # Arguments
/// * `length` - The payload length from the message header
/// * `data` - The payload bytes
/// * `expected` - The stored CRC to verify against
///
/// # Returns
/// `true` if the checksum matches, `false` otherwise
#[inline]
pub(crate) fn verify_message_crc(length: u32, data: &[u8], expected: u32) -> bool {
    calculate_message_crc(length, data) == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_message_crc() {
        let data = b"test message";
        let length = data.len() as u32;
        let crc = calculate_message_crc(length, data);

        // Same input produces same CRC
        let crc2 = calculate_message_crc(length, data);
        assert_eq!(crc, crc2);

        // Different length produces different CRC
        let crc3 = calculate_message_crc(length + 1, data);
        assert_ne!(crc, crc3);
    }

    #[test]
    fn test_verify_message_crc() {
        let data = b"message to verify";
        let length = data.len() as u32;
        let crc = calculate_message_crc(length, data);

        assert!(verify_message_crc(length, data, crc));
        assert!(!verify_message_crc(length, data, crc.wrapping_add(1)));
        assert!(!verify_message_crc(length + 1, data, crc));
        assert!(!verify_message_crc(length, b"wrong", crc));
    }
}
