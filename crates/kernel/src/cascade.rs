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
// Builder
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
    }
}
