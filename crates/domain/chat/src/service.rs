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
use openrouter_rs::{
    api::chat::{Content, ContentPart, Message},
    types::Role,
};
use rara_agents::{model::OpenRouterLoaderRef, runner::AgentRunner, tool_registry::ToolRegistry};
use rara_domain_shared::settings::model::{ModelScenario, Settings};
use rara_sessions::{
    repository::SessionRepository,
    types::{
        ChannelBinding, ChatMessage, ContentBlock, MessageContent, MessageRole, SessionEntry,
        SessionKey,
    },
};
use tokio::sync::watch;
use tracing::{info, instrument};

use crate::error::ChatError;

/// Default system prompt used when no custom prompt is configured in settings
/// and no session-level override is provided.
const SYSTEM_PROMPT_FILE: &str = "chat/default_system.md";
const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../../../../prompts/chat/default_system.md");

fn compose_system_prompt(base_prompt: &str, soul_prompt: Option<&str>) -> String {
    if let Some(soul) = soul_prompt.filter(|s| !s.trim().is_empty()) {
        return format!("{soul}\n\n# Chat Instructions\n{base_prompt}");
    }
    base_prompt.to_owned()
}

fn resolve_soul_prompt(settings: &Settings) -> Option<String> {
    if settings
        .agent
        .soul
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return settings.agent.soul.clone();
    }
    let markdown_soul = rara_paths::load_agent_soul_prompt();
    if markdown_soul.trim().is_empty() {
        return None;
    }
    Some(markdown_soul)
}

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
    session_repo: Arc<dyn SessionRepository>,
    /// Factory for creating OpenRouter API clients.
    llm_provider: OpenRouterLoaderRef,
    /// Registry of tools available to the agent during execution.
    tools:        Arc<ToolRegistry>,
    /// Watch receiver for runtime settings — provides dynamic model and
    /// system prompt configuration.
    settings_rx:  watch::Receiver<Settings>,
}

impl ChatService {
    /// Create a new chat service with the given dependencies.
    ///
    /// The `settings_rx` watch receiver supplies the current runtime settings,
    /// from which the default model and system prompt are read dynamically.
    #[must_use]
    pub fn new(
        session_repo: Arc<dyn SessionRepository>,
        llm_provider: OpenRouterLoaderRef,
        tools: Arc<ToolRegistry>,
        settings_rx: watch::Receiver<Settings>,
    ) -> Self {
        Self {
            session_repo,
            llm_provider,
            tools,
            settings_rx,
        }
    }

    /// Read the current default model from runtime settings.
    fn current_default_model(&self) -> String {
        self.settings_rx
            .borrow()
            .ai
            .model_for(ModelScenario::Chat)
            .to_owned()
    }

