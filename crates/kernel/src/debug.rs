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

//! Message debug summary — aggregates tape entries belonging to a single
//! `rara_message_id` into a structured view used by both the Telegram
//! `/debug` command and the `rara debug` CLI subcommand.
//!
//! This module is presentation-agnostic. It produces structured data
//! ([`MessageDebugSummary`]); rendering (HTML, plain text, JSON) lives in
//! the caller.

use crate::memory::TapEntry;

/// Aggregated debug view of a single message turn.
#[derive(Debug, Clone)]
pub struct MessageDebugSummary {
    /// The `rara_message_id` this summary describes.
    pub message_id:    String,
    /// All tape entries that referenced the message ID.
    pub entries:       Vec<TapEntry>,
    /// LLM model name (first non-empty value seen in metadata).
    pub model:         Option<String>,
    /// Cumulative input tokens across all iterations.
    pub input_tokens:  u64,
    /// Cumulative output tokens across all iterations.
    pub output_tokens: u64,
    /// Number of LLM iterations (entries with an `iteration` field).
    pub iterations:    usize,
    /// Cumulative streaming duration across all iterations, in milliseconds.
    pub stream_ms:     u64,
    /// Per-tool execution metrics extracted from `tool_metrics` metadata.
    pub tools:         Vec<ToolMetric>,
    /// Number of tool calls that reported `success: false`.
    pub tool_failures: usize,
    /// Timeline entries — kind, timestamp, and rendered detail.
    pub timeline:      Vec<TimelineItem>,
}

/// Single tool execution record extracted from tape metadata.
#[derive(Debug, Clone)]
pub struct ToolMetric {
    pub name:        String,
    pub duration_ms: Option<u64>,
    pub success:     bool,
    pub error:       Option<String>,
}

/// Timeline item shown in chronological order.
#[derive(Debug, Clone)]
pub struct TimelineItem {
    /// Tape entry kind ("message", "tool_call", "tool_result", ...).
    pub kind:      String,
    /// ISO timestamp string.
    pub timestamp: String,
    /// Rendered detail text (content preview, tool name, etc.).
    pub detail:    String,
}

impl MessageDebugSummary {
    /// Aggregate tape entries into a debug summary. Filters entries to
    /// only those whose `metadata.rara_message_id` matches the target.
    pub fn from_entries(message_id: &str, entries: Vec<TapEntry>) -> Self {
        let matched: Vec<TapEntry> = entries
            .into_iter()
            .filter(|e| {
                e.metadata.as_ref().is_some_and(|m| {
                    m.get("rara_message_id")
                        .and_then(|v| v.as_str())
                        .is_some_and(|id| id == message_id)
                })
            })
            .collect();

        let mut model: Option<String> = None;
        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;
        let mut iterations = 0usize;
        let mut stream_ms = 0u64;
        let mut tools: Vec<ToolMetric> = Vec::new();
        let mut tool_failures = 0usize;
        let mut timeline: Vec<TimelineItem> = Vec::with_capacity(matched.len());

        for entry in &matched {
            // Timeline detail derived from the payload.
            let kind = entry.kind.to_string();
            let detail = match kind.as_str() {
                "message" => entry
                    .payload
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| s.chars().take(100).collect::<String>())
                    .unwrap_or_default(),
                "tool_call" => {
                    let name = entry
                        .payload
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    format!("→ {name}")
                }
                "tool_result" => {
                    let name = entry
                        .payload
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let success = entry
                        .payload
                        .get("success")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let icon = if success { "✓" } else { "✗" };
                    format!("{icon} {name}")
                }
                _ => String::new(),
            };
            timeline.push(TimelineItem {
                kind,
                timestamp: entry.timestamp.strftime("%H:%M:%S").to_string(),
                detail,
            });

            // Aggregations from the metadata blob.
            let Some(meta) = entry.metadata.as_ref() else {
                continue;
            };

            if model.is_none() {
                if let Some(m) = meta.get("model").and_then(|v| v.as_str()) {
                    model = Some(m.to_owned());
                }
            }
            if let Some(usage) = meta.get("usage") {
                if let Some(t) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                    input_tokens += t;
                }
                if let Some(t) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                    output_tokens += t;
                }
            }
            if meta.get("iteration").is_some() {
                iterations += 1;
            }
            if let Some(ms) = meta.get("stream_ms").and_then(|v| v.as_u64()) {
                stream_ms += ms;
            }
            if let Some(metrics) = meta.get("tool_metrics").and_then(|v| v.as_array()) {
                for m in metrics {
                    let success = m.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
                    if !success {
                        tool_failures += 1;
                    }
                    tools.push(ToolMetric {
                        name: m
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                            .to_owned(),
                        duration_ms: m.get("duration_ms").and_then(|v| v.as_u64()),
                        success,
                        error: m.get("error").and_then(|v| v.as_str()).map(str::to_owned),
                    });
                }
            }
        }

        Self {
            message_id: message_id.to_owned(),
            entries: matched,
            model,
            input_tokens,
            output_tokens,
            iterations,
            stream_ms,
            tools,
            tool_failures,
            timeline,
        }
    }

    /// Returns true if no tape entries matched the message ID.
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}
