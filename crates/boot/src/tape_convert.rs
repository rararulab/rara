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
//! [`default_tape_context`](rara_memory::tape::default_tape_context) into the
//! kernel's typed [`ChatMessage`] format.
//!
//! Moved from `rara-memory` to `rara-boot` to break the circular dependency
//! between `rara-memory` and `rara-kernel`.

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
