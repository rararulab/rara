// Copyright 2025 Crrow
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

//! Chat domain service — session-based conversations backed by LLM agents.
//!
//! [`ChatService`] is the primary entry point for all chat operations. It
//! holds references to the session repository, LLM provider, and tool
//! registry, and exposes high-level methods for session management and
//! message exchange.

use std::sync::Arc;

use chrono::Utc;
use rara_agents::{
    builtin::chat::ChatAgent,
    runner::UserContent,
    tool_registry::ToolRegistry,
};
use rara_sessions::{
    repository::SessionRepository,
    types::{
        ChannelBinding, ChatMessage, ContentBlock, MessageContent, MessageRole, SessionEntry,
        SessionKey,
    },
};
use tokio::sync::mpsc;
use tracing::{info, instrument};

use crate::stream::ChatStreamEvent;

use crate::{
    error::ChatError,
    model_catalog::{ChatModel, ModelCatalog},
};

/// Central orchestrator for session-based AI chat.
///
/// `ChatService` ties together three concerns:
///
/// 1. **Session persistence** — CRUD operations on sessions and messages,
///    delegated to a [`SessionRepository`] implementation.
/// 2. **LLM execution** — Building and running an [`AgentRunner`] with the
///    session's conversation history and registered tools.
/// 3. **Channel routing** — Mapping external messaging channels to internal
///    session keys via channel bindings.
///
/// The service is cheaply cloneable (`Arc`-wrapped internals) and safe to
/// share across axum handler tasks.
#[derive(Clone)]
pub struct ChatService {
    /// Persistence layer for sessions, messages, and channel bindings.
    session_repo:  Arc<dyn SessionRepository>,
    /// Cached catalog of models fetched from OpenRouter.
    model_catalog: ModelCatalog,
    /// Settings service for persisting favorite models.
    settings_svc:  rara_domain_shared::settings::SettingsSvc,
    /// Built-in chat agent — encapsulates prompt assembly, tool construction,
    /// context compaction, and memory reflection.
    chat_agent:    ChatAgent,
}

impl ChatService {
    /// Create a new chat service with the given dependencies.
    ///
    /// The `chat_agent` encapsulates the orchestrator and handles LLM
    /// provider access, tool management, prompt assembly, and memory
    /// reflection. The `settings_svc` is used for persisting user
    /// preferences such as favorite models.
    #[must_use]
    pub fn new(
        session_repo: Arc<dyn SessionRepository>,
        settings_svc: rara_domain_shared::settings::SettingsSvc,
        chat_agent: ChatAgent,
    ) -> Self {
        Self {
            session_repo,
            model_catalog: ModelCatalog::new(),
            settings_svc,
            chat_agent,
        }
    }

    /// Read the current default model from runtime settings.
    fn current_default_model(&self) -> String {
        self.chat_agent.orchestrator().current_default_model()
    }

    /// Read the current system prompt from runtime settings, falling back
    /// to a built-in default when no custom prompt is configured.
    fn current_system_prompt(&self) -> String {
        self.chat_agent.orchestrator().current_system_prompt()
    }

    // -- model catalog ------------------------------------------------------

    /// List available models, dynamically fetching from OpenRouter when an
    /// API key is configured. Favorites are marked and sorted to the top.
    pub async fn list_models(&self) -> Vec<ChatModel> {
        let settings = self.chat_agent.orchestrator().settings();
        let api_key = settings.ai.openrouter_api_key.as_deref();
        let favorites = &settings.ai.favorite_models;
        self.model_catalog.list_models(api_key, favorites).await
    }

