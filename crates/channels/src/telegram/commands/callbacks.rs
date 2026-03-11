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
//! - [`SessionSwitchCallbackHandler`]: handles `switch:{session_key}`
//!   callbacks.
//! - [`SearchPaginationCallbackHandler`]: handles
//!   `search_more:{count}:{params}` callbacks.

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
// Helpers
// ---------------------------------------------------------------------------

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
