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

use super::{HandoffState, TapEntry, TapEntryKind, TapResult};
use crate::llm::{Message, ToolCallRequest};

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
        messages.push(Message::assistant_with_tool_calls("", tool_calls));
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
        let call_id = pending_calls
            .get(index)
            .map(|c| c.id.as_str())
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
        other => {
            serde_json::to_string(other).map_err(|source| super::TapError::JsonEncode { source })?
        }
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
pub fn user_tape_context(
    entries: &[TapEntry],
    anchor_summary: Option<&str>,
) -> Option<Message> {
    let notes: Vec<&TapEntry> = entries
        .iter()
        .filter(|e| e.kind == TapEntryKind::Note)
        .collect();

    let mut sections: Vec<String> = Vec::new();

    for entry in &notes {
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

    Some(Message::system(body))
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
