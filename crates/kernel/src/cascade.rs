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

//! Cascade execution trace model and builder.
//!
//! A cascade trace visualizes one agent turn as a sequence of "ticks" —
//! each tick represents a round of LLM reasoning followed by tool actions
//! and their observations.  The [`build_cascade`] function converts raw
//! [`TapEntry`] slices into a structured [`CascadeTrace`].

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::memory::{TapEntry, TapEntryKind};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Top-level cascade trace for a single agent turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeTrace {
    /// Opaque identifier for this trace (e.g. `"{session_key}-{seq}"`).
    pub message_id: String,
    /// Ordered list of ticks within the turn.
    pub ticks:      Vec<CascadeTick>,
    /// High-level summary statistics.
    pub summary:    CascadeSummary,
}

/// One reasoning-action cycle within a turn.
///
/// A new tick starts when a new assistant `Message` entry appears after
/// tool results (i.e. the LLM was called again).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeTick {
    /// Zero-based tick index within the trace.
    pub index:   usize,
    /// Entries belonging to this tick, in chronological order.
    pub entries: Vec<CascadeEntry>,
}

/// A single entry in the cascade trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeEntry {
    /// Human-readable entry ID: `"{kind_prefix} . {tick}-{short_id}-{seq}"`.
    pub id:        String,
    /// What kind of entry this is.
    pub kind:      CascadeEntryKind,
    /// Display content (text, tool arguments, tool output, etc.).
    pub content:   String,
    /// Timestamp from the underlying tape entry.
    pub timestamp: Timestamp,
    /// Optional structured metadata from the tape entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata:  Option<Value>,
}

/// Classification of cascade entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CascadeEntryKind {
    /// The user's input message that started this turn.
    UserInput,
    /// Assistant reasoning / textual response.
    Thought,
    /// A tool invocation (action).
    Action,
    /// Tool execution result (observation).
    Observation,
}

impl CascadeEntryKind {
    /// Short prefix used in entry IDs.
    fn prefix(self) -> &'static str {
        match self {
            Self::UserInput => "usr",
            Self::Thought => "thk",
            Self::Action => "act",
            Self::Observation => "obs",
        }
    }
}

/// Aggregate statistics for the cascade trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeSummary {
    /// Total number of ticks (LLM call rounds).
    pub tick_count:      usize,
    /// Total number of tool invocations.
    pub tool_call_count: usize,
    /// Total number of entries across all ticks.
    pub total_entries:   usize,
}

// ---------------------------------------------------------------------------
// Incremental Builder
// ---------------------------------------------------------------------------

/// Incremental cascade trace builder.
///
/// Mirrors the state machine of [`build_cascade`] but accepts entries one at a
/// time via typed `push_*` methods.  This allows the agent loop to construct
/// the trace as entries are created instead of doing a post-hoc scan.
pub struct CascadeBuilder {
    message_id:           String,
    ticks:                Vec<CascadeTick>,
    current_entries:      Vec<CascadeEntry>,
    tick_index:           usize,
    global_seq:           usize,
    tool_call_count:      usize,
    seen_assistant:       bool,
    last_was_tool_result: bool,
}

impl CascadeBuilder {
    /// Create a new builder for the given message/trace identifier.
    pub fn new(message_id: String) -> Self {
        Self {
            message_id,
            ticks: Vec::new(),
            current_entries: Vec::new(),
            tick_index: 0,
            global_seq: 0,
            tool_call_count: 0,
            seen_assistant: false,
            last_was_tool_result: false,
        }
    }

    /// Append a user-input entry.
    pub fn push_user(
        &mut self,
        entry_id: u64,
        content: &str,
        timestamp: Timestamp,
        metadata: Option<Value>,
    ) {
        self.global_seq += 1;
        self.current_entries.push(CascadeEntry {
            id: format_entry_id(
                CascadeEntryKind::UserInput,
                self.tick_index,
                entry_id,
                self.global_seq,
            ),
            kind: CascadeEntryKind::UserInput,
            content: content.to_owned(),
            timestamp,
            metadata,
        });
    }

