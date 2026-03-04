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

//! Conversion utilities between tape [`Value`] messages and kernel
//! [`ChatMessage`] types.
//!
//! The tape subsystem stores messages as raw JSON [`Value`]s. This module
//! provides [`tape_values_to_chat_messages`] for converting the output of
//! [`default_tape_context`](super::default_tape_context) into the kernel's
//! typed [`ChatMessage`] format.

use rara_kernel::channel::types::ChatMessage;
use serde_json::Value;

/// Convert tape context values (from [`default_tape_context`]) into kernel
/// [`ChatMessage`]s.
///
/// Each value is expected to be a JSON object with at least `role` and
/// `content` fields. Values that fail to deserialize are silently skipped
/// with a warning log.
pub fn tape_values_to_chat_messages(values: Vec<Value>) -> Vec<ChatMessage> {
    values
        .into_iter()
        .filter_map(|value| match serde_json::from_value::<ChatMessage>(value) {
            Ok(msg) => Some(msg),
            Err(e) => {
                tracing::warn!("skipping tape entry that failed ChatMessage conversion: {e}");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_convert_user_message() {
        let values = vec![json!({
            "role": "user",
            "content": "hello",
            "created_at": "2025-01-01T00:00:00Z",
        })];

        let messages = tape_values_to_chat_messages(values);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_text(), "hello");
    }

    #[test]
    fn test_convert_assistant_message() {
        let values = vec![json!({
            "role": "assistant",
            "content": "hi there",
            "created_at": "2025-01-01T00:00:00Z",
        })];

        let messages = tape_values_to_chat_messages(values);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_text(), "hi there");
    }

    #[test]
    fn test_convert_tool_message() {
        let values = vec![json!({
            "role": "tool",
            "content": "{\"result\": 42}",
            "tool_call_id": "call_abc",
            "name": "calculator",
            "created_at": "2025-01-01T00:00:00Z",
        })];

        let messages = tape_values_to_chat_messages(values);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tool_call_id.as_deref(), Some("call_abc"));
        assert_eq!(messages[0].tool_name.as_deref(), Some("calculator"));
    }

    #[test]
    fn test_skip_invalid_values() {
        let values = vec![
            json!({
                "role": "user",
                "content": "valid",
                "created_at": "2025-01-01T00:00:00Z",
            }),
            json!("not an object"),
            json!({"missing_role": true}),
        ];

        let messages = tape_values_to_chat_messages(values);
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_empty_input() {
        let messages = tape_values_to_chat_messages(vec![]);
        assert!(messages.is_empty());
    }
}
