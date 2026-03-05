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

//! Chat domain service — session/message/model management.
//!
//! [`SessionService`] is the primary entry point for all chat CRUD operations.
//! It holds references to the session index and tape store, and exposes
//! high-level methods for session management, model catalog queries, and
//! channel bindings.
//!
//! Session metadata is managed by [`SessionIndex`]. Message persistence has
//! moved to the tape subsystem via [`TapeService`].

use std::sync::Arc;

use chrono::Utc;
use rara_domain_shared::settings::{SettingsProvider, keys};
use rara_kernel::session::SessionIndexRef;
use rara_kernel::memory::TapeService;
use rara_sessions::types::{ChannelBinding, SessionEntry, SessionKey};
use tracing::{info, instrument};

use crate::chat::{
    error::ChatError,
    model_catalog::{ChatModel, ModelCatalog},
};

/// Central orchestrator for session-based AI chat.
///
/// `SessionService` ties together two concerns:
///
/// 1. **Session metadata** — CRUD operations on sessions and channel bindings,
///    delegated to a [`SessionIndex`] implementation.
/// 2. **Channel routing** — Mapping external messaging channels to internal
///    session keys via channel bindings.
///
/// Message persistence is handled by the tape subsystem ([`TapeService`]).
/// LLM execution has moved to the kernel path (`process_loop`).
///
/// The service is cheaply cloneable (`Arc`-wrapped internals) and safe to
/// share across axum handler tasks.
#[derive(Clone)]
pub struct SessionService {
    /// Tape-based session index for metadata.
    session_index:     SessionIndexRef,
    /// Tape service for append-only session recording.
    tape_service:      TapeService,
    /// Cached catalog of models fetched from OpenRouter.
    model_catalog:     ModelCatalog,
    /// Settings provider for reading and writing flat KV settings.
    settings_provider: Arc<dyn SettingsProvider>,
}

impl SessionService {
    /// Create a new chat service with the given dependencies.
    #[must_use]
    pub fn new(
        session_index: SessionIndexRef,
        tape_service: TapeService,
        settings_provider: Arc<dyn SettingsProvider>,
    ) -> Self {
        Self {
            session_index,
            tape_service,
            model_catalog: ModelCatalog::new(),
            settings_provider,
        }
    }

    // -- model catalog ------------------------------------------------------

    /// List available models, dynamically fetching from OpenRouter when an
    /// API key is configured. Favorites are marked and sorted to the top.
    pub async fn list_models(&self) -> Vec<ChatModel> {
        let api_key = self
            .settings_provider
            .get(keys::LLM_PROVIDERS_OPENROUTER_API_KEY)
            .await;
        let favorites_json = self.settings_provider.get(keys::LLM_FAVORITE_MODELS).await;
        let favorites: Vec<String> = favorites_json
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        self.model_catalog
            .list_models(api_key.as_deref(), &favorites)
            .await
    }

