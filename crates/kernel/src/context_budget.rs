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

//! Two-layer context budget for tool result truncation.
//!
//! Prevents context bloat from large MCP tool results (50K+ tokens).
//!
//! - **Layer 1** ([`truncate_tool_result`]): Per-result cap at 30% of context
//!   window, applied when each tool result is pushed into messages.
//! - **Layer 2** ([`apply_context_guard`]): Whole-history scan before each LLM
//!   request — caps any single result > 50% and compacts oldest results when
//!   total tool content exceeds 75% of headroom.

use tracing::debug;

use crate::llm::{self, ToolDefinition};

/// Estimated chars-per-token for tool result content.
///
/// Tool output is denser than natural language (code, JSON, paths),
/// so we use 3 as a middle ground between the conservative 4 in
/// `agent.rs` and the aggressive 2 in OpenFang.
const TOOL_CHARS_PER_TOKEN: usize = 3;

/// Target size when compacting old tool results in Layer 2.
const COMPACT_TARGET_CHARS: usize = 2_000;

// -------------------------------------------------------------------------
// Layer 1: per-result truncation
// -------------------------------------------------------------------------

/// Truncate a single tool result to 30% of the model's context window.
///
/// Breaks at newline boundaries when possible to avoid mid-line cuts.
/// Appends a `[TRUNCATED: ...]` marker when content is shortened.
pub fn truncate_tool_result(content: &str, context_window_tokens: usize) -> String {
    let cap = per_result_cap(context_window_tokens);
    if content.len() <= cap {
        return content.to_string();
    }

    let break_point = find_break_point(content, cap);

    format!(
        "{}\n\n[TRUNCATED: result was {} chars, showing first {}]",
        &content[..break_point],
        content.len(),
        break_point,
    )
}

// -------------------------------------------------------------------------
// Layer 2: context guard
// -------------------------------------------------------------------------

/// Scan all tool-result messages and compact oversized results.
///
/// **Pass 1**: Cap any single tool result exceeding 50% of context window.
/// **Pass 2**: If total tool-result chars exceed 75% of headroom, compact
/// oldest results to ~2K chars each until under budget.
///
/// Returns the number of results that were compacted.
pub fn apply_context_guard(
    messages: &mut Vec<llm::Message>,
    context_window_tokens: usize,
    _tools: &[ToolDefinition],
) -> usize {
    let single_max = single_result_max(context_window_tokens);
    let headroom = total_tool_headroom_chars(context_window_tokens);

    // Collect tool-result locations (indices into messages vec).
    struct Loc {
        idx:      usize,
        char_len: usize,
    }

    let mut locations: Vec<Loc> = Vec::new();
    let mut total_chars: usize = 0;

    for (idx, msg) in messages.iter().enumerate() {
        if msg.role != llm::Role::Tool {
            continue;
        }
        let len = msg.content.as_text().len();
        total_chars += len;
        locations.push(Loc { idx, char_len: len });
    }

    if total_chars <= headroom {
        return 0;
    }

    debug!(
        total_chars,
        headroom,
        results = locations.len(),
        "context guard: tool results exceed headroom, compacting"
    );

    let mut compacted = 0usize;

    // Pass 1: cap any single result > 50% of context window
    for loc in &locations {
        if loc.char_len > single_max {
            if let Some(msg) = messages.get_mut(loc.idx) {
                let old_len = compact_message_content(msg, single_max);
                if old_len > 0 {
                    total_chars = total_chars - old_len + msg.content.as_text().len();
                    compacted += 1;
                }
            }
        }
    }

    // Pass 2: compact oldest results until under headroom
    for loc in &locations {
        if total_chars <= headroom {
            break;
        }
        if let Some(msg) = messages.get_mut(loc.idx) {
            let current_len = msg.content.as_text().len();
            if current_len > COMPACT_TARGET_CHARS {
                let old_len = compact_message_content(msg, COMPACT_TARGET_CHARS);
                if old_len > 0 {
                    total_chars = total_chars - old_len + msg.content.as_text().len();
                    compacted += 1;
                }
            }
        }
    }

    compacted
}

// -------------------------------------------------------------------------
// Internal helpers
// -------------------------------------------------------------------------

/// 30% of context window in chars.
fn per_result_cap(context_window_tokens: usize) -> usize {
    (context_window_tokens as f64 * 0.30) as usize * TOOL_CHARS_PER_TOKEN
}

/// 50% of context window in chars.
fn single_result_max(context_window_tokens: usize) -> usize {
    (context_window_tokens as f64 * 0.50) as usize * TOOL_CHARS_PER_TOKEN
}

/// 75% of context window in chars.
fn total_tool_headroom_chars(context_window_tokens: usize) -> usize {
    (context_window_tokens as f64 * 0.75) as usize * TOOL_CHARS_PER_TOKEN
}

