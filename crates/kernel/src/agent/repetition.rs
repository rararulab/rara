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

//! Streaming repetition detection for LLM output.
//!
//! [`RepetitionGuard`] monitors accumulated text and detects when the model
//! starts looping — emitting the same block of content repeatedly.  When a
//! repeat is found the guard returns a byte index at which the caller should
//! truncate the output.

/// Trailing characters used as the search probe.
const PROBE_LEN: usize = 200;

/// Minimum accumulated character count before any check is performed.
const MIN_CHECK_LEN: usize = 600;

/// Character count between consecutive repetition checks.
const CHECK_INTERVAL: usize = 500;

/// Detects verbatim repetition in streaming LLM output.
///
/// The caller feeds each new text delta together with the full accumulated
/// output so far.  When the trailing `PROBE_LEN` characters appear earlier
/// in the accumulated text, `feed` returns `Some(byte_index)` indicating
/// the point at which the output should be truncated (keeping only the
/// first occurrence plus the probe).
pub(crate) struct RepetitionGuard {
    chars_since_check: usize,
    /// Running total of characters fed so far, avoiding O(n) recount.
    total_chars:       usize,
}

impl RepetitionGuard {
    /// Create a new guard with zero accumulated character count.
    pub(crate) fn new() -> Self {
        Self {
            chars_since_check: 0,
            total_chars:       0,
        }
    }

    /// Feed a new text delta and check for repetition.
    ///
    /// Returns `Some(byte_index)` when the trailing `PROBE_LEN` characters
    /// of `accumulated` also appear earlier in the text, indicating a
    /// verbatim loop.  The returned index points just past the first
    /// occurrence of the probe — the caller should truncate there.
    pub(crate) fn feed(&mut self, delta: &str, accumulated: &str) -> Option<usize> {
        let delta_chars = delta.chars().count();
        self.chars_since_check += delta_chars;
        self.total_chars += delta_chars;

        let total_chars = self.total_chars;
        if total_chars < MIN_CHECK_LEN {
            return None;
        }
        if self.chars_since_check < CHECK_INTERVAL {
            return None;
        }

        self.chars_since_check = 0;

        // Convert the last PROBE_LEN chars into a byte-bounded slice.
        let probe_char_start = total_chars - PROBE_LEN;
        let probe_start_byte = accumulated
            .char_indices()
            .nth(probe_char_start)
            .map(|(i, _)| i)
            .unwrap_or(0);

        let probe = &accumulated[probe_start_byte..];
        let search_hay = &accumulated[..probe_start_byte];

        // Look for the probe earlier in the text.
        search_hay
            .find(probe)
            .map(|match_byte_pos| match_byte_pos + probe.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a string of `n` chars where no 200-char substring repeats.
    ///
    /// Uses a simple counter encoding so every position is globally unique.
    fn unique_chars(n: usize) -> String {
        let mut s = String::with_capacity(n * 4);
        let mut i = 0u32;
        while s.chars().count() < n {
            // Encode counter as variable-length base-26 letters.
            let mut num = i;
            let mut buf = Vec::new();
            loop {
                buf.push(b'a' + (num % 26) as u8);
                num /= 26;
                if num == 0 {
                    break;
                }
            }
            buf.reverse();
            for &b in &buf {
                if s.chars().count() >= n {
                    break;
                }
                s.push(char::from(b));
            }
            // Separator to prevent cross-boundary matches.
            if s.chars().count() < n {
                s.push('_');
            }
            i += 1;
        }
        // Trim to exactly n chars.
        s.chars().take(n).collect()
    }

    #[test]
    fn no_repetition_returns_none() {
        let mut guard = RepetitionGuard::new();
        let text = unique_chars(800);
        let result = guard.feed(&text, &text);
        assert!(result.is_none(), "unique text must not trigger detection");
    }

    #[test]
    fn short_text_skips_check() {
        let mut guard = RepetitionGuard::new();
        let block = "x".repeat(400);
        let result = guard.feed(&block, &block);
        assert!(
            result.is_none(),
            "text shorter than MIN_CHECK_LEN must be skipped"
        );
    }

    #[test]
    fn exact_block_repetition_detected() {
        let mut guard = RepetitionGuard::new();
        let block = unique_chars(300);
        let repeated = format!("{block}{block}");
        let result = guard.feed(&repeated, &repeated);
        assert!(result.is_some(), "exact repetition must be detected");

        let trunc = result.unwrap();
        // Truncation should yield roughly one copy of the block.
        assert!(
            trunc <= block.len() + PROBE_LEN,
            "truncated length {trunc} should be at most one block + probe"
        );
    }

    #[test]
    fn cjk_text_repetition_detected() {
        let mut guard = RepetitionGuard::new();
        // 300 CJK characters (each 3 bytes in UTF-8).
        let cjk_block: String = (0..300)
            .map(|i| char::from_u32(0x4E00 + (i % 200) as u32).unwrap())
            .collect();
        let repeated = format!("{cjk_block}{cjk_block}");
        let result = guard.feed(&repeated, &repeated);
        assert!(result.is_some(), "CJK repetition must be detected");

        let trunc = result.unwrap();
        assert!(
            trunc <= cjk_block.len() + PROBE_LEN * 3 + 3,
            "CJK truncation index {trunc} unexpectedly large"
        );
    }

    #[test]
    fn near_miss_no_false_positive() {
        let mut guard = RepetitionGuard::new();
        let block_a = unique_chars(300);
        // Flip the last character to create a near-miss.
        let mut block_b = block_a.clone();
        let last = block_b.pop().unwrap();
        block_b.push(if last == 'Z' { 'A' } else { 'Z' });
        let combined = format!("{block_a}{block_b}");
        let result = guard.feed(&combined, &combined);
        assert!(
            result.is_none(),
            "near-miss blocks must not trigger false positive"
        );
    }

    #[test]
    fn check_interval_respected() {
        let mut guard = RepetitionGuard::new();
        let block = unique_chars(300);
        let repeated = format!("{block}{block}{block}");

        // Feed in 100-char increments; accumulate text.
        let mut detected = false;
        let chars: Vec<char> = repeated.chars().collect();
        let mut fed_chars = 0;
        for chunk_start in (0..chars.len()).step_by(100) {
            let chunk_end = (chunk_start + 100).min(chars.len());
            let delta: String = chars[chunk_start..chunk_end].iter().collect();
            fed_chars += chunk_end - chunk_start;
            let acc: String = chars[..fed_chars].iter().collect();
            if guard.feed(&delta, &acc).is_some() {
                detected = true;
                break;
            }
        }
        assert!(
            detected,
            "incremental feeding must eventually detect repetition"
        );
    }

    #[test]
    fn realistic_paragraph_repetition() {
        let mut guard = RepetitionGuard::new();
        let paragraph = "Rust是一种系统编程语言，专注于安全性、速度和并发性。\
                         它通过所有权系统实现内存安全，无需垃圾回收器。\
                         Rust的类型系统和借用检查器确保线程安全和内存安全。\
                         该语言适用于性能关键的服务、嵌入式设备和命令行工具。\
                         开发者喜欢Rust是因为它的表达力和可靠性保证。";
        let repeated = paragraph.repeat(5);
        let result = guard.feed(&repeated, &repeated);
        assert!(
            result.is_some(),
            "realistic paragraph repetition must be detected"
        );

        let trunc = result.unwrap();
        let two_copies = paragraph.len() * 2;
        assert!(
            trunc <= two_copies,
            "truncation point {trunc} should be at most 2x paragraph length ({two_copies})"
        );
    }
}
