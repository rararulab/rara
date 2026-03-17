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

//! Callback handlers for inline keyboard interactions.
//!
//! - [`SessionSwitchCallbackHandler`] — handles `switch:{session_key}`
//!   callbacks.
//! - [`SessionDetailCallbackHandler`] — handles `detail:{session_key}`
//!   callbacks.
//! - Search pagination callbacks handle `search_more:{count}:{params}`.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{CallbackContext, CallbackHandler, CallbackResult},
    error::KernelError,
};

use super::client::BotServiceClient;

// ---------------------------------------------------------------------------
// SessionSwitchCallbackHandler
// ---------------------------------------------------------------------------

/// Handles `switch:{session_key}` callback queries from the `/sessions`
/// inline keyboard.
pub struct SessionSwitchCallbackHandler {
    client: Arc<dyn BotServiceClient>,
}

impl SessionSwitchCallbackHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CallbackHandler for SessionSwitchCallbackHandler {
    fn prefix(&self) -> &str { "switch:" }

    async fn handle(&self, context: &CallbackContext) -> Result<CallbackResult, KernelError> {
        let session_key = &context.data["switch:".len()..];
        let chat_id = super::extract_chat_id(&context.metadata)?;

        match self
            .client
            .bind_channel("telegram", &chat_id, session_key)
            .await
        {
            Ok(_) => Ok(CallbackResult::SendMessage {
                text: format!(
                    "Switched to session: <code>{}</code>",
                    html_escape(session_key)
                ),
            }),
            Err(e) => Ok(CallbackResult::SendMessage {
                text: format!("Failed to switch session: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// SessionDetailCallbackHandler
// ---------------------------------------------------------------------------

/// Handles `detail:{session_key}` callback queries from the `/sessions`
/// inline keyboard — shows session details for the currently active session.
pub struct SessionDetailCallbackHandler {
    client: Arc<dyn BotServiceClient>,
}

impl SessionDetailCallbackHandler {
    /// Create a new handler backed by the given service client.
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CallbackHandler for SessionDetailCallbackHandler {
    fn prefix(&self) -> &str { "detail:" }

    async fn handle(&self, context: &CallbackContext) -> Result<CallbackResult, KernelError> {
        let session_key = &context.data["detail:".len()..];

        match self.client.get_session(session_key).await {
            Ok(detail) => {
                let title = detail.title.as_deref().unwrap_or("Untitled");
                let model = detail.model.as_deref().unwrap_or("(default)");
                let text = format!(
                    "<b>{}</b>\nKey: <code>{}</code>\nModel: {}\nCreated: {}\nLast active: {}",
                    html_escape(title),
                    html_escape(&detail.key),
                    html_escape(model),
                    format_timestamp(&detail.created_at),
                    format_timestamp(&detail.updated_at),
                );
                Ok(CallbackResult::SendMessage { text })
            }
            Err(e) => Ok(CallbackResult::SendMessage {
                text: format!("Failed to get session details: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Format an ISO-8601 timestamp into a compact `YYYY-MM-DD HH:MM` form.
fn format_timestamp(raw: &str) -> String {
    if raw.len() >= 16 {
        let date_part = &raw[..10];
        let time_part = &raw[11..16];
        if !time_part.is_empty() {
            return format!("{date_part} {time_part}");
        }
    }
    raw.to_owned()
}
