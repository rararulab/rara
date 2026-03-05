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

//! Default identity and session resolvers for the I/O Bus pipeline.
//!
//! The [`DefaultIdentityResolver`] always resolves to the single owner
//! user, regardless of channel or platform identity.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    channel::types::ChannelType,
    io::{IOError, IdentityResolver, SessionResolver},
    process::principal::UserId,
    session::{SessionIndex, SessionKey},
};
use tracing::debug;

// ---------------------------------------------------------------------------
// DefaultIdentityResolver
// ---------------------------------------------------------------------------

/// Identity resolver that always returns the single owner user.
///
/// In single-owner mode, all channels (Web, Telegram, CLI) resolve to the
/// same kernel user.
pub struct DefaultIdentityResolver {
    owner_user_id: UserId,
}

impl DefaultIdentityResolver {
    /// Create a new resolver for the given owner.
    pub fn new(owner_user_id: UserId) -> Self { Self { owner_user_id } }
}

#[async_trait]
impl IdentityResolver for DefaultIdentityResolver {
    async fn resolve(
        &self,
        channel_type: ChannelType,
        platform_user_id: &str,
        _platform_chat_id: Option<&str>,
    ) -> Result<UserId, IOError> {
        debug!(
            channel = %channel_type,
            platform_user_id,
            resolved_user = %self.owner_user_id.0,
            "identity resolved to owner"
        );
        Ok(self.owner_user_id.clone())
    }
}

// ---------------------------------------------------------------------------
// DefaultSessionResolver
// ---------------------------------------------------------------------------

/// Session resolver that first consults [`ChannelBinding`]s in the session
/// index and falls back to the raw `platform_chat_id` when no binding exists.
///
/// This allows the Telegram `/new` and `/sessions` commands to redirect
/// subsequent messages to the correct session after re-binding.
pub struct DefaultSessionResolver {
    session_index: Arc<dyn SessionIndex>,
}

impl DefaultSessionResolver {
    /// Create a new resolver backed by the given session index.
    pub fn new(session_index: Arc<dyn SessionIndex>) -> Self { Self { session_index } }
}

#[async_trait]
impl SessionResolver for DefaultSessionResolver {
    async fn resolve(
        &self,
        _user: &UserId,
        channel_type: ChannelType,
        platform_chat_id: Option<&str>,
    ) -> Result<SessionKey, IOError> {
        let chat_id = platform_chat_id.unwrap_or("default");

        // Try channel binding lookup first (honours /new and /sessions switch).
        if let Some(chat_id_str) = platform_chat_id {
            let ch_label = channel_type.to_string();
            match self
                .session_index
                .get_binding_by_chat(&ch_label, chat_id_str)
                .await
            {
                Ok(Some(binding)) => {
                    debug!(
                        channel = %channel_type,
                        chat_id = chat_id_str,
                        bound_session = %binding.session_key,
                        "session resolved via channel binding"
                    );
                    return Ok(binding.session_key);
                }
                Ok(None) => {
                    // No binding — fall through to raw chat_id.
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        channel = %channel_type,
                        chat_id = chat_id_str,
                        "channel binding lookup failed, falling back to raw chat_id"
                    );
                }
            }
        }

        // No binding found — create a new session and bind it.
        let now = chrono::Utc::now();
        let entry = rara_kernel::session::SessionEntry {
            key:           rara_kernel::session::SessionKey::new(),
            title:         None,
            model:         None,
            system_prompt: None,
            message_count: 0,
            preview:       None,
            metadata:      None,
            created_at:    now,
            updated_at:    now,
        };
        let session = self
            .session_index
            .create_session(&entry)
            .await
            .map_err(|e| IOError::Internal {
                message: format!("failed to create session: {e}"),
            })?;
        let ch_label = channel_type.to_string();
        if let Err(e) = self
            .session_index
            .bind_channel(&rara_kernel::session::ChannelBinding {
                channel_type: ch_label,
                account:      String::new(),
                chat_id:      chat_id.to_string(),
                session_key:  session.key,
                created_at:   now,
                updated_at:   now,
            })
            .await
        {
            tracing::warn!(error = %e, "failed to bind new session to channel");
        }
        Ok(session.key)
    }
}