    /// Read the current system prompt from runtime settings, falling back
    /// to [`DEFAULT_SYSTEM_PROMPT`] when no custom prompt is configured.
    fn current_system_prompt(&self) -> String {
        let settings = self.settings_rx.borrow();
        let base_prompt =
            rara_paths::load_prompt_markdown(SYSTEM_PROMPT_FILE, DEFAULT_SYSTEM_PROMPT);
        let soul_prompt = resolve_soul_prompt(&settings);
        compose_system_prompt(&base_prompt, soul_prompt.as_deref())
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
    pub fn tools(&self) -> &Arc<ToolRegistry> { &self.tools }

    // -- send message (LLM) -------------------------------------------------

    /// Send a user message and get an assistant response.
    ///
    /// This method:
    /// 1. Ensures the session exists (creates it if `auto_create` is true).
    /// 2. Reads the existing message history.
    /// 3. Persists the user message (multimodal if `image_urls` are provided).
    /// 4. Converts history to `openrouter_rs::api::chat::Message` format.
    /// 5. Runs the agent loop.
    /// 6. Persists the assistant response.
    /// 7. Updates session metadata.
    #[instrument(skip(self, user_text, image_urls))]
    pub async fn send_message(
        &self,
        key: &SessionKey,
        user_text: String,
        image_urls: Option<Vec<String>>,
    ) -> Result<ChatMessage, ChatError> {
        if user_text.trim().is_empty() {
            return Err(ChatError::InvalidRequest {
                message: "message text cannot be empty".to_owned(),
            });
        }

        // 1. Ensure session exists
        let mut session = match self.session_repo.get_session(key).await? {
            Some(s) => s,
            None => {
                // Auto-create session
                self.create_session(key.clone(), None, None, None).await?
            }
        };

        // 2. Read existing history
        let history = self.session_repo.read_messages(key, None, None).await?;

        // 3. Persist user message — multimodal if images are present
        let has_images = image_urls.as_ref().is_some_and(|urls| !urls.is_empty());
        let user_msg = if has_images {
            let urls = image_urls.as_ref().unwrap();
            let mut blocks = vec![ContentBlock::Text {
                text: user_text.clone(),
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
            ChatMessage::user(&user_text)
        };
        self.session_repo.append_message(key, &user_msg).await?;

        // 4. Convert history to openrouter format
        let openrouter_history = history
            .iter()
            .map(to_openrouter_message)
            .collect::<Vec<_>>();

        // 5. Build and run agent — multimodal content for user message
        let model = session
            .model
            .clone()
            .unwrap_or_else(|| self.current_default_model());
        let system_prompt = session
            .system_prompt
            .clone()
            .unwrap_or_else(|| self.current_system_prompt());

        let user_content = if has_images {
            let urls = image_urls.as_ref().unwrap();
            let mut parts = vec![ContentPart::text(&user_text)];
            for url in urls {
                parts.push(ContentPart::image_url(url));
            }
            Content::Parts(parts)
        } else {
            Content::Text(user_text.clone())
        };

        let runner = AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .model_name(model)
            .system_prompt(system_prompt)
            .user_content(user_content)
            .history(openrouter_history)
            .build();

        let result = runner
            .run(&self.tools, None)
            .await
            .map_err(|e| ChatError::AgentError {
                message: e.to_string(),
            })?;

        // 6. Extract assistant text from response
        let assistant_text = result
            .provider_response
            .choices
            .first()
            .and_then(|choice| choice.content())
            .unwrap_or_default()
            .to_owned();

        // 7. Persist assistant response
        let assistant_msg = ChatMessage::assistant(&assistant_text);
        let persisted = self
            .session_repo
            .append_message(key, &assistant_msg)
            .await?;

        // 8. Update session metadata
        session.message_count += 2; // user + assistant
        if session.preview.is_none() {
            // Use first user message as preview
            session.preview = Some(truncate_preview(&user_text, 100));
        }
        let _ = self.session_repo.update_session(&session).await;

        info!(
            key = %key,
            iterations = result.iterations,
            tool_calls = result.tool_calls_made,
            "message exchange complete"
        );

        Ok(persisted)
    }

    // -- export to memory ---------------------------------------------------

    /// Export a session's message history to the memory directory as markdown.
    ///
    /// Reads all messages for the given session key, formats them as a
    /// markdown document, and writes it to
    /// `rara_paths::memory_sessions_dir()/{key}.md`. Returns the path of
    /// the written file.
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

    /// Fork a session at a specific message sequence number, creating a new
    /// session that shares the conversation history up to that point.
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
    ///
    /// If a binding for the same channel already exists, the session key is
    /// updated (upsert semantics).
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
    ///
    /// Returns `None` if no binding exists for the given channel coordinates.
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
    ///
    /// This is intended for background workers (e.g. the scheduled agent)
    /// that produce messages outside the normal `send_message` flow.
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
// Conversion helpers
// ---------------------------------------------------------------------------

/// Convert a session [`ChatMessage`] to an
/// [`openrouter_rs::api::chat::Message`].
///
/// Maps domain roles to OpenRouter roles and converts text / multimodal
/// content to the appropriate [`Content`] variant. Tool-related fields
/// (`tool_call_id`, `tool_name`) are carried over when present.
pub fn to_openrouter_message(msg: &ChatMessage) -> Message {
    let role = match msg.role {
        MessageRole::System => Role::System,
        MessageRole::User => Role::User,
        MessageRole::Assistant => Role::Assistant,
        MessageRole::Tool | MessageRole::ToolResult => Role::Tool,
    };

    let content = match &msg.content {
        MessageContent::Text(text) => Content::Text(text.clone()),
        MessageContent::Multimodal(blocks) => {
            let parts = blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => ContentPart::text(text),
                    ContentBlock::ImageUrl { url } => ContentPart::image_url(url),
                })
                .collect();
            Content::Parts(parts)
        }
    };

    let mut message = Message::new(role, content);

    if let Some(ref tool_call_id) = msg.tool_call_id {
        message.tool_call_id = Some(tool_call_id.clone());
    }
    if let Some(ref tool_name) = msg.tool_name {
        message.name = Some(tool_name.clone());
    }

    message
}

/// Truncate a string to at most `max_len` characters.
///
/// If the string exceeds the limit, it is cut and `"..."` is appended (the
/// total length will be exactly `max_len`).
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

    #[test]
    fn to_openrouter_message_text() {
        let msg = ChatMessage::user("hello");
        let converted = to_openrouter_message(&msg);
        assert!(matches!(converted.role, Role::User));
        assert!(matches!(converted.content, Content::Text(ref t) if t == "hello"));
    }

    #[test]
    fn to_openrouter_message_assistant() {
        let msg = ChatMessage::assistant("response");
        let converted = to_openrouter_message(&msg);
        assert!(matches!(converted.role, Role::Assistant));
    }

    #[test]
    fn to_openrouter_message_system() {
        let msg = ChatMessage::system("you are helpful");
        let converted = to_openrouter_message(&msg);
        assert!(matches!(converted.role, Role::System));
    }
}
