//! Chat domain service — session-based conversations backed by LLM agents.

use std::sync::Arc;

use chrono::Utc;
use openrouter_rs::api::chat::{Content, ContentPart, Message};
use openrouter_rs::types::Role;
use rara_agents::{
    model::OpenRouterLoaderRef,
    runner::AgentRunner,
    tool_registry::ToolRegistry,
};
use rara_sessions::{
    repository::SessionRepository,
    types::{
        ChannelBinding, ChatMessage, ContentBlock, MessageContent, MessageRole, SessionEntry,
        SessionKey,
    },
};
use tracing::{info, instrument};

use crate::error::ChatError;

/// The main chat service: manages sessions and delegates to the agent runner
/// for LLM interactions.
#[derive(Clone)]
pub struct ChatService {
    session_repo:              Arc<dyn SessionRepository>,
    llm_provider:              OpenRouterLoaderRef,
    tools:                     Arc<ToolRegistry>,
    pub default_model:         String,
    pub default_system_prompt: String,
}

impl ChatService {
    /// Create a new chat service.
    #[must_use]
    pub fn new(
        session_repo: Arc<dyn SessionRepository>,
        llm_provider: OpenRouterLoaderRef,
        tools: Arc<ToolRegistry>,
        default_model: String,
        default_system_prompt: String,
    ) -> Self {
        Self {
            session_repo,
            llm_provider,
            tools,
            default_model,
            default_system_prompt,
        }
    }

    // -- session CRUD -------------------------------------------------------

    /// Create a new session.
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
            model: model.or_else(|| Some(self.default_model.clone())),
            system_prompt: system_prompt.or_else(|| Some(self.default_system_prompt.clone())),
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

    /// List sessions.
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

    /// Get a single session.
    #[instrument(skip(self))]
    pub async fn get_session(&self, key: &SessionKey) -> Result<SessionEntry, ChatError> {
        self.session_repo
            .get_session(key)
            .await?
            .ok_or_else(|| ChatError::SessionNotFound {
                key: key.as_str().to_owned(),
            })
    }

    /// Delete a session and all its messages.
    #[instrument(skip(self))]
    pub async fn delete_session(&self, key: &SessionKey) -> Result<(), ChatError> {
        self.session_repo.delete_session(key).await?;
        info!(key = %key, "session deleted");
        Ok(())
    }

    // -- messages -----------------------------------------------------------

    /// Get message history for a session.
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

    /// Clear all messages for a session.
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

    // -- send message (LLM) -------------------------------------------------

    /// Send a user message and get an assistant response.
    ///
    /// This method:
    /// 1. Ensures the session exists (creates it if `auto_create` is true).
    /// 2. Reads the existing message history.
    /// 3. Persists the user message.
    /// 4. Converts history to `openrouter_rs::api::chat::Message` format.
    /// 5. Runs the agent loop.
    /// 6. Persists the assistant response.
    /// 7. Updates session metadata.
    #[instrument(skip(self, user_text))]
    pub async fn send_message(
        &self,
        key: &SessionKey,
        user_text: String,
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
        let history = self
            .session_repo
            .read_messages(key, None, None)
            .await?;

        // 3. Persist user message
        let user_msg = ChatMessage::user(&user_text);
        self.session_repo.append_message(key, &user_msg).await?;

        // 4. Convert history to openrouter format
        let openrouter_history = history
            .iter()
            .map(to_openrouter_message)
            .collect::<Vec<_>>();

        // 5. Build and run agent
        let model = session
            .model
            .clone()
            .unwrap_or_else(|| self.default_model.clone());
        let system_prompt = session
            .system_prompt
            .clone()
            .unwrap_or_else(|| self.default_system_prompt.clone());

        let runner = AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .model_name(model)
            .system_prompt(system_prompt)
            .user_content(Content::Text(user_text.clone()))
            .history(openrouter_history)
            .build();

        let result = runner.run(&self.tools, None).await.map_err(|e| {
            ChatError::AgentError {
                message: e.to_string(),
            }
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

    /// Bind an external channel to a session key.
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

    /// Look up which session an external channel maps to.
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
}

impl std::fmt::Debug for ChatService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatService")
            .field("default_model", &self.default_model)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

/// Convert a session `ChatMessage` to an `openrouter_rs::api::chat::Message`.
fn to_openrouter_message(msg: &ChatMessage) -> Message {
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

/// Truncate a string to at most `max_len` characters, appending "..." if
/// truncated.
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