    /// Replace the user's favorite model list and persist to settings.
    pub async fn set_favorite_models(&self, ids: Vec<String>) -> Result<(), ChatError> {
        use rara_domain_shared::settings::model::{AiRuntimeSettingsPatch, UpdateRequest};

        let patch = UpdateRequest {
            ai:           Some(AiRuntimeSettingsPatch {
                favorite_models: Some(ids),
                ..Default::default()
            }),
            telegram:     None,
            agent:        None,
            job_pipeline: None,
        };
        self.settings_svc
            .update(patch)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to update favorite models: {e}"),
            })?;
        Ok(())
    }

    // -- session CRUD -------------------------------------------------------

    /// Create a new session with the given key and optional overrides.
    ///
    /// If `model` or `system_prompt` are `None`, the service-level defaults
    /// are used.
    #[instrument(skip(self))]
    pub async fn create_session(
        &self,
        key: SessionKey,
        title: Option<String>,
        model: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<SessionEntry, ChatError> {
        let now = Utc::now();
        let entry = SessionEntry {
            key,
            title,
            model: model.or_else(|| Some(self.current_default_model())),
            system_prompt: system_prompt.or_else(|| Some(self.current_system_prompt())),
            message_count: 0,
            preview: None,
            metadata: None,
            created_at: now,
            updated_at: now,
        };
        let created = self.session_repo.create_session(&entry).await?;
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
            .session_repo
            .list_sessions(limit.unwrap_or(50), offset.unwrap_or(0))
            .await?;
        Ok(sessions)
    }

    /// Get a single session by key. Returns [`ChatError::SessionNotFound`]
    /// if the key does not exist.
    #[instrument(skip(self))]
    pub async fn get_session(&self, key: &SessionKey) -> Result<SessionEntry, ChatError> {
        self.session_repo
            .get_session(key)
            .await?
            .ok_or_else(|| ChatError::SessionNotFound {
                key: key.as_str().to_owned(),
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
        let updated = self.session_repo.update_session(&session).await?;
        info!(key = %key, "session fields updated");
        Ok(updated)
    }

    /// Delete a session and all its messages.
    #[instrument(skip(self))]
    pub async fn delete_session(&self, key: &SessionKey) -> Result<(), ChatError> {
        self.session_repo.delete_session(key).await?;
        info!(key = %key, "session deleted");
        Ok(())
    }

    // -- messages -----------------------------------------------------------

    /// Get message history for a session, with optional cursor-based
    /// pagination via `after_seq` and `limit`.
    #[instrument(skip(self))]
    pub async fn get_messages(
        &self,
        key: &SessionKey,
        after_seq: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<ChatMessage>, ChatError> {
        // Verify session exists
        let _ = self.get_session(key).await?;
        let messages = self
            .session_repo
            .read_messages(key, after_seq, limit)
            .await?;
        Ok(messages)
    }

    /// Clear all messages for a session and reset its `message_count` and
    /// `preview` to their initial values.
    #[instrument(skip(self))]
    pub async fn clear_messages(&self, key: &SessionKey) -> Result<(), ChatError> {
        let _ = self.get_session(key).await?;
        self.session_repo.clear_messages(key).await?;

        // Reset session message_count
        let mut session = self.get_session(key).await?;
        session.message_count = 0;
        session.preview = None;
        self.session_repo.update_session(&session).await?;
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
        match self.session_repo.get_session(key).await? {
            Some(existing) => Ok(existing),
            None => {
                self.create_session(
                    key.clone(),
                    title.map(ToOwned::to_owned),
                    model.map(ToOwned::to_owned),
                    system_prompt.map(ToOwned::to_owned),
                )
                .await
            }
        }
    }

    // -- append raw message -------------------------------------------------

    /// Append a pre-built message to a session without invoking the LLM.
    ///
    /// This is useful for background workers that run their own
    /// [`AgentRunner`] loop and need to persist the conversation turns
    /// after the fact.
    #[instrument(skip(self, message))]
    pub async fn append_message_raw(
        &self,
        key: &SessionKey,
        message: &ChatMessage,
    ) -> Result<ChatMessage, ChatError> {
        let persisted = self.session_repo.append_message(key, message).await?;
        Ok(persisted)
    }

    /// Return a reference to the tools registry shared by this service.
    pub fn tools(&self) -> &Arc<ToolRegistry> { self.chat_agent.orchestrator().tools() }

    /// Persist a compaction effect: clear old messages, store the summary.
    async fn persist_compaction(
        &self,
        key: &SessionKey,
        summary: &ChatMessage,
    ) -> Result<(), ChatError> {
        info!(session = %key, "persisting compaction to session store");
        self.session_repo
            .clear_messages(key)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to clear messages during compaction: {e}"),
            })?;
        self.session_repo
            .append_message(key, summary)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to append summary during compaction: {e}"),
            })?;
        Ok(())
    }

    // -- send message (LLM) -------------------------------------------------

    /// Common session-level setup for `send_message` and
    /// `send_message_streaming`.
    ///
    /// Handles: session creation, history retrieval, user message persistence,
    /// model resolution, and context length lookup. Returns the data needed
    /// by the [`ChatAgent`] to run.
    async fn prepare_session_data(
        &self,
        key: &SessionKey,
        user_text: &str,
        image_urls: &Option<Vec<String>>,
    ) -> Result<SessionData, ChatError> {
        if user_text.trim().is_empty() {
            return Err(ChatError::InvalidRequest {
                message: "message text cannot be empty".to_owned(),
            });
        }

        // 1. Ensure session exists
        let session = match self.session_repo.get_session(key).await? {
            Some(s) => s,
            None => self.create_session(key.clone(), None, None, None).await?,
        };

        // 2. Read existing history
        let history = self.session_repo.read_messages(key, None, None).await?;

        // 3. Persist user message -- multimodal if images are present
        let has_images = image_urls.as_ref().is_some_and(|urls| !urls.is_empty());
        let user_msg = if has_images {
            let urls = image_urls.as_ref().unwrap();
            let mut blocks = vec![ContentBlock::Text {
                text: user_text.to_owned(),
            }];
            for url in urls {
                blocks.push(ContentBlock::ImageUrl { url: url.clone() });
            }
            ChatMessage {
                seq:          0,
                role:         MessageRole::User,
                content:      MessageContent::Multimodal(blocks),
                tool_call_id: None,
                tool_name:    None,
                created_at:   Utc::now(),
            }
        } else {
            ChatMessage::user(user_text)
        };
        self.session_repo.append_message(key, &user_msg).await?;

        // 4. Resolve model and context length
        let model = session
            .model
            .clone()
            .unwrap_or_else(|| self.current_default_model());
        let context_length = self
            .model_catalog
            .get_context_length(&model)
            .unwrap_or(128_000) as usize;

        // 5. Resolve base system prompt
        let base_system_prompt = session
            .system_prompt
            .clone()
            .unwrap_or_else(|| self.current_system_prompt());

        // 6. Build UserContent
        let user_content = if has_images {
            let urls = image_urls.as_ref().unwrap();
            UserContent::Multimodal {
                text:       user_text.to_owned(),
                image_urls: urls.clone(),
            }
        } else {
            UserContent::Text(user_text.to_owned())
        };

        Ok(SessionData {
            session,
            history,
            model,
            context_length,
            base_system_prompt,
            user_content,
        })
    }

    /// Send a user message and get an assistant response.
    #[instrument(skip(self, user_text, image_urls))]
    pub async fn send_message(
        &self,
        key: &SessionKey,
        user_text: String,
        image_urls: Option<Vec<String>>,
    ) -> Result<ChatMessage, ChatError> {
        let SessionData {
            mut session,
            history,
            model,
            context_length,
            base_system_prompt,
            user_content,
        } = self.prepare_session_data(key, &user_text, &image_urls).await?;

        // Delegate agent execution to ChatAgent.
        let (output, compaction) = self
            .chat_agent
            .run(&base_system_prompt, user_content, &history, &model, context_length)
            .await
            .map_err(|e| ChatError::AgentError {
                message: e.to_string(),
            })?;

        // Persist compaction if it occurred.
        if let Some(effect) = compaction {
            self.persist_compaction(key, &effect.summary).await?;
        }

        // Persist assistant response
        let assistant_msg = ChatMessage::assistant(&output.response_text);
        let persisted = self
            .session_repo
            .append_message(key, &assistant_msg)
            .await?;

        // Update session metadata
        session.message_count += 2; // user + assistant
        if session.preview.is_none() {
            session.preview = Some(truncate_preview(&user_text, 100));
        }
        let _ = self.session_repo.update_session(&session).await;

        info!(
            key = %key,
            iterations = output.iterations,
            tool_calls = output.tool_calls_made,
            "message exchange complete"
        );

        Ok(persisted)
    }

    /// Streaming variant of [`send_message`](Self::send_message).
    #[instrument(skip(self, user_text, image_urls))]
    pub async fn send_message_streaming(
        &self,
        key: &SessionKey,
        user_text: String,
        image_urls: Option<Vec<String>>,
    ) -> Result<mpsc::Receiver<ChatStreamEvent>, ChatError> {
        let SessionData {
            mut session,
            history,
            model,
            context_length,
            base_system_prompt,
            user_content,
        } = self.prepare_session_data(key, &user_text, &image_urls).await?;

        // Delegate streaming setup to ChatAgent.
        let stream_setup = self
            .chat_agent
            .prepare_streaming(
                &base_system_prompt,
                user_content,
                &history,
                &model,
                context_length,
            )
            .await
            .map_err(|e| ChatError::AgentError {
                message: e.to_string(),
            })?;

        // Persist compaction if it occurred.
        if let Some(effect) = stream_setup.compaction {
            self.persist_compaction(key, &effect.summary).await?;
        }

        // Start the streaming agent loop.
        let mut runner_rx = stream_setup.runner.run_streaming(stream_setup.effective_tools);

        // Channel for ChatStreamEvents sent to the SSE handler.
        let (tx, rx) = mpsc::channel::<ChatStreamEvent>(128);

        // Spawn a task that bridges RunnerEvent -> ChatStreamEvent and
        // handles persistence on completion.
        let session_repo = Arc::clone(&self.session_repo);
        let session_key = key.clone();
        let orchestrator = stream_setup.orchestrator;

        tokio::spawn(async move {
            while let Some(runner_event) = runner_rx.recv().await {
                let chat_event = ChatStreamEvent::from(runner_event.clone());
                let is_done = matches!(chat_event, ChatStreamEvent::Done { .. });
                let is_error = matches!(chat_event, ChatStreamEvent::Error { .. });

                // On Done: persist assistant message and update session.
                if let ChatStreamEvent::Done { ref text } = chat_event {
                    let assistant_msg = ChatMessage::assistant(text);
                    if let Err(e) = session_repo
                        .append_message(&session_key, &assistant_msg)
                        .await
                    {
                        tracing::error!(error = %e, "failed to persist streaming assistant message");
                    }

                    session.message_count += 2; // user + assistant
                    if session.preview.is_none() {
                        session.preview = Some(truncate_preview(&user_text, 100));
                    }
                    let _ = session_repo.update_session(&session).await;

                    info!(key = %session_key, "streaming message exchange complete");

                    // Fire-and-forget memory reflection.
                    orchestrator.spawn_memory_reflection(&user_text, text);
                }

                // Forward the event to the SSE stream; if the receiver is
                // dropped (client disconnected), stop the loop.
                if tx.send(chat_event).await.is_err() {
                    tracing::debug!("SSE client disconnected, stopping stream bridge");
                    break;
                }

                if is_done || is_error {
                    break;
                }
            }
        });

        Ok(rx)
    }

    // -- export to memory ---------------------------------------------------

    /// Export a session's message history to the memory directory as markdown.
    #[instrument(skip(self))]
    pub async fn export_session_to_memory(
        &self,
        key: &SessionKey,
    ) -> Result<std::path::PathBuf, ChatError> {
        let session = self.get_session(key).await?;
        let messages = self.get_messages(key, None, None).await?;

        let mut md = String::new();
        md.push_str(&format!("# Session: {}\n\n", key.as_str()));
        if let Some(title) = &session.title {
            md.push_str(&format!("**Title:** {title}\n\n"));
        }
        md.push_str(&format!(
            "**Exported at:** {}\n\n",
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        ));
        md.push_str("---\n\n");

        for msg in &messages {
            let role = &msg.role;
            let text = msg.content.as_text();
            md.push_str(&format!("### {role}\n\n{text}\n\n"));
        }

        let sessions_dir = rara_paths::memory_sessions_dir();
        tokio::fs::create_dir_all(sessions_dir)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to create sessions memory dir: {e}"),
            })?;

        // Sanitize the key for use as a filename (replace ':' and '/' with '-').
        let safe_name: String = key
            .as_str()
            .chars()
            .map(|c| if c == ':' || c == '/' { '-' } else { c })
            .collect();
        let file_path = sessions_dir.join(format!("{safe_name}.md"));
        tokio::fs::write(&file_path, &md)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to write session export: {e}"),
            })?;

        info!(key = %key, path = %file_path.display(), "session exported to memory");
        Ok(file_path)
    }

    // -- fork ---------------------------------------------------------------

    /// Fork a session at a specific message sequence number.
    #[instrument(skip(self))]
    pub async fn fork_session(
        &self,
        source_key: &SessionKey,
        target_key: SessionKey,
        fork_at_seq: i64,
    ) -> Result<SessionEntry, ChatError> {
        let forked = self
            .session_repo
            .fork_session(source_key, &target_key, fork_at_seq)
            .await?;
        info!(
            source = %source_key,
            target = %target_key,
            fork_at_seq,
            "session forked"
        );
        Ok(forked)
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
        let result = self.session_repo.bind_channel(&binding).await?;
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
            .session_repo
            .get_channel_binding(channel_type, account, chat_id)
            .await?;
        Ok(binding)
    }

    // -- accessors for background workers -----------------------------------

    /// Append a user message and an assistant response to a session,
    /// auto-creating the session if it does not exist.
    #[instrument(skip(self, user_text, assistant_text))]
    pub async fn append_messages(
        &self,
        key: &SessionKey,
        user_text: &str,
        assistant_text: &str,
    ) -> Result<(), ChatError> {
        // Ensure session exists.
        if self.session_repo.get_session(key).await?.is_none() {
            self.create_session(key.clone(), None, None, None).await?;
        }

        let user_msg = ChatMessage::user(user_text);
        self.session_repo.append_message(key, &user_msg).await?;

        let assistant_msg = ChatMessage::assistant(assistant_text);
        self.session_repo
            .append_message(key, &assistant_msg)
            .await?;

        Ok(())
    }

}

impl std::fmt::Debug for ChatService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatService")
            .field("default_model", &self.current_default_model())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Bundle returned by [`ChatService::prepare_session_data`] containing
/// session-level data needed by the [`ChatAgent`].
struct SessionData {
    session:           SessionEntry,
    history:           Vec<ChatMessage>,
    model:             String,
    context_length:    usize,
    base_system_prompt: String,
    user_content:      UserContent,
}

/// Truncate a string to at most `max_len` characters.
fn truncate_preview(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_owned()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_preview_short() {
        assert_eq!(truncate_preview("hello", 100), "hello");
    }

    #[test]
    fn truncate_preview_long() {
        let long = "a".repeat(200);
        let result = truncate_preview(&long, 50);
        assert!(result.len() <= 50);
        assert!(result.ends_with("..."));
    }

}
