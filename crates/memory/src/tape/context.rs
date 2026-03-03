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

use serde_json::{Map, Value, json};

use super::{TapEntry, TapEntryKind, TapResult};

/// Reconstruct chat messages from persisted tape entries.
///
/// The reconstruction mirrors Bub's behavior:
/// - raw `message` entries are forwarded as-is,
/// - `tool_call` entries become an assistant message with `tool_calls`,
/// - `tool_result` entries become one or more tool-role messages.
pub fn default_tape_context(entries: &[TapEntry]) -> TapResult<Vec<Value>> {
    let mut messages = Vec::new();
    let mut pending_calls: Vec<Value> = Vec::new();

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

fn append_message_entry(messages: &mut Vec<Value>, entry: &TapEntry) {
    if let Some(payload) = entry.payload.as_object() {
        messages.push(Value::Object(payload.clone()));
    }
}

/// Convert a persisted tool-call entry into the assistant message shape used by
/// chat context reconstruction.
fn append_tool_call_entry(messages: &mut Vec<Value>, entry: &TapEntry) -> Vec<Value> {
    let calls = normalize_tool_calls(entry.payload.get("calls"));
    if !calls.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": "",
            "tool_calls": calls,
        }));
    }
    calls
}

/// Expand a persisted tool-result entry into one tool-role message per result.
fn append_tool_result_entry(
    messages: &mut Vec<Value>,
    pending_calls: &[Value],
    entry: &TapEntry,
) -> TapResult<()> {
    let Some(results) = entry.payload.get("results").and_then(Value::as_array) else {
        return Ok(());
    };

    for (index, result) in results.iter().enumerate() {
        messages.push(build_tool_result_message(result, pending_calls, index)?);
    }

    Ok(())
}

/// Build one tool-role message, attaching tool-call metadata when available.
fn build_tool_result_message(
    result: &Value,
    pending_calls: &[Value],
    index: usize,
) -> TapResult<Value> {
    let mut message = Map::new();
    message.insert("role".to_owned(), Value::String("tool".to_owned()));
    message.insert("content".to_owned(), render_tool_result(result)?);

    let Some(call) = pending_calls.get(index).and_then(Value::as_object) else {
        return Ok(Value::Object(message));
    };

    if let Some(call_id) = call.get("id").and_then(Value::as_str) {
        if !call_id.is_empty() {
            message.insert("tool_call_id".to_owned(), Value::String(call_id.to_owned()));
        }
    }

    if let Some(name) = call
        .get("function")
        .and_then(Value::as_object)
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
    {
        if !name.is_empty() {
            message.insert("name".to_owned(), Value::String(name.to_owned()));
        }
    }

    Ok(Value::Object(message))
}

/// Retain only well-formed object values from a tool-call payload.
fn normalize_tool_calls(value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.is_object())
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Render a tool result payload into the string content shape expected by chat
/// messages.
fn render_tool_result(result: &Value) -> TapResult<Value> {
    Ok(match result {
        Value::String(text) => Value::String(text.clone()),
        other => Value::String(
            serde_json::to_string(other)
                .map_err(|source| super::TapError::JsonEncode { source })?,
        ),
    })
}