    /// Append an assistant (thought) entry.
    ///
    /// Detects tick boundaries using the same logic as [`build_cascade`]: a new
    /// tick starts when an assistant message arrives after tool results and at
    /// least one entry already exists in the current tick.
    pub fn push_assistant(
        &mut self,
        entry_id: u64,
        text: &str,
        reasoning: Option<&str>,
        timestamp: Timestamp,
        metadata: Option<Value>,
    ) {
        // Tick boundary detection — identical to build_cascade.
        if self.seen_assistant && self.last_was_tool_result && !self.current_entries.is_empty() {
            self.ticks.push(CascadeTick {
                index:   self.tick_index,
                entries: std::mem::take(&mut self.current_entries),
            });
            self.tick_index += 1;
        }
        self.seen_assistant = true;
        self.last_was_tool_result = false;

        let reasoning = reasoning.unwrap_or("");
        let content = if !reasoning.is_empty() && !text.is_empty() {
            format!("[reasoning]\n{reasoning}\n\n[response]\n{text}")
        } else if !reasoning.is_empty() {
            reasoning.to_owned()
        } else {
            text.to_owned()
        };

        if !content.is_empty() {
            self.global_seq += 1;
            self.current_entries.push(CascadeEntry {
                id: format_entry_id(
                    CascadeEntryKind::Thought,
                    self.tick_index,
                    entry_id,
                    self.global_seq,
                ),
                kind: CascadeEntryKind::Thought,
                content,
                timestamp,
                metadata,
            });
        }
    }

    /// Append tool-call (action) entries.
    ///
    /// Each `(name, arguments)` pair produces one [`CascadeEntryKind::Action`]
    /// entry, matching the per-call expansion in [`build_cascade`].
    pub fn push_tool_calls(
        &mut self,
        entry_id: u64,
        calls: &[(&str, &str)],
        timestamp: Timestamp,
        metadata: Option<Value>,
    ) {
        self.last_was_tool_result = false;
        for (name, args) in calls {
            self.global_seq += 1;
            self.tool_call_count += 1;
            self.current_entries.push(CascadeEntry {
                id: format_entry_id(
                    CascadeEntryKind::Action,
                    self.tick_index,
                    entry_id,
                    self.global_seq,
                ),
                kind: CascadeEntryKind::Action,
                content: format!("{name}({args})"),
                timestamp,
                metadata: metadata.clone(),
            });
        }
    }

    /// Append tool-result (observation) entries.
    ///
    /// Each result string produces one [`CascadeEntryKind::Observation`] entry.
    pub fn push_tool_results(
        &mut self,
        entry_id: u64,
        results: &[&str],
        timestamp: Timestamp,
        metadata: Option<Value>,
    ) {
        self.last_was_tool_result = true;
        for result in results {
            self.global_seq += 1;
            self.current_entries.push(CascadeEntry {
                id: format_entry_id(
                    CascadeEntryKind::Observation,
                    self.tick_index,
                    entry_id,
                    self.global_seq,
                ),
                kind: CascadeEntryKind::Observation,
                content: (*result).to_owned(),
                timestamp,
                metadata: metadata.clone(),
            });
        }
    }

