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
//! Session metadata is managed by [`SessionIndexRef`]. Message persistence has
//! moved to the tape subsystem via [`TapeService`].

use std::sync::Arc;

use chrono::Utc;
use rara_domain_shared::settings::{SettingsProvider, keys};
use rara_kernel::{
    cascade::{
        CascadeTrace, build_cascade, find_turn_boundaries, load_persisted_cascade, turn_slice,
    },
    channel::types::{ChatMessage, MessageContent, MessageRole, ToolCall as ChannelToolCall},
    llm::{Message, Role},
    memory::{TapEntry, TapEntryKind, TapeService},
    session::SessionIndexRef,
};
use rara_sessions::types::{ChannelBinding, SessionEntry, SessionKey};
use serde_json::Value;
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
///    delegated to a [`SessionIndexRef`] implementation.
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
    /// Cached catalog of models fetched from the LLM provider.
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
        model_lister: rara_kernel::llm::LlmModelListerRef,
    ) -> Self {
        Self {
            session_index,
            tape_service,
            model_catalog: ModelCatalog::new(model_lister),
            settings_provider,
        }
    }

    // -- model catalog ------------------------------------------------------

    /// List available models from the configured provider. Favorites are
    /// marked and sorted to the top.
    pub async fn list_models(&self) -> Vec<ChatModel> {
        let favorites_json = self.settings_provider.get(keys::LLM_FAVORITE_MODELS).await;
        let favorites: Vec<String> = favorites_json
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        self.model_catalog.list_models(&favorites).await
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

    // -- messages (tape-backed) ---------------------------------------------

    /// List conversational messages for a session by reading tape entries.
    ///
    /// Only `Message`, `ToolCall`, and `ToolResult` entries are converted to
    /// [`ChatMessage`] structs. Non-conversational kinds are skipped.
    #[instrument(skip(self))]
    pub async fn list_messages(
        &self,
        key: &SessionKey,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, ChatError> {
        let tape_name = key.to_string();
        let entries =
            self.tape_service
                .entries(&tape_name)
                .await
                .map_err(|e| ChatError::SessionError {
                    message: format!("failed to read tape: {e}"),
                })?;

        let messages = tap_entries_to_chat_messages(&entries);
        let total = messages.len();
        // Return the last `limit` messages (most recent).
        let start = total.saturating_sub(limit);
        Ok(messages[start..].to_vec())
    }

    /// Clear all tape entries for a session (reset the tape).
    #[instrument(skip(self))]
    pub async fn clear_messages(&self, key: &SessionKey) -> Result<(), ChatError> {
        let tape_name = key.to_string();
        self.tape_service
            .reset(&tape_name, false)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to clear tape: {e}"),
            })?;
        info!(key = %key, "messages cleared");
        Ok(())
    }

    // -- cascade trace ------------------------------------------------------

    /// Build a cascade execution trace for a specific turn in the session.
    ///
    /// The `message_seq` identifies the user message that starts the turn.
    /// All entries from that user message until the next user message (or
    /// end of tape) are collected and passed to the cascade builder.
    #[instrument(skip(self))]
    pub async fn get_cascade_trace(
        &self,
        key: &SessionKey,
        message_seq: usize,
    ) -> Result<CascadeTrace, ChatError> {
        let tape_name = key.to_string();
        let entries =
            self.tape_service
                .entries(&tape_name)
                .await
                .map_err(|e| ChatError::SessionError {
                    message: format!("failed to read tape: {e}"),
                })?;

        // Convert tape → chat messages so we can map the 1-based message_seq
        // back to the owning user-message turn.  The seq values in ChatMessage
        // can skip numbers (e.g. a ToolResult with N results increments seq
        // by N), so a direct index into tape entries is unreliable.
        let chat_msgs = tap_entries_to_chat_messages(&entries);

        let i_seq = message_seq as i64;
        // Find the last user message whose seq <= the clicked message_seq.
        let owning_user = chat_msgs
            .iter()
            .rfind(|m| m.role == MessageRole::User && m.seq <= i_seq);

        let Some(owner) = owning_user else {
            return Err(ChatError::InvalidRequest {
                message: format!("no user message found for seq {message_seq}"),
            });
        };

        // Determine the 0-based ordinal of this user message.
        let user_ordinal = chat_msgs
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .position(|m| m.seq == owner.seq)
            .unwrap_or(0);

        // Extract the turn slice, then try the pre-built trace before
        // falling back to the post-hoc builder.
        let boundaries = find_turn_boundaries(&entries);
        let turn_entries = turn_slice(&entries, &boundaries, user_ordinal);

        if let Some(trace) = load_persisted_cascade(turn_entries) {
            return Ok(trace);
        }

        let message_id = format!("{}-{}", key, message_seq);
        let trace = build_cascade(turn_entries, &message_id);
        Ok(trace)
    }

    // -- channel bindings ---------------------------------------------------

    /// Bind an external channel (e.g. Telegram chat) to a session key.
    #[instrument(skip(self))]
    pub async fn bind_channel(
        &self,
        channel_type: String,
        chat_id: String,
        session_key: SessionKey,
    ) -> Result<ChannelBinding, ChatError> {
        let now = Utc::now();
        let binding = ChannelBinding {
            channel_type,
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
        chat_id: &str,
    ) -> Result<Option<ChannelBinding>, ChatError> {
        let binding = self
            .session_index
            .get_channel_binding(channel_type, chat_id)
            .await?;
        Ok(binding)
    }
}