    /// Replace the user's favorite model list and persist to settings.
    pub async fn set_favorite_models(&self, ids: Vec<String>) -> Result<(), ChatError> {
        let json = serde_json::to_string(&ids).unwrap_or_default();
        self.settings_provider
            .set(keys::LLM_FAVORITE_MODELS, &json)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to update favorite models: {e}"),
            })?;
        Ok(())
    }

    // -- session CRUD -------------------------------------------------------

    /// Create a new session with optional overrides.
    ///
    /// The session key is generated (UUID).
    #[instrument(skip(self))]
    pub async fn create_session(
        &self,
        title: Option<String>,
        model: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<SessionEntry, ChatError> {
        let now = Utc::now();
        let entry = SessionEntry {
            key: SessionKey::new(),
            title,
            model,
            system_prompt,
            message_count: 0,
            preview: None,
            metadata: None,
            created_at: now,
            updated_at: now,
        };
        let created = self.session_index.create_session(&entry).await?;
        info!(key = %created.key, "session created");
        Ok(created)
    }

    /// List sessions ordered by most recently updated, with pagination.
    #[instrument(skip(self))]
    pub async fn list_sessions(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<SessionEntry>, ChatError> {
        let sessions = self
            .session_index
            .list_sessions(limit.unwrap_or(50), offset.unwrap_or(0))
            .await?;
        Ok(sessions)
    }

    /// Get a single session by key. Returns [`ChatError::SessionNotFound`]
    /// if the key does not exist.
    #[instrument(skip(self))]
    pub async fn get_session(&self, key: &SessionKey) -> Result<SessionEntry, ChatError> {
        self.session_index
            .get_session(key)
            .await?
            .ok_or_else(|| ChatError::SessionNotFound {
                key: key.to_string(),
            })
    }

    /// Partially update mutable fields of a session.
    ///
    /// Only the fields that are `Some` in the arguments are overwritten; the
    /// rest are left untouched. Returns the updated [`SessionEntry`].
    #[instrument(skip(self))]
    pub async fn update_session_fields(
        &self,
        key: &SessionKey,
        title: Option<String>,
        model: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<SessionEntry, ChatError> {
        let mut session = self.get_session(key).await?;
        if let Some(t) = title {
            session.title = Some(t);
        }
        if let Some(m) = model {
            session.model = Some(m);
        }
        if let Some(sp) = system_prompt {
            session.system_prompt = Some(sp);
        }
        session.updated_at = Utc::now();
        let updated = self.session_index.update_session(&session).await?;
        info!(key = %key, "session fields updated");
        Ok(updated)
    }

    /// Delete a session.
    #[instrument(skip(self))]
    pub async fn delete_session(&self, key: &SessionKey) -> Result<(), ChatError> {
        self.session_index.delete_session(key).await?;
        info!(key = %key, "session deleted");
        Ok(())
    }

    // -- ensure session -----------------------------------------------------

    /// Ensure a session exists, creating it if it does not.
    ///
    /// Returns the existing or newly created [`SessionEntry`].
    #[instrument(skip(self))]
    pub async fn ensure_session(
        &self,
        key: &SessionKey,
        title: Option<&str>,
        model: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Result<SessionEntry, ChatError> {
        match self.session_index.get_session(key).await? {
            Some(existing) => Ok(existing),
            None => {
                let now = Utc::now();
                let entry = SessionEntry {
                    key:           *key,
                    title:         title.map(ToOwned::to_owned),
                    model:         model.map(ToOwned::to_owned),
                    system_prompt: system_prompt.map(ToOwned::to_owned),
                    message_count: 0,
                    preview:       None,
                    metadata:      None,
                    created_at:    now,
                    updated_at:    now,
                };
                Ok(self.session_index.create_session(&entry).await?)
            }
        }
    }

    // -- channel bindings ---------------------------------------------------

    /// Bind an external channel (e.g. Telegram chat) to a session key.
    #[instrument(skip(self))]
    pub async fn bind_channel(
        &self,
        channel_type: String,
        account: String,
        chat_id: String,
        session_key: SessionKey,
    ) -> Result<ChannelBinding, ChatError> {
        let now = Utc::now();
        let binding = ChannelBinding {
            channel_type,
            account,
            chat_id,
            session_key,
            created_at: now,
            updated_at: now,
        };
        let result = self.session_index.bind_channel(&binding).await?;
        Ok(result)
    }

    /// Look up which session an external channel is currently bound to.
    #[instrument(skip(self))]
    pub async fn get_channel_session(
        &self,
        channel_type: &str,
        account: &str,
        chat_id: &str,
    ) -> Result<Option<ChannelBinding>, ChatError> {
        let binding = self
            .session_index
            .get_channel_binding(channel_type, account, chat_id)
            .await?;
        Ok(binding)
    }
}

impl std::fmt::Debug for SessionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionService").finish_non_exhaustive()
    }
}