/// Find a clean break point near `max_chars`, preferring newline boundaries.
///
/// Ensures the break is on a valid UTF-8 char boundary.
fn find_break_point(content: &str, max_chars: usize) -> usize {
    let mut safe = max_chars.min(content.len());
    // Walk back to char boundary
    while safe > 0 && !content.is_char_boundary(safe) {
        safe -= 1;
    }

    // Search last 200 bytes for a newline to break cleanly
    let search_start = {
        let mut s = safe.saturating_sub(200);
        while s > 0 && !content.is_char_boundary(s) {
            s -= 1;
        }
        s
    };

    content[search_start..safe]
        .rfind('\n')
        .map(|pos| search_start + pos)
        .unwrap_or(safe)
}

/// Replace message content with a truncated version. Returns old char length,
/// or 0 if nothing was done.
fn compact_message_content(msg: &mut llm::Message, max_chars: usize) -> usize {
    let text = msg.content.as_text();
    if text.len() <= max_chars {
        return 0;
    }
    let old_len = text.len();
    let break_point = find_break_point(text, max_chars.saturating_sub(80));

    let compacted = format!(
        "{}\n\n[COMPACTED: {} \u{2192} {} chars by context guard]",
        &text[..break_point],
        old_len,
        break_point,
    );
    msg.content = llm::MessageContent::Text(compacted);
    old_len
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_content_unchanged() {
        let result = truncate_tool_result("hello world", 200_000);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn truncates_at_newline_boundary() {
        // 100 tokens * 0.30 * 3 = 90 char cap
        let content = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\n\
                        line9\nline10\nline11\nline12\nline13\nline14\nline15\n\
                        line16\nline17\nline18\nline19\nline20\nline21";
        let result = truncate_tool_result(content, 100);
        assert!(result.contains("[TRUNCATED:"));
        // Should not split mid-line
        assert!(!result.starts_with("[TRUNCATED:"));
    }

    #[test]
    fn multibyte_content_does_not_panic() {
        // Tiny budget: cap = 30% of 100 * 3 = 90 bytes
        // Each Chinese char is 3 bytes; 100 chars = 300 bytes
        let content: String = "\u{4f60}\u{597d}\u{4e16}\u{754c}".repeat(25);
        assert_eq!(content.len(), 300);
        let result = truncate_tool_result(&content, 100);
        assert!(result.contains("[TRUNCATED:"));
    }

    #[test]
    fn emoji_content_does_not_panic() {
        // Each emoji is 4 bytes; 200 emojis = 800 bytes
        let content: String = "\u{1f600}".repeat(200);
        let result = truncate_tool_result(&content, 100);
        assert!(result.contains("[TRUNCATED:"));
    }

    #[test]
    fn guard_noop_when_under_budget() {
        let mut messages = vec![llm::Message::user("hello")];
        let compacted = apply_context_guard(&mut messages, 200_000, &[]);
        assert_eq!(compacted, 0);
    }

    #[test]
    fn guard_compacts_oversized_results() {
        // Tiny budget: headroom = 75% of 100 * 3 = 225 chars
        let big = "x".repeat(500);
        let mut messages = vec![
            llm::Message::tool_result("t1", big.clone()),
            llm::Message::tool_result("t2", big),
        ];

        let compacted = apply_context_guard(&mut messages, 100, &[]);
        assert!(compacted > 0);

        // Verify actually truncated
        let len = messages[0].content.as_text().len();
        assert!(len < 500, "expected < 500, got {len}");
    }

    #[test]
    fn guard_compacts_oldest_first() {
        // headroom = 75% of 5000 * 3 = 11250 chars
        // Two results of 8000 chars each = 16000 > 11250
        let result_a = "a\n".repeat(4000); // 8000 chars
        let result_b = "b\n".repeat(4000); // 8000 chars
        let mut messages = vec![
            llm::Message::tool_result("t1", result_a),
            llm::Message::tool_result("t2", result_b),
        ];

        let compacted = apply_context_guard(&mut messages, 5_000, &[]);
        assert!(compacted > 0);

        // Oldest (first) should have been compacted to ~2K
        let first_len = messages[0].content.as_text().len();
        assert!(first_len < 8000, "oldest should be compacted, got {first_len}");
    }

    #[test]
    fn guard_multibyte_tool_results() {
        // Chinese text tool result
        let big_chinese: String = "\u{4e2d}\u{6587}\u{6d4b}\u{8bd5}".repeat(200);
        let mut messages = vec![llm::Message::tool_result("t1", big_chinese)];
        // Must not panic
        let compacted = apply_context_guard(&mut messages, 100, &[]);
        assert!(compacted > 0);
    }

    #[test]
    fn per_result_cap_calculation() {
        // 200K tokens * 0.30 * 3 = 180K chars
        assert_eq!(per_result_cap(200_000), 180_000);
        // 8K tokens * 0.30 * 3 = 7200 chars
        assert_eq!(per_result_cap(8_000), 7_200);
    }
}
