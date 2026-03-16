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

//! Shared notification helper for pushing messages to the TG notification
//! channel.

use rara_kernel::{event::KernelEventEnvelope, tool::ToolContext};

/// Push a notification message to the configured Telegram notification channel.
///
/// Silently does nothing if the event queue is unavailable (e.g. in tests).
pub fn push_notification(context: &ToolContext, message: impl Into<String>) {
    let _ = context
        .event_queue
        .try_push(KernelEventEnvelope::send_notification(message.into()));
}
