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

use serde_json::Value;

use super::{TapEntry, TapEntryKind, TapResult};
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
