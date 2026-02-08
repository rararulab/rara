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

//! Content hashing utilities for resume deduplication.
//!
//! We use SHA-256 to produce a deterministic fingerprint of resume
//! content. Before hashing the text is normalized (trimmed, collapsed
//! whitespace) so that insignificant formatting differences do not
//! produce distinct hashes.

use sha2::{Digest, Sha256};

/// Compute a hex-encoded SHA-256 hash of the given resume content.
///
/// The content is normalized before hashing:
/// - leading/trailing whitespace is stripped
/// - runs of whitespace are collapsed to a single space
///
/// This ensures that trivial formatting changes (extra newlines, trailing
/// spaces) do not cause a hash mismatch.
#[must_use]
pub fn content_hash(content: &str) -> String {
    let normalized = normalize(content);
    let digest = Sha256::digest(normalized.as_bytes());
    format!("{digest:x}")
}

/// Normalize resume content for consistent hashing.
fn normalize(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_content_produces_same_hash() {
        let a = content_hash("Hello, world!");
        let b = content_hash("Hello, world!");
        assert_eq!(a, b);
    }

    #[test]
    fn whitespace_differences_produce_same_hash() {
        let a = content_hash("  Hello,   world!  \n\n");
        let b = content_hash("Hello, world!");
        assert_eq!(a, b);
    }

    #[test]
    fn different_content_produces_different_hash() {
        let a = content_hash("Version 1");
        let b = content_hash("Version 2");
        assert_ne!(a, b);
    }

    #[test]
    fn hash_is_64_hex_chars() {
        let h = content_hash("test");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