    /// Consume the builder and produce the final [`CascadeTrace`].
    ///
    /// Flushes any remaining entries into the last tick and computes summary
    /// statistics using the same formula as [`build_cascade`].
    pub fn finish(mut self) -> CascadeTrace {
        if !self.current_entries.is_empty() {
            self.ticks.push(CascadeTick {
                index:   self.tick_index,
                entries: self.current_entries,
            });
        }

        let total_entries: usize = self.ticks.iter().map(|t| t.entries.len()).sum();

        CascadeTrace {
            message_id: self.message_id,
            ticks:      self.ticks,
            summary:    CascadeSummary {
                tick_count: self.tick_index + usize::from(total_entries > 0),
                tool_call_count: self.tool_call_count,
                total_entries,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Batch Builder
// ---------------------------------------------------------------------------

/// Build a structured cascade trace from a slice of tape entries.
///
/// The entries should cover exactly one agent turn (from the user message
/// through all assistant replies and tool calls until the next user message
/// or end of tape).
pub fn build_cascade(entries: &[TapEntry], message_id: &str) -> CascadeTrace {
    let mut ticks: Vec<CascadeTick> = Vec::new();
    let mut current_entries: Vec<CascadeEntry> = Vec::new();
    let mut tick_index: usize = 0;
    let mut global_seq: usize = 0;
    let mut tool_call_count: usize = 0;
    // Track whether we have seen at least one assistant message so we can
    // detect tick boundaries (a new assistant message after tool results).
    let mut seen_assistant = false;
    let mut last_was_tool_result = false;

    for entry in entries {
        match entry.kind {
            TapEntryKind::Message => {
                let role = entry
                    .payload
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("");

                match role {
                    "user" => {
                        global_seq += 1;
                        current_entries.push(CascadeEntry {
                            id:        format_entry_id(
                                CascadeEntryKind::UserInput,
                                tick_index,
                                entry.id,
                                global_seq,
                            ),
                            kind:      CascadeEntryKind::UserInput,
                            content:   extract_text_content(&entry.payload),
                            timestamp: entry.timestamp,
                            metadata:  entry.metadata.clone(),
                        });
                    }
                    "assistant" => {
                        // A new assistant message after tool results starts a new tick.
                        if seen_assistant && last_was_tool_result && !current_entries.is_empty() {
                            ticks.push(CascadeTick {
                                index:   tick_index,
                                entries: std::mem::take(&mut current_entries),
                            });
                            tick_index += 1;
                        }
                        seen_assistant = true;
                        last_was_tool_result = false;

                        let text = extract_text_content(&entry.payload);
                        // Extract reasoning from metadata if present.
                        let reasoning = entry
                            .metadata
                            .as_ref()
                            .and_then(|m| m.get("reasoning_content"))
                            .and_then(Value::as_str)
                            .unwrap_or("");

                        let content = if !reasoning.is_empty() && !text.is_empty() {
                            format!("[reasoning]\n{reasoning}\n\n[response]\n{text}")
                        } else if !reasoning.is_empty() {
                            reasoning.to_owned()
                        } else {
                            text
                        };

                        if !content.is_empty() {
                            global_seq += 1;
                            current_entries.push(CascadeEntry {
                                id: format_entry_id(
                                    CascadeEntryKind::Thought,
                                    tick_index,
                                    entry.id,
                                    global_seq,
                                ),
                                kind: CascadeEntryKind::Thought,
                                content,
                                timestamp: entry.timestamp,
                                metadata: entry.metadata.clone(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            TapEntryKind::ToolCall => {
                last_was_tool_result = false;
                if let Some(calls) = entry.payload.get("calls").and_then(Value::as_array) {
                    for call in calls {
                        let func = call.get("function").and_then(Value::as_object);
                        let name = func
                            .and_then(|f| f.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("unknown");
                        let args = func
                            .and_then(|f| f.get("arguments"))
                            .and_then(Value::as_str)
                            .unwrap_or("{}");

                        global_seq += 1;
                        tool_call_count += 1;
                        current_entries.push(CascadeEntry {
                            id:        format_entry_id(
                                CascadeEntryKind::Action,
                                tick_index,
                                entry.id,
                                global_seq,
                            ),
                            kind:      CascadeEntryKind::Action,
                            content:   format!("{name}({args})"),
                            timestamp: entry.timestamp,
                            metadata:  entry.metadata.clone(),
                        });
                    }
                }
            }
            TapEntryKind::ToolResult => {
                last_was_tool_result = true;
                if let Some(results) = entry.payload.get("results").and_then(Value::as_array) {
                    for result in results {
                        let content = match result {
                            Value::String(s) => s.clone(),
                            other => serde_json::to_string(other).unwrap_or_default(),
                        };
                        global_seq += 1;
                        current_entries.push(CascadeEntry {
                            id: format_entry_id(
                                CascadeEntryKind::Observation,
                                tick_index,
                                entry.id,
                                global_seq,
                            ),
                            kind: CascadeEntryKind::Observation,
                            content,
                            timestamp: entry.timestamp,
                            metadata: entry.metadata.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // Flush remaining entries into the last tick.
    if !current_entries.is_empty() {
        ticks.push(CascadeTick {
            index:   tick_index,
            entries: current_entries,
        });
    }

    let total_entries: usize = ticks.iter().map(|t| t.entries.len()).sum();

    CascadeTrace {
        message_id: message_id.to_owned(),
        ticks,
        summary: CascadeSummary {
            tick_count: tick_index + usize::from(total_entries > 0),
            tool_call_count,
            total_entries,
        },
    }
}

/// Format a human-readable entry ID.
fn format_entry_id(kind: CascadeEntryKind, tick: usize, entry_id: u64, seq: usize) -> String {
    // Use the last 4 hex digits of the entry ID as a short identifier.
    let short_id = format!("{:04x}", entry_id & 0xFFFF);
    format!("{} \u{2022} {}-{}-{}", kind.prefix(), tick, short_id, seq)
}

/// Find indices of user-message entries in a tape, defining turn boundaries.
///
/// Each returned index marks the start of a new "turn" (user → assistant
/// round).  The slice for turn *N* spans `boundaries[N] .. boundaries[N+1]`
/// (or end-of-tape for the last turn).
pub fn find_turn_boundaries(entries: &[TapEntry]) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.kind == TapEntryKind::Message
                && e.payload
                    .get("role")
                    .and_then(Value::as_str)
                    .is_some_and(|r| r == "user")
        })
        .map(|(i, _)| i)
        .collect()
}

/// Extract the sub-slice of `entries` that belongs to turn number `turn`
/// (0-based), given pre-computed `boundaries` from [`find_turn_boundaries`].
pub fn turn_slice<'a>(
    entries: &'a [TapEntry],
    boundaries: &[usize],
    turn: usize,
) -> &'a [TapEntry] {
    let start = boundaries.get(turn).copied().unwrap_or(0);
    let end = boundaries.get(turn + 1).copied().unwrap_or(entries.len());
    &entries[start..end]
}

/// Find the turn whose user-message timestamp is closest to (but not after)
/// `target`.  Returns a 0-based turn index suitable for [`turn_slice`].
pub fn find_turn_by_timestamp(
    entries: &[TapEntry],
    boundaries: &[usize],
    target: Timestamp,
) -> usize {
    if boundaries.is_empty() {
        return 0;
    }
    boundaries
        .iter()
        .rposition(|&i| entries[i].timestamp <= target)
        .unwrap_or(0)
}

/// Extract plain text content from a message payload.
fn extract_text_content(payload: &Value) -> String {
    match payload.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(Value::as_str) == Some("text") {
                    b.get("text").and_then(Value::as_str).map(str::to_owned)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use serde_json::json;

    use super::*;

    fn make_entry(id: u64, kind: TapEntryKind, payload: Value) -> TapEntry {
        TapEntry {
            id,
            kind,
            payload,
            timestamp: Timestamp::now(),
            metadata: None,
        }
    }

    #[test]
    fn build_cascade_single_turn_no_tools() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hello"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "hi there"}),
            ),
        ];
        let trace = build_cascade(&entries, "test-1");
        assert_eq!(trace.ticks.len(), 1);
        assert_eq!(trace.summary.tool_call_count, 0);
        assert_eq!(trace.ticks[0].entries.len(), 2);
        assert_eq!(trace.ticks[0].entries[0].kind, CascadeEntryKind::UserInput);
        assert_eq!(trace.ticks[0].entries[1].kind, CascadeEntryKind::Thought);
    }

    #[test]
    fn build_cascade_with_tool_calls() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "search for rust"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": ""}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({
                    "calls": [{"id": "c1", "function": {"name": "search", "arguments": "{\"q\":\"rust\"}"}}]
                }),
            ),
            make_entry(
                4,
                TapEntryKind::ToolResult,
                json!({
                    "results": ["found 10 results"]
                }),
            ),
            make_entry(
                5,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "I found 10 results."}),
            ),
        ];
        let trace = build_cascade(&entries, "test-2");
        assert_eq!(trace.ticks.len(), 2);
        assert_eq!(trace.summary.tool_call_count, 1);
        // First tick: user + assistant(empty skipped) + action + observation
        // Second tick: assistant thought
        assert!(trace.ticks[1].entries[0].kind == CascadeEntryKind::Thought);
    }

    #[test]
    fn build_cascade_empty_entries() {
        let trace = build_cascade(&[], "empty");
        assert!(trace.ticks.is_empty());
        assert_eq!(trace.summary.total_entries, 0);
        assert_eq!(trace.summary.tick_count, 0);
        assert_eq!(trace.summary.tool_call_count, 0);
    }

    #[test]
    fn multi_iteration_creates_multiple_ticks() {
        // user -> assistant -> tool_call -> tool_result -> assistant -> tool_call ->
        // tool_result -> assistant
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "do two things"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "first I'll search"}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c1", "function": {"name": "search", "arguments": "{}"}}]}),
            ),
            make_entry(4, TapEntryKind::ToolResult, json!({"results": ["result1"]})),
            make_entry(
                5,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "now I'll read"}),
            ),
            make_entry(
                6,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c2", "function": {"name": "read", "arguments": "{}"}}]}),
            ),
            make_entry(7, TapEntryKind::ToolResult, json!({"results": ["result2"]})),
            make_entry(
                8,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "done"}),
            ),
        ];
        let trace = build_cascade(&entries, "multi");
        assert_eq!(trace.ticks.len(), 3);
        assert_eq!(trace.summary.tick_count, 3);
        assert_eq!(trace.summary.tool_call_count, 2);

        // Tick 0: user_input + thought + action + observation
        assert_eq!(trace.ticks[0].index, 0);
        assert_eq!(trace.ticks[0].entries.len(), 4);
        assert_eq!(trace.ticks[0].entries[0].kind, CascadeEntryKind::UserInput);
        assert_eq!(trace.ticks[0].entries[1].kind, CascadeEntryKind::Thought);
        assert_eq!(trace.ticks[0].entries[2].kind, CascadeEntryKind::Action);
        assert_eq!(
            trace.ticks[0].entries[3].kind,
            CascadeEntryKind::Observation
        );

        // Tick 1: thought + action + observation
        assert_eq!(trace.ticks[1].index, 1);
        assert_eq!(trace.ticks[1].entries[0].kind, CascadeEntryKind::Thought);
        assert_eq!(trace.ticks[1].entries[0].content, "now I'll read");

        // Tick 2: final thought
        assert_eq!(trace.ticks[2].index, 2);
        assert_eq!(trace.ticks[2].entries[0].kind, CascadeEntryKind::Thought);
        assert_eq!(trace.ticks[2].entries[0].content, "done");
    }

    #[test]
    fn multiple_tool_calls_in_single_entry() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hi"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "let me do both"}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({
                    "calls": [
                        {"id": "c1", "function": {"name": "search", "arguments": "{\"q\":\"a\"}"}},
                        {"id": "c2", "function": {"name": "read", "arguments": "{\"path\":\"b\"}"}}
                    ]
                }),
            ),
            make_entry(
                4,
                TapEntryKind::ToolResult,
                json!({"results": ["r1", "r2"]}),
            ),
        ];
        let trace = build_cascade(&entries, "multi-call");
        assert_eq!(trace.summary.tool_call_count, 2);
        // 2 action entries from the single ToolCall tape entry
        let actions: Vec<_> = trace.ticks[0]
            .entries
            .iter()
            .filter(|e| e.kind == CascadeEntryKind::Action)
            .collect();
        assert_eq!(actions.len(), 2);
        assert!(actions[0].content.starts_with("search("));
        assert!(actions[1].content.starts_with("read("));

        // 2 observation entries
        let obs: Vec<_> = trace.ticks[0]
            .entries
            .iter()
            .filter(|e| e.kind == CascadeEntryKind::Observation)
            .collect();
        assert_eq!(obs.len(), 2);
    }

    #[test]
    fn reasoning_content_from_metadata() {
        let mut entry = make_entry(
            2,
            TapEntryKind::Message,
            json!({"role": "assistant", "content": "visible response"}),
        );
        entry.metadata = Some(json!({"reasoning_content": "internal reasoning here"}));

        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "think"}),
            ),
            entry,
        ];
        let trace = build_cascade(&entries, "reasoning");
        let thought = &trace.ticks[0].entries[1];
        assert_eq!(thought.kind, CascadeEntryKind::Thought);
        assert!(thought.content.contains("[reasoning]"));
        assert!(thought.content.contains("internal reasoning here"));
        assert!(thought.content.contains("[response]"));
        assert!(thought.content.contains("visible response"));
    }

    #[test]
    fn reasoning_only_no_visible_content() {
        let mut entry = make_entry(
            2,
            TapEntryKind::Message,
            json!({"role": "assistant", "content": ""}),
        );
        entry.metadata = Some(json!({"reasoning_content": "just thinking"}));

        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "q"}),
            ),
            entry,
        ];
        let trace = build_cascade(&entries, "reasoning-only");
        let thought = &trace.ticks[0].entries[1];
        assert_eq!(thought.content, "just thinking");
        // Should NOT contain [reasoning] / [response] wrappers when only reasoning is
        // present
        assert!(!thought.content.contains("[reasoning]"));
    }

    #[test]
    fn empty_assistant_content_skipped() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hi"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": ""}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c1", "function": {"name": "search", "arguments": "{}"}}]}),
            ),
        ];
        let trace = build_cascade(&entries, "skip-empty");
        // Empty assistant content should not produce a Thought entry
        let thoughts: Vec<_> = trace.ticks[0]
            .entries
            .iter()
            .filter(|e| e.kind == CascadeEntryKind::Thought)
            .collect();
        assert!(thoughts.is_empty());
    }

    #[test]
    fn multimodal_content_extraction() {
        let entries = vec![make_entry(
            1,
            TapEntryKind::Message,
            json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "look at this"},
                    {"type": "image", "source": {"data": "base64..."}},
                    {"type": "text", "text": "what is it?"}
                ]
            }),
        )];
        let trace = build_cascade(&entries, "multimodal");
        let user_entry = &trace.ticks[0].entries[0];
        assert_eq!(user_entry.kind, CascadeEntryKind::UserInput);
        // Text blocks joined with newline, image blocks ignored
        assert_eq!(user_entry.content, "look at this\nwhat is it?");
    }

    #[test]
    fn tool_result_json_value_serialized() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hi"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "checking"}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c1", "function": {"name": "get_data", "arguments": "{}"}}]}),
            ),
            make_entry(
                4,
                TapEntryKind::ToolResult,
                json!({
                    "results": [{"key": "value", "count": 42}]
                }),
            ),
        ];
        let trace = build_cascade(&entries, "json-result");
        let obs = trace.ticks[0]
            .entries
            .iter()
            .find(|e| e.kind == CascadeEntryKind::Observation)
            .unwrap();
        // Non-string result should be JSON-serialized
        assert!(obs.content.contains("key"));
        assert!(obs.content.contains("value"));
        assert!(obs.content.contains("42"));
    }

    #[test]
    fn tool_result_string_value_preserved() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hi"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "ok"}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c1", "function": {"name": "echo", "arguments": "{}"}}]}),
            ),
            make_entry(
                4,
                TapEntryKind::ToolResult,
                json!({"results": ["plain text output"]}),
            ),
        ];
        let trace = build_cascade(&entries, "string-result");
        let obs = trace.ticks[0]
            .entries
            .iter()
            .find(|e| e.kind == CascadeEntryKind::Observation)
            .unwrap();
        assert_eq!(obs.content, "plain text output");
    }

    #[test]
    fn entry_id_format() {
        let entries = vec![make_entry(
            0xABCD,
            TapEntryKind::Message,
            json!({"role": "user", "content": "hi"}),
        )];
        let trace = build_cascade(&entries, "id-fmt");
        let id = &trace.ticks[0].entries[0].id;
        // Format: "{prefix} • {tick}-{hex4}-{seq}"
        assert!(id.starts_with("usr \u{2022} 0-abcd-1"), "got: {id}");
    }

    #[test]
    fn entry_id_truncates_to_last_4_hex() {
        let entries = vec![make_entry(
            0x12345678,
            TapEntryKind::Message,
            json!({"role": "user", "content": "hi"}),
        )];
        let trace = build_cascade(&entries, "id-trunc");
        let id = &trace.ticks[0].entries[0].id;
        // Only last 16 bits (0x5678) should appear
        assert!(id.contains("5678"), "got: {id}");
        assert!(
            !id.contains("1234"),
            "should not contain upper bits, got: {id}"
        );
    }

    #[test]
    fn summary_statistics_accurate() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "go"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "step1"}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({"calls": [
                    {"id": "c1", "function": {"name": "a", "arguments": "{}"}},
                    {"id": "c2", "function": {"name": "b", "arguments": "{}"}}
                ]}),
            ),
            make_entry(
                4,
                TapEntryKind::ToolResult,
                json!({"results": ["r1", "r2"]}),
            ),
            make_entry(
                5,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "step2"}),
            ),
            make_entry(
                6,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c3", "function": {"name": "c", "arguments": "{}"}}]}),
            ),
            make_entry(7, TapEntryKind::ToolResult, json!({"results": ["r3"]})),
            make_entry(
                8,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "final"}),
            ),
        ];
        let trace = build_cascade(&entries, "stats");
        assert_eq!(trace.summary.tick_count, 3);
        assert_eq!(trace.summary.tool_call_count, 3);
        // user(1) + thought(1) + action(2) + obs(2) + thought(1) + action(1) + obs(1) +
        // thought(1) = 10
        assert_eq!(trace.summary.total_entries, 10);
    }

    #[test]
    fn system_messages_ignored() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "system", "content": "you are a bot"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hi"}),
            ),
            make_entry(
                3,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "hello"}),
            ),
        ];
        let trace = build_cascade(&entries, "sys-ignore");
        assert_eq!(trace.ticks.len(), 1);
        // Only user_input and thought — system message is skipped
        assert_eq!(trace.ticks[0].entries.len(), 2);
        assert_eq!(trace.ticks[0].entries[0].kind, CascadeEntryKind::UserInput);
        assert_eq!(trace.ticks[0].entries[1].kind, CascadeEntryKind::Thought);
    }

    #[test]
    fn non_message_kinds_ignored() {
        // Event, System, Anchor, Note etc. should be silently skipped
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hi"}),
            ),
            make_entry(2, TapEntryKind::Event, json!({"event": "heartbeat"})),
            make_entry(3, TapEntryKind::System, json!({"info": "started"})),
            make_entry(4, TapEntryKind::Anchor, json!({"anchor": "a1"})),
            make_entry(5, TapEntryKind::Note, json!({"note": "internal"})),
            make_entry(
                6,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "hello"}),
            ),
        ];
        let trace = build_cascade(&entries, "skip-kinds");
        assert_eq!(trace.ticks.len(), 1);
        assert_eq!(trace.ticks[0].entries.len(), 2);
        assert_eq!(trace.summary.total_entries, 2);
    }

    #[test]
    fn only_user_message_produces_single_tick() {
        let entries = vec![make_entry(
            1,
            TapEntryKind::Message,
            json!({"role": "user", "content": "pending"}),
        )];
        let trace = build_cascade(&entries, "user-only");
        assert_eq!(trace.ticks.len(), 1);
        assert_eq!(trace.ticks[0].entries.len(), 1);
        assert_eq!(trace.ticks[0].entries[0].kind, CascadeEntryKind::UserInput);
        assert_eq!(trace.summary.tick_count, 1);
    }

    #[test]
    fn tool_call_missing_function_field() {
        // Malformed tool call payload — missing "function" key
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hi"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "ok"}),
            ),
            make_entry(3, TapEntryKind::ToolCall, json!({"calls": [{"id": "c1"}]})),
        ];
        let trace = build_cascade(&entries, "bad-call");
        let action = trace.ticks[0]
            .entries
            .iter()
            .find(|e| e.kind == CascadeEntryKind::Action)
            .unwrap();
        // Should fallback to "unknown" name and "{}" args
        assert_eq!(action.content, "unknown({})");
    }

    #[test]
    fn tool_call_no_calls_array() {
        // ToolCall entry with no "calls" key — should produce no action entries
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hi"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "ok"}),
            ),
            make_entry(3, TapEntryKind::ToolCall, json!({"something_else": true})),
        ];
        let trace = build_cascade(&entries, "no-calls");
        let actions: Vec<_> = trace.ticks[0]
            .entries
            .iter()
            .filter(|e| e.kind == CascadeEntryKind::Action)
            .collect();
        assert!(actions.is_empty());
        assert_eq!(trace.summary.tool_call_count, 0);
    }

    #[test]
    fn tool_result_no_results_array() {
        // ToolResult entry with no "results" key — should produce no observation
        // entries
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "hi"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "ok"}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c1", "function": {"name": "f", "arguments": "{}"}}]}),
            ),
            make_entry(4, TapEntryKind::ToolResult, json!({"error": "timeout"})),
        ];
        let trace = build_cascade(&entries, "no-results");
        let obs: Vec<_> = trace.ticks[0]
            .entries
            .iter()
            .filter(|e| e.kind == CascadeEntryKind::Observation)
            .collect();
        assert!(obs.is_empty());
    }

    #[test]
    fn message_id_propagated() {
        let trace = build_cascade(&[], "my-custom-id-123");
        assert_eq!(trace.message_id, "my-custom-id-123");
    }

    #[test]
    fn tick_indices_sequential() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "go"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "a"}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c1", "function": {"name": "t", "arguments": "{}"}}]}),
            ),
            make_entry(4, TapEntryKind::ToolResult, json!({"results": ["r"]})),
            make_entry(
                5,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "b"}),
            ),
            make_entry(
                6,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c2", "function": {"name": "t", "arguments": "{}"}}]}),
            ),
            make_entry(7, TapEntryKind::ToolResult, json!({"results": ["r"]})),
            make_entry(
                8,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "c"}),
            ),
        ];
        let trace = build_cascade(&entries, "idx");
        for (i, tick) in trace.ticks.iter().enumerate() {
            assert_eq!(tick.index, i, "tick {} has wrong index {}", i, tick.index);
        }
    }

    #[test]
    fn content_null_handled_gracefully() {
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": null}),
            ),
            make_entry(2, TapEntryKind::Message, json!({"role": "assistant"})),
        ];
        let trace = build_cascade(&entries, "null-content");
        // user with null content => empty string => still creates entry
        assert_eq!(trace.ticks[0].entries[0].kind, CascadeEntryKind::UserInput);
        assert_eq!(trace.ticks[0].entries[0].content, "");
        // assistant with no content key and no reasoning => empty, should be skipped
        let thoughts: Vec<_> = trace.ticks[0]
            .entries
            .iter()
            .filter(|e| e.kind == CascadeEntryKind::Thought)
            .collect();
        assert!(thoughts.is_empty());
    }

    #[test]
    fn metadata_preserved_on_entries() {
        let mut user_entry = make_entry(
            1,
            TapEntryKind::Message,
            json!({"role": "user", "content": "hi"}),
        );
        user_entry.metadata = Some(json!({"source": "telegram", "chat_id": 123}));

        let entries = vec![user_entry];
        let trace = build_cascade(&entries, "meta");
        let meta = trace.ticks[0].entries[0].metadata.as_ref().unwrap();
        assert_eq!(meta["source"], "telegram");
        assert_eq!(meta["chat_id"], 123);
    }

    #[test]
    fn cascade_entry_kind_prefix() {
        assert_eq!(CascadeEntryKind::UserInput.prefix(), "usr");
        assert_eq!(CascadeEntryKind::Thought.prefix(), "thk");
        assert_eq!(CascadeEntryKind::Action.prefix(), "act");
        assert_eq!(CascadeEntryKind::Observation.prefix(), "obs");
    }

    #[test]
    fn cascade_builder_matches_build_cascade() {
        // Reproduce the same scenario as multi_iteration_creates_multiple_ticks
        // using CascadeBuilder, then assert parity with build_cascade output.
        let ts = Timestamp::now();
        let msg_id = "builder-parity";

        // -- build_cascade path (from TapEntry slice) --
        let entries = vec![
            make_entry(
                1,
                TapEntryKind::Message,
                json!({"role": "user", "content": "do two things"}),
            ),
            make_entry(
                2,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "first I'll search"}),
            ),
            make_entry(
                3,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c1", "function": {"name": "search", "arguments": "{}"}}]}),
            ),
            make_entry(4, TapEntryKind::ToolResult, json!({"results": ["result1"]})),
            make_entry(
                5,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "now I'll read"}),
            ),
            make_entry(
                6,
                TapEntryKind::ToolCall,
                json!({"calls": [{"id": "c2", "function": {"name": "read", "arguments": "{}"}}]}),
            ),
            make_entry(7, TapEntryKind::ToolResult, json!({"results": ["result2"]})),
            make_entry(
                8,
                TapEntryKind::Message,
                json!({"role": "assistant", "content": "done"}),
            ),
        ];
        let expected = build_cascade(&entries, msg_id);

        // -- CascadeBuilder path --
        let mut builder = CascadeBuilder::new(msg_id.to_owned());
        builder.push_user(1, "do two things", ts, None);
        builder.push_assistant(2, "first I'll search", None, ts, None);
        builder.push_tool_calls(3, &[("search", "{}")], ts, None);
        builder.push_tool_results(4, &["result1"], ts, None);
        builder.push_assistant(5, "now I'll read", None, ts, None);
        builder.push_tool_calls(6, &[("read", "{}")], ts, None);
        builder.push_tool_results(7, &["result2"], ts, None);
        builder.push_assistant(8, "done", None, ts, None);
        let actual = builder.finish();

        // Structural parity checks.
        assert_eq!(actual.message_id, expected.message_id);
        assert_eq!(actual.ticks.len(), expected.ticks.len());
        assert_eq!(actual.summary.tick_count, expected.summary.tick_count);
        assert_eq!(
            actual.summary.tool_call_count,
            expected.summary.tool_call_count
        );
        assert_eq!(actual.summary.total_entries, expected.summary.total_entries);

        for (a_tick, e_tick) in actual.ticks.iter().zip(expected.ticks.iter()) {
            assert_eq!(a_tick.index, e_tick.index);
            assert_eq!(a_tick.entries.len(), e_tick.entries.len());
            for (a_entry, e_entry) in a_tick.entries.iter().zip(e_tick.entries.iter()) {
                assert_eq!(a_entry.kind, e_entry.kind);
                assert_eq!(a_entry.content, e_entry.content);
                assert_eq!(a_entry.id, e_entry.id);
            }
        }
    }
}