impl std::fmt::Debug for SessionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionService").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Tape → ChatMessage conversion
// ---------------------------------------------------------------------------

/// Convert tape entries into a flat list of [`ChatMessage`] structs.
///
/// Mirrors the logic in `memory/context.rs` but targets the channel-layer
/// `ChatMessage` type instead of `llm::Message`.
fn tap_entries_to_chat_messages(entries: &[TapEntry]) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    let mut seq: i64 = 0;
    let mut pending_calls: Vec<(String, String)> = Vec::new(); // (id, name)

    for entry in entries {
        match entry.kind {
            TapEntryKind::Message => {
                if let Ok(msg) = serde_json::from_value::<Message>(entry.payload.clone()) {
                    seq += 1;
                    let role = match msg.role {
                        Role::System | Role::Developer => MessageRole::System,
                        Role::User => MessageRole::User,
                        Role::Assistant => MessageRole::Assistant,
                        Role::Tool => MessageRole::Tool,
                    };
                    let content = MessageContent::Text(msg.content.as_text().to_owned());
                    let tool_calls: Vec<ChannelToolCall> = msg
                        .tool_calls
                        .iter()
                        .map(|tc| ChannelToolCall {
                            id:        tc.id.clone().into(),
                            name:      tc.name.clone().into(),
                            arguments: serde_json::from_str(&tc.arguments)
                                .unwrap_or(Value::String(tc.arguments.clone())),
                        })
                        .collect();
                    messages.push(ChatMessage {
                        seq,
                        role,
                        content,
                        tool_calls,
                        tool_call_id: msg.tool_call_id.clone(),
                        tool_name: None,
                        created_at: entry.timestamp,
                    });
                }
            }
            TapEntryKind::ToolCall => {
                pending_calls.clear();
                if let Some(calls) = entry.payload.get("calls").and_then(Value::as_array) {
                    let mut tc_list = Vec::new();
                    for call in calls {
                        let id = call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let func = call.get("function").and_then(Value::as_object);
                        let name = func
                            .and_then(|f| f.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let arguments = func
                            .and_then(|f| f.get("arguments"))
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let args_val: Value = serde_json::from_str(arguments)
                            .unwrap_or(Value::String(arguments.to_owned()));
                        pending_calls.push((id.clone(), name.clone()));
                        tc_list.push(ChannelToolCall {
                            id:        id.into(),
                            name:      name.into(),
                            arguments: args_val,
                        });
                    }
                    seq += 1;
                    messages.push(ChatMessage {
                        seq,
                        role: MessageRole::Assistant,
                        content: MessageContent::Text(String::new()),
                        tool_calls: tc_list,
                        tool_call_id: None,
                        tool_name: None,
                        created_at: entry.timestamp,
                    });
                }
            }
            TapEntryKind::ToolResult => {
                if let Some(results) = entry.payload.get("results").and_then(Value::as_array) {
                    for (i, result) in results.iter().enumerate() {
                        let content_str = match result {
                            Value::String(s) => s.clone(),
                            other => serde_json::to_string(other).unwrap_or_default(),
                        };
                        let (call_id, tool_name) =
                            pending_calls.get(i).cloned().unwrap_or_default();
                        seq += 1;
                        messages.push(ChatMessage {
                            seq,
                            role: MessageRole::ToolResult,
                            content: MessageContent::Text(content_str),
                            tool_calls: Vec::new(),
                            tool_call_id: if call_id.is_empty() {
                                None
                            } else {
                                Some(call_id)
                            },
                            tool_name: if tool_name.is_empty() {
                                None
                            } else {
                                Some(tool_name)
                            },
                            created_at: entry.timestamp,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    messages
}
