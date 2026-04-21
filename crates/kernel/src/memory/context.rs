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

//! Tape-to-LLM context reconstruction.
//!
//! [`default_tape_context()`] converts a slice of [`TapEntry`] values into a
//! `Vec<llm::Message>` suitable for feeding to the LLM.  `Message` entries are
//! deserialized directly, `ToolCall` entries become assistant messages with a
//! `tool_calls` array, and `ToolResult` entries become tool-role messages.
//! Non-conversational kinds (`Event`, `System`, `Anchor`, `Note`, `Summary`)
//! are skipped.

use serde_json::Value;
use snafu::ResultExt;

use super::{HandoffState, TapEntry, TapEntryKind, TapResult};
use crate::llm::{Message, MessageContent, ToolCallRequest};

/// When the number of notes since the last anchor exceeds this threshold, a
/// hint is appended to the system message suggesting memory consolidation.
const CONSOLIDATION_HINT_THRESHOLD: usize = 15;

/// Hard safety cap on notes injected into LLM context.  When notes exceed this
/// limit the most recent entries are kept and a prominent overflow warning is
/// prepended.  This prevents unbounded context growth when distillation is
/// delayed.
const MAX_USER_NOTES_HARD_CAP: usize = 50;

/// Reconstruct LLM messages from persisted tape entries.
///
/// The reconstruction mirrors Bub's behavior:
/// - raw `message` entries are deserialized directly into [`Message`],
/// - `tool_call` entries become an assistant message with `tool_calls`,
/// - `tool_result` entries become one or more tool-role messages.
pub fn default_tape_context(entries: &[TapEntry]) -> TapResult<Vec<Message>> {
    let mut messages = Vec::new();
    let mut pending_calls: Vec<PendingCall> = Vec::new();

    for entry in entries {
        match entry.kind {
            TapEntryKind::Message => append_message_entry(&mut messages, entry),
            TapEntryKind::ToolCall => pending_calls = append_tool_call_entry(&mut messages, entry),
            TapEntryKind::ToolResult => {
                append_tool_result_entry(&mut messages, &pending_calls, entry)?;
                pending_calls.clear();
            }
            _ => {}
        }
    }

    Ok(messages)
}

fn append_message_entry(messages: &mut Vec<Message>, entry: &TapEntry) {
    if let Some(payload) = entry.payload.as_object() {
        if let Ok(msg) = serde_json::from_value::<Message>(Value::Object(payload.clone())) {
            messages.push(msg);
        }
    }
}

/// Intermediate representation for a pending tool call, used to pair
/// tool-call IDs with their corresponding tool-result messages.
struct PendingCall {
    id:   String,
    name: String,
}

/// Convert a persisted tool-call entry into an assistant message with tool
/// calls.
fn append_tool_call_entry(messages: &mut Vec<Message>, entry: &TapEntry) -> Vec<PendingCall> {
    let Some(calls_value) = entry.payload.get("calls").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut pending = Vec::new();
    let mut tool_calls = Vec::new();

    for call in calls_value {
        let Some(obj) = call.as_object() else {
            continue;
        };
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let function = obj.get("function").and_then(Value::as_object);
        let name = function
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let arguments = function
            .and_then(|f| f.get("arguments"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();

        pending.push(PendingCall {
            id:   id.clone(),
            name: name.clone(),
        });
        tool_calls.push(ToolCallRequest {
            id,
            name,
            arguments,
        });
    }

    if !tool_calls.is_empty() {
        let reasoning = entry
            .payload
            .get("reasoning_content")
            .and_then(Value::as_str)
            .map(String::from);
        messages.push(Message::assistant_with_tool_calls_and_reasoning(
            "", tool_calls, reasoning,
        ));
    }

    pending
}

/// Expand a persisted tool-result entry into one tool-role message per result.
fn append_tool_result_entry(
    messages: &mut Vec<Message>,
    pending_calls: &[PendingCall],
    entry: &TapEntry,
) -> TapResult<()> {
    let Some(results) = entry.payload.get("results").and_then(Value::as_array) else {
        return Ok(());
    };

    for (index, result) in results.iter().enumerate() {
        let content = render_tool_result(result)?;
        // Successful tool results are persisted from raw tool output in
        // agent/mod.rs, so a top-level `tool_call_id` may be user-visible tool
        // payload rather than our correlation id. Trust it only when it matches
        // one of the assistant's pending calls; otherwise keep the legacy
        // positional fallback.
        let call_id = result
            .get("tool_call_id")
            .and_then(Value::as_str)
            .filter(|id| pending_calls.iter().any(|call| call.id == *id))
            .or_else(|| pending_calls.get(index).map(|c| c.id.as_str()))
            .unwrap_or("");
        messages.push(Message::tool_result(call_id, content));
    }

    Ok(())
}

/// Render a tool result payload into the string content expected by LLM
/// messages.
fn render_tool_result(result: &Value) -> TapResult<String> {
    Ok(match result {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).context(super::error::JsonEncodeSnafu)?,
    })
}

// ---------------------------------------------------------------------------
// Anchor context
// ---------------------------------------------------------------------------

/// Build a system-role message from the last anchor's state so the LLM retains
/// key context (summary, next steps) even after older entries leave the context
/// window.
///
/// Returns `None` when the anchor carries no `summary` or `next_steps`.
pub fn anchor_context(entries: &[TapEntry]) -> Option<Message> {
    let anchor = entries
        .iter()
        .rev()
        .find(|e| e.kind == TapEntryKind::Anchor)?;

    let state_val = anchor.payload.get("state")?;

    // Try typed deserialization, fall back to raw JSON fields for old anchors.
    let (summary, next_steps) = match serde_json::from_value::<HandoffState>(state_val.clone()) {
        Ok(hs) => (
            hs.summary.unwrap_or_default(),
            hs.next_steps.unwrap_or_default(),
        ),
        Err(_) => (
            state_val
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            state_val
                .get("next_steps")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        ),
    };

    if summary.is_empty() && next_steps.is_empty() {
        return None;
    }

    let mut body = String::from("[Previous Context]\n");
    if !summary.is_empty() {
        body.push_str(&summary);
    }
    if !next_steps.is_empty() {
        if !summary.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("Next steps: ");
        body.push_str(&next_steps);
    }

    Some(Message::system(body))
}

/// Extract the `summary` field from the last anchor entry in the given slice.
///
/// This is used to retrieve the distilled knowledge summary from a user tape's
/// anchor state.  Returns `None` when there is no anchor or the anchor has no
/// summary.
pub fn anchor_summary_from_entries(entries: &[TapEntry]) -> Option<String> {
    let anchor = entries
        .iter()
        .rev()
        .find(|e| e.kind == TapEntryKind::Anchor)?;

    let state_val = anchor.payload.get("state")?;
    match serde_json::from_value::<HandoffState>(state_val.clone()) {
        Ok(hs) => hs.summary.filter(|s| !s.is_empty()),
        Err(_) => state_val
            .get("summary")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    }
}

// ---------------------------------------------------------------------------
// User tape context
// ---------------------------------------------------------------------------

/// Build a system-role message summarizing the user tape for LLM context
/// injection.
///
/// Reads all `Note` entries from the user tape and formats them into a single
/// system message.  Returns `None` when the user tape has no notes, so the
/// caller can skip injection entirely.
pub fn user_tape_context(entries: &[TapEntry], anchor_summary: Option<&str>) -> Option<Message> {
    let all_notes: Vec<&TapEntry> = entries
        .iter()
        .filter(|e| e.kind == TapEntryKind::Note)
        .collect();

    let total_notes = all_notes.len();

    // Apply hard safety cap — keep the most recent entries when the note count
    // exceeds the limit so we never blow up the model context window.
    let (notes, overflowed) = if total_notes > MAX_USER_NOTES_HARD_CAP {
        (&all_notes[total_notes - MAX_USER_NOTES_HARD_CAP..], true)
    } else {
        (&all_notes[..], false)
    };

    let mut sections: Vec<String> = Vec::new();

    if overflowed {
        sections.push(format!(
            "[Memory overflow: {total_notes} notes since last consolidation, showing most recent \
             {MAX_USER_NOTES_HARD_CAP}. Urgent distillation needed.]"
        ));
    }

    for entry in notes {
        let category = entry
            .payload
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("general");
        let content = entry
            .payload
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("");
        if content.is_empty() {
            continue;
        }
        // Include the date so the LLM can gauge information freshness.
        let date = entry.timestamp.strftime("%Y-%m-%d");
        sections.push(format!("- [{category}] ({date}) {content}"));
    }

    let has_summary = anchor_summary.is_some_and(|s| !s.is_empty());
    let has_notes = !sections.is_empty();

    if !has_summary && !has_notes {
        return None;
    }

    let mut body = String::from(
        "[User Memory]\nThe following notes were previously recorded about this user. Use them to \
         personalize your responses.\n",
    );

    if let Some(summary) = anchor_summary.filter(|s| !s.is_empty()) {
        body.push_str("\n[Distilled Knowledge]\n");
        body.push_str(summary);
        body.push('\n');
    }

    if has_notes {
        body.push_str("\n[Recent Notes]\n");
        body.push_str(&sections.join("\n"));
    }

    if total_notes > CONSOLIDATION_HINT_THRESHOLD {
        body.push_str(&format!(
            "\n[Memory Status: {total_notes} notes since last consolidation. Memory consolidation \
             may be needed soon.]"
        ));
    }

    Some(Message::system(body))
}

/// Merge all consecutive system-role messages at the front of the list into a
/// single system message.
///
/// Providers with strict chat templates (e.g. Qwen via llama.cpp) require
/// exactly one system message at position 0.  This function is safe for all
/// providers — the semantic content is preserved by joining with `\n\n---\n\n`.
pub fn merge_leading_system_messages(messages: Vec<Message>) -> Vec<Message> {
    let system_count = messages
        .iter()
        .take_while(|m| m.role == crate::llm::Role::System)
        .count();

    if system_count <= 1 {
        return messages;
    }

    let merged_text =
        messages[..system_count]
            .iter()
            .enumerate()
            .fold(String::new(), |mut acc, (i, m)| {
                debug_assert!(
                    matches!(m.content, MessageContent::Text(_)),
                    "merge_leading_system_messages only handles text content"
                );
                if i > 0 {
                    acc.push_str("\n\n---\n\n");
                }
                acc.push_str(m.content.as_text());
                acc
            });

    let mut result = Vec::with_capacity(messages.len() - system_count + 1);
    result.push(Message::system(merged_text));
    result.extend(messages.into_iter().skip(system_count));
    result
}

/// Prefix inserted when a non-leading system message is rewritten as a
/// user-role message to satisfy providers that reject mid-stream `system`
/// roles (e.g. MiniMax `invalid message role: system (2013)`).
const SYSTEM_NOTE_PREFIX: &str = "[system note] ";

/// Normalize the message list so that `Role::System` appears **only** at
/// position 0.
///
/// Rationale: some providers (notably MiniMax) reject any chat-completion
/// request that contains a `system` role after the first message with HTTP
/// 400 `invalid message role: system (2013)`. Tape-driven context rebuilds
/// can legitimately interleave system-role metadata (anchor summaries, user
/// memory, ad-hoc system notes) into the middle of the stream, so we must
/// rewrite any non-leading occurrence before dispatch.
///
/// Strategy:
/// 1. Collapse every leading system message (position 0..k) into a single
///    concatenated system message via [`merge_leading_system_messages`].
/// 2. Rewrite any remaining `Role::System` message into a `Role::User` message
///    prefixed with [`SYSTEM_NOTE_PREFIX`] so the semantic intent (out-of-band
///    instruction) is preserved without abusing the `system` role the provider
///    rejects.
///
/// The `user`-with-prefix form is a deliberate trade-off: an alternative
/// would be to silently merge mid-stream system content back into the
/// leading prompt, but that destroys the message's position in the
/// conversation timeline. Preserving position as a `user` turn keeps the
/// LLM's chronological reasoning intact.
pub fn collapse_system_messages(messages: Vec<Message>) -> Vec<Message> {
    let merged = merge_leading_system_messages(messages);
    merged
        .into_iter()
        .enumerate()
        .map(|(idx, msg)| {
            if idx > 0 && msg.role == crate::llm::Role::System {
                let text = msg.content.as_text().to_owned();
                Message::user(format!("{SYSTEM_NOTE_PREFIX}{text}"))
            } else {
                msg
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use serde_json::json;

    use super::*;
    use crate::llm::{MessageContent, Role};

    /// Helper to build a `TapEntry` with kind `Note`.
    fn note_entry(category: &str, content: &str, date: &str) -> TapEntry {
        TapEntry {
            id:        1,
            kind:      TapEntryKind::Note,
            payload:   json!({"category": category, "content": content}),
            timestamp: Timestamp::from(
                jiff::civil::date(
                    date[..4].parse().unwrap(),
                    date[5..7].parse().unwrap(),
                    date[8..10].parse().unwrap(),
                )
                .to_zoned(jiff::tz::TimeZone::UTC)
                .unwrap(),
            ),
            metadata:  None,
        }
    }

    #[test]
    fn user_tape_context_returns_none_for_empty_entries() {
        assert!(user_tape_context(&[], None).is_none());
    }

    #[test]
    fn user_tape_context_returns_none_for_non_note_entries() {
        let entry = TapEntry {
            id:        1,
            kind:      TapEntryKind::Message,
            payload:   json!({"role": "user", "content": "hello"}),
            timestamp: Timestamp::now(),
            metadata:  None,
        };
        assert!(user_tape_context(&[entry], None).is_none());
    }

    #[test]
    fn user_tape_context_skips_empty_content() {
        let entry = note_entry("fact", "", "2026-03-06");
        assert!(user_tape_context(&[entry], None).is_none());
    }

    #[test]
    fn user_tape_context_renders_with_timestamp() {
        let entry = note_entry("preference", "prefers dark mode", "2026-03-06");
        let msg = user_tape_context(&[entry], None).expect("should produce a message");
        assert_eq!(msg.role, Role::System);
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("[User Memory]"));
        assert!(text.contains("- [preference] (2026-03-06) prefers dark mode"));
    }

    #[test]
    fn user_tape_context_multiple_notes() {
        let entries = vec![
            note_entry("fact", "name is Alice", "2026-01-15"),
            note_entry("todo", "follow up on project", "2026-02-20"),
        ];
        let msg = user_tape_context(&entries, None).expect("should produce a message");
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("- [fact] (2026-01-15) name is Alice"));
        assert!(text.contains("- [todo] (2026-02-20) follow up on project"));
    }

    #[test]
    fn user_tape_context_renders_all_notes_without_truncation() {
        let entries: Vec<TapEntry> = (0..25)
            .map(|i| note_entry("fact", &format!("note {i}"), "2026-03-06"))
            .collect();
        let msg = user_tape_context(&entries, None).expect("should produce a message");
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        // All 25 notes should be present — no truncation.
        for i in 0..25 {
            assert!(text.contains(&format!("note {i}")), "missing note {i}");
        }
        assert!(!text.contains("Earlier notes omitted"));
    }

    #[test]
    fn user_tape_context_shows_consolidation_hint_above_threshold() {
        let entries: Vec<TapEntry> = (0..16)
            .map(|i| note_entry("fact", &format!("note {i}"), "2026-03-06"))
            .collect();
        let msg = user_tape_context(&entries, None).expect("should produce a message");
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("[Memory Status: 16 notes since last consolidation"));
    }

    #[test]
    fn user_tape_context_hard_cap_truncates_at_50() {
        let entries: Vec<TapEntry> = (0..60)
            .map(|i| note_entry("fact", &format!("note {i}"), "2026-03-06"))
            .collect();
        let msg = user_tape_context(&entries, None).expect("should produce a message");
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        // Overflow warning should be present.
        assert!(text.contains("Memory overflow: 60 notes"));
        assert!(text.contains("Urgent distillation needed"));
        // Oldest 10 notes (0..10) should be truncated.
        for i in 0..10 {
            // "note 0" through "note 9" must not appear — but careful: "note 0"
            // is a substring of "note 50" etc. Use the exact formatted line.
            assert!(
                !text.contains(&format!("fact] (2026-03-06) note {i}\n")),
                "note {i} should have been truncated"
            );
        }
        // Most recent 50 notes (10..60) should be present.
        for i in 10..60 {
            assert!(text.contains(&format!("note {i}")), "missing note {i}");
        }
        // Consolidation hint should also appear (60 > 15).
        assert!(text.contains("[Memory Status: 60 notes since last consolidation"));
    }

    #[test]
    fn user_tape_context_no_overflow_at_exactly_50() {
        let entries: Vec<TapEntry> = (0..50)
            .map(|i| note_entry("fact", &format!("note {i}"), "2026-03-06"))
            .collect();
        let msg = user_tape_context(&entries, None).expect("should produce a message");
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        assert!(!text.contains("Memory overflow"));
        // All 50 notes present.
        for i in 0..50 {
            assert!(text.contains(&format!("note {i}")), "missing note {i}");
        }
    }

    #[test]
    fn user_tape_context_no_consolidation_hint_at_threshold() {
        let entries: Vec<TapEntry> = (0..15)
            .map(|i| note_entry("fact", &format!("note {i}"), "2026-03-06"))
            .collect();
        let msg = user_tape_context(&entries, None).expect("should produce a message");
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        assert!(!text.contains("Memory Status"));
    }

    #[test]
    fn default_tape_context_reconstructs_messages() {
        let entries = vec![
            TapEntry {
                id:        1,
                kind:      TapEntryKind::Message,
                payload:   json!({"role": "user", "content": "hello"}),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        2,
                kind:      TapEntryKind::Message,
                payload:   json!({"role": "assistant", "content": "hi there"}),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
        ];
        let messages = default_tape_context(&entries).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].role, Role::Assistant);
    }

    #[test]
    fn default_tape_context_uses_matching_tool_call_id_from_result_payload() {
        let entries = vec![
            TapEntry {
                id:        1,
                kind:      TapEntryKind::ToolCall,
                payload:   json!({
                    "calls": [
                        {
                            "id": "call_first",
                            "function": {"name": "first_tool", "arguments": "{}"}
                        },
                        {
                            "id": "call_second",
                            "function": {"name": "second_tool", "arguments": "{}"}
                        }
                    ]
                }),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        2,
                kind:      TapEntryKind::ToolResult,
                payload:   json!({
                    "results": [
                        {
                            "tool_call_id": "call_second",
                            "ok": true
                        }
                    ]
                }),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
        ];

        let messages = default_tape_context(&entries).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, Role::Tool);
        assert_eq!(messages[1].tool_call_id.as_deref(), Some("call_second"));
    }

    #[test]
    fn default_tape_context_ignores_non_matching_tool_call_id_from_result_payload() {
        let entries = vec![
            TapEntry {
                id:        1,
                kind:      TapEntryKind::ToolCall,
                payload:   json!({
                    "calls": [{
                        "id": "call_first",
                        "function": {"name": "first_tool", "arguments": "{}"}
                    }]
                }),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        2,
                kind:      TapEntryKind::ToolResult,
                payload:   json!({
                    "results": [{
                        "tool_call_id": "payload_owned_by_tool",
                        "ok": true
                    }]
                }),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
        ];

        let messages = default_tape_context(&entries).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, Role::Tool);
        assert_eq!(messages[1].tool_call_id.as_deref(), Some("call_first"));
    }

    // -----------------------------------------------------------------------
    // anchor_context tests
    // -----------------------------------------------------------------------

    fn anchor_entry(state: serde_json::Value) -> TapEntry {
        TapEntry {
            id:        10,
            kind:      TapEntryKind::Anchor,
            payload:   json!({"name": "topic/done", "state": state}),
            timestamp: Timestamp::now(),
            metadata:  None,
        }
    }

    #[test]
    fn anchor_context_returns_none_for_no_anchors() {
        assert!(anchor_context(&[]).is_none());
    }

    #[test]
    fn anchor_context_returns_none_when_state_has_no_summary() {
        let entry = anchor_entry(json!({"owner": "human"}));
        assert!(anchor_context(&[entry]).is_none());
    }

    #[test]
    fn anchor_context_includes_summary() {
        let entry = anchor_entry(json!({"summary": "User logged into Immich"}));
        let msg = anchor_context(&[entry]).expect("should produce a message");
        assert_eq!(msg.role, Role::System);
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("[Previous Context]"));
        assert!(text.contains("User logged into Immich"));
    }

    #[test]
    fn anchor_context_includes_summary_and_next_steps() {
        let entry = anchor_entry(json!({
            "summary": "Took screenshot of Immich",
            "next_steps": "Send image via Telegram"
        }));
        let msg = anchor_context(&[entry]).expect("should produce a message");
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("Took screenshot of Immich"));
        assert!(text.contains("Next steps: Send image via Telegram"));
    }

    #[test]
    fn anchor_context_only_next_steps() {
        let entry = anchor_entry(json!({"next_steps": "Follow up tomorrow"}));
        let msg = anchor_context(&[entry]).expect("should produce a message");
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("Next steps: Follow up tomorrow"));
        // Should not contain double newlines when summary is absent.
        assert!(!text.contains("\n\n"));
    }

    // -----------------------------------------------------------------------
    // merge_leading_system_messages tests
    // -----------------------------------------------------------------------

    #[test]
    fn merge_leading_system_messages_single() {
        let messages = vec![Message::system("hello"), Message::user("hi")];
        let merged = merge_leading_system_messages(messages);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].role, Role::System);
        assert_eq!(merged[0].content.as_text(), "hello");
        assert_eq!(merged[1].role, Role::User);
    }

    #[test]
    fn merge_leading_system_messages_multiple() {
        let messages = vec![
            Message::system("system prompt"),
            Message::system("[Previous Context]\nSummary here"),
            Message::system("[User Memory]\nNotes here"),
            Message::user("hi"),
            Message::assistant("hello"),
        ];
        let merged = merge_leading_system_messages(messages);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].role, Role::System);
        assert!(merged[0].content.as_text().contains("system prompt"));
        assert!(merged[0].content.as_text().contains("[Previous Context]"));
        assert!(merged[0].content.as_text().contains("[User Memory]"));
        assert_eq!(merged[1].role, Role::User);
        assert_eq!(merged[2].role, Role::Assistant);
    }

    #[test]
    fn merge_leading_system_messages_no_system() {
        let messages = vec![Message::user("hi")];
        let merged = merge_leading_system_messages(messages);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].role, Role::User);
    }

    #[test]
    fn merge_leading_system_messages_empty() {
        let merged = merge_leading_system_messages(vec![]);
        assert!(merged.is_empty());
    }

    #[test]
    fn merge_leading_system_messages_all_system() {
        let messages = vec![Message::system("a"), Message::system("b")];
        let merged = merge_leading_system_messages(messages);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].content.as_text(), "a\n\n---\n\nb");
    }

    #[test]
    fn collapse_system_messages_merges_leading_and_rewrites_midstream() {
        let messages = vec![
            Message::system("prompt"),
            Message::system("[User Memory]\nnotes"),
            Message::user("hi"),
            Message::assistant("hello"),
            Message::system("mid-stream hint"),
            Message::user("bye"),
        ];
        let out = collapse_system_messages(messages);
        // One leading system, the rest mid-stream system has been rewritten.
        let system_count = out.iter().filter(|m| m.role == Role::System).count();
        assert_eq!(system_count, 1, "only position 0 should carry Role::System");
        assert_eq!(out[0].role, Role::System);
        assert!(out[0].content.as_text().contains("prompt"));
        assert!(out[0].content.as_text().contains("[User Memory]"));
        // Mid-stream system is now a user message with the prefix marker.
        let rewritten = out
            .iter()
            .find(|m| m.role == Role::User && m.content.as_text().starts_with("[system note] "));
        let rewritten = rewritten.expect("mid-stream system should be rewritten as user");
        assert!(rewritten.content.as_text().contains("mid-stream hint"));
    }

    #[test]
    fn collapse_system_messages_round_trip_no_midstream_system() {
        let messages = vec![
            Message::system("prompt"),
            Message::user("hi"),
            Message::assistant("hello"),
        ];
        let out = collapse_system_messages(messages.clone());
        assert_eq!(out.len(), messages.len());
        assert_eq!(out[0].role, Role::System);
        assert_eq!(out[1].role, Role::User);
        assert_eq!(out[2].role, Role::Assistant);
    }

    #[test]
    fn collapse_system_messages_empty() {
        assert!(collapse_system_messages(vec![]).is_empty());
    }

    #[test]
    fn collapse_system_messages_multiple_midstream() {
        let messages = vec![
            Message::system("prompt"),
            Message::user("hi"),
            Message::system("hint A"),
            Message::system("hint B"),
            Message::assistant("ok"),
        ];
        let out = collapse_system_messages(messages);
        let system_count = out.iter().filter(|m| m.role == Role::System).count();
        assert_eq!(system_count, 1);
        let rewritten: Vec<_> = out
            .iter()
            .filter(|m| m.role == Role::User && m.content.as_text().starts_with("[system note] "))
            .collect();
        assert_eq!(rewritten.len(), 2);
    }

    #[test]
    fn merge_leading_system_messages_does_not_merge_non_leading() {
        // System message after a user message should NOT be merged
        let messages = vec![
            Message::system("prompt"),
            Message::user("hi"),
            Message::system("injected"),
            Message::assistant("ok"),
        ];
        let merged = merge_leading_system_messages(messages);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].role, Role::System);
        assert_eq!(merged[0].content.as_text(), "prompt");
        assert_eq!(merged[2].role, Role::System);
    }

    #[test]
    fn anchor_context_uses_last_anchor() {
        let entries = vec![
            anchor_entry(json!({"summary": "old context"})),
            TapEntry {
                id:        11,
                kind:      TapEntryKind::Message,
                payload:   json!({"role": "user", "content": "hello"}),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
            TapEntry {
                id:        20,
                kind:      TapEntryKind::Anchor,
                payload:   json!({"name": "topic/new", "state": {"summary": "new context"}}),
                timestamp: Timestamp::now(),
                metadata:  None,
            },
        ];
        let msg = anchor_context(&entries).expect("should produce a message");
        let text = match &msg.content {
            MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("new context"));
        assert!(!text.contains("old context"));
    }
}
