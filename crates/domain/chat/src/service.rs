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

use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
    ChatCompletionRequestUserMessageArgs,
    ChatCompletionRequestUserMessageContentPart,
    ChatCompletionRequestMessageContentPartImage,
    ChatCompletionRequestMessageContentPartText, ImageUrlArgs,
};
use chrono::Utc;
use rara_agents::{
    model::LlmProviderLoaderRef,
    runner::{AgentRunner, UserContent},
    tool_registry::ToolRegistry,
};
use rara_domain_shared::settings::model::{ModelScenario, Settings};
use rara_mcp::{manager::mgr::McpManager, tool_bridge::McpToolBridge};
use rara_memory::MemoryManager;
use rara_sessions::{
    repository::SessionRepository,
    types::{
        ChannelBinding, ChatMessage, ContentBlock, MessageContent, MessageRole, SessionEntry,
        SessionKey,
    },
};
use tokio::sync::{mpsc, watch};
use tracing::{info, instrument};

use crate::stream::ChatStreamEvent;

use crate::{
    error::ChatError,
    model_catalog::{ChatModel, ModelCatalog},
};

/// Default system prompt used when no custom prompt is configured in settings
/// and no session-level override is provided.
const SYSTEM_PROMPT_FILE: &str = "chat/default_system.md";
const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../../../../prompts/chat/default_system.md");

fn compose_system_prompt(base_prompt: &str, soul_prompt: Option<&str>) -> String {
    if let Some(soul) = soul_prompt.filter(|s| !s.trim().is_empty()) {
        // Avoid duplicating soul when a persisted session prompt already
        // contains it (e.g. prompt was previously composed and stored).
        if base_prompt.contains(soul.trim()) {
            return base_prompt.to_owned();
        }
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
    session_repo:   Arc<dyn SessionRepository>,
    /// Factory for creating LLM provider clients.
    llm_provider:   LlmProviderLoaderRef,
    /// Registry of tools available to the agent during execution.
    tools:          Arc<ToolRegistry>,
    /// Watch receiver for runtime settings — provides dynamic model and
    /// system prompt configuration.
    settings_rx:    watch::Receiver<Settings>,
    /// Optional memory manager for pre-fetching relevant context on first
    /// turn of a session.
    memory_manager: Option<Arc<MemoryManager>>,
    /// Cached catalog of models fetched from OpenRouter.
    model_catalog:  ModelCatalog,
    /// Settings service for persisting favorite models.
    settings_svc:   rara_domain_shared::settings::SettingsSvc,
    /// Skills registry for available skills listing in system prompt.
    skill_registry: rara_skills::registry::InMemoryRegistry,
    /// MCP manager for dynamic per-request MCP tool discovery.
    mcp_manager:    McpManager,
}

impl ChatService {
    /// Create a new chat service with the given dependencies.
    ///
    /// The `settings_rx` watch receiver supplies the current runtime settings,
    /// from which the default model and system prompt are read dynamically.
    /// The optional `memory_manager` enables automatic memory pre-fetch on
    /// the first turn of a session.
    #[must_use]
    pub fn new(
        session_repo: Arc<dyn SessionRepository>,
        llm_provider: LlmProviderLoaderRef,
        tools: Arc<ToolRegistry>,
        settings_rx: watch::Receiver<Settings>,
        memory_manager: Option<Arc<MemoryManager>>,
        settings_svc: rara_domain_shared::settings::SettingsSvc,
        skill_registry: rara_skills::registry::InMemoryRegistry,
        mcp_manager: McpManager,
    ) -> Self {
        Self {
            session_repo,
            llm_provider,
            tools,
            settings_rx,
            memory_manager,
            model_catalog: ModelCatalog::new(),
            settings_svc,
            skill_registry,
            mcp_manager,
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

    // -- model catalog ------------------------------------------------------

    /// List available models, dynamically fetching from OpenRouter when an
    /// API key is configured. Favorites are marked and sorted to the top.
    pub async fn list_models(&self) -> Vec<ChatModel> {
        let settings = self.settings_rx.borrow().clone();
        let api_key = settings.ai.openrouter_api_key.as_deref();
        let favorites = &settings.ai.favorite_models;
        self.model_catalog.list_models(api_key, favorites).await
    }

    /// Replace the user's favorite model list and persist to settings.
    pub async fn set_favorite_models(&self, ids: Vec<String>) -> Result<(), ChatError> {
        use rara_domain_shared::settings::model::{AiRuntimeSettingsPatch, UpdateRequest};

        let patch = UpdateRequest {
            ai:       Some(AiRuntimeSettingsPatch {
                favorite_models: Some(ids),
                ..Default::default()
            }),
            telegram: None,
            agent:    None,
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
    pub fn tools(&self) -> &Arc<ToolRegistry> { &self.tools }

    // -- send message (LLM) -------------------------------------------------

    /// Common setup for `send_message` and `send_message_streaming`.
    async fn prepare_agent_run(
        &self,
        key: &SessionKey,
        user_text: &str,
        image_urls: &Option<Vec<String>>,
    ) -> Result<AgentRunSetup, ChatError> {
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

        // 3. Persist user message — multimodal if images are present
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

        // 4. Resolve model
        let model = session
            .model
            .clone()
            .unwrap_or_else(|| self.current_default_model());

        // 4a. Check if context compaction is needed
        let history_tokens = estimate_history_tokens(&history);
        let model_context = self
            .model_catalog
            .get_context_length(&model)
            .unwrap_or(128_000);
        let threshold = (model_context as f64 * 0.80) as usize;

        let history = if history_tokens > threshold {
            self.compact_history(key, &history, &model).await?
        } else {
            history
        };

        // 4b. Convert history to async-openai message format
        let chat_history = history
            .iter()
            .map(to_chat_message)
            .collect::<Vec<_>>();

        // 5. Build system prompt
        let base_system_prompt = session
            .system_prompt
            .clone()
            .unwrap_or_else(|| self.current_system_prompt());
        let soul_prompt = {
            let settings = self.settings_rx.borrow();
            resolve_soul_prompt(&settings)
        };
        let mut system_prompt = compose_system_prompt(&base_system_prompt, soul_prompt.as_deref());

        // Inject core user profile into system prompt.
        if let Some(ref mm) = self.memory_manager {
            if let Ok(profile) = mm.read_core_profile().await {
                if !profile.trim().is_empty() {
                    system_prompt = format!("{profile}\n\n---\n\n{system_prompt}");
                }
            }
        }

        // Pre-fetch relevant memory context for new / short sessions.
        if history.len() < 3 {
            if let Some(ref mm) = self.memory_manager {
                match mm.search(user_text, 5).await {
                    Ok(results) if !results.is_empty() => {
                        system_prompt.push_str("\n\n## Relevant Memory Context\n");
                        for hit in &results {
                            system_prompt.push_str(&format!("- [{}] {}\n", hit.path, hit.snippet,));
                        }
                        info!(
                            hits = results.len(),
                            "memory pre-fetch injected into system prompt"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "memory pre-fetch failed, continuing without context");
                    }
                }
            }
        }

        // -- skill listing injection --
        let tool_whitelist = {
            let all_skills = self.skill_registry.list_all();
            let skills_xml = rara_skills::prompt_gen::generate_skills_prompt(&all_skills);
            if !skills_xml.is_empty() {
                system_prompt.push_str(&format!("\n\n{skills_xml}"));
            }
            Vec::<String>::new()
        };

        // Build effective tool registry with dynamic MCP tools
        let effective_tools = {
            let mut registry = if tool_whitelist.is_empty() {
                self.tools.filtered(&[])
            } else {
                self.tools.filtered(&tool_whitelist)
            };
            // Dynamically add MCP tools (per-request, cached by ManagedClient 5min TTL)
            match McpToolBridge::from_manager(self.mcp_manager.clone()).await {
                Ok(bridges) => {
                    for bridge in bridges {
                        let server = bridge.server_name().to_string();
                        registry.register_mcp(Arc::new(bridge), server);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to fetch dynamic MCP tools");
                }
            }
            Arc::new(registry)
        };

        let user_content = if has_images {
            let urls = image_urls.as_ref().unwrap();
            UserContent::Multimodal {
                text:       user_text.to_owned(),
                image_urls: urls.clone(),
            }
        } else {
            UserContent::Text(user_text.to_owned())
        };

        // Resolve fallback models from settings.
        let fallback_models = {
            let settings = self.settings_rx.borrow();
            let chain = settings.ai.fallback_chain(ModelScenario::Chat);
            chain
                .into_iter()
                .skip(1)
                .map(|s| s.to_owned().into())
                .collect()
        };

        let runner = AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .model_name(model)
            .system_prompt(system_prompt)
            .user_content(user_content)
            .history(chat_history)
            .fallback_models(fallback_models)
            .build();

        Ok(AgentRunSetup {
            runner,
            effective_tools,
            session,
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
        let AgentRunSetup {
            runner,
            effective_tools,
            mut session,
        } = self.prepare_agent_run(key, &user_text, &image_urls).await?;

        let result =
            runner
                .run(&effective_tools, None)
                .await
                .map_err(|e| ChatError::AgentError {
                    message: e.to_string(),
                })?;

        // 6. Extract assistant text from response
        let assistant_text = result
            .provider_response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
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
            session.preview = Some(truncate_preview(&user_text, 100));
        }
        let _ = self.session_repo.update_session(&session).await;

        info!(
            key = %key,
            iterations = result.iterations,
            tool_calls = result.tool_calls_made,
            "message exchange complete"
        );

        // Fire-and-forget: reflect on what was learned and update memory.
        if let Some(ref mm) = self.memory_manager {
            let mm = Arc::clone(mm);
            let user_text_clone = user_text.clone();
            let assistant_text_clone = assistant_text.clone();
            let llm = self.llm_provider.clone();
            let tools = Arc::clone(&self.tools);
            let model = self.current_default_model();

            tokio::spawn(async move {
                if let Err(e) = memory_reflection(
                    &mm,
                    &llm,
                    &tools,
                    &model,
                    &user_text_clone,
                    &assistant_text_clone,
                )
                .await
                {
                    tracing::warn!(error = %e, "memory reflection failed");
                }
            });
        }

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
        let AgentRunSetup {
            runner,
            effective_tools,
            mut session,
        } = self.prepare_agent_run(key, &user_text, &image_urls).await?;

        // Start the streaming agent loop.
        let mut runner_rx = runner.run_streaming(effective_tools);

        // Channel for ChatStreamEvents sent to the SSE handler.
        let (tx, rx) = mpsc::channel::<ChatStreamEvent>(128);

        // Spawn a task that bridges RunnerEvent -> ChatStreamEvent and
        // handles persistence on completion.
        let session_repo = Arc::clone(&self.session_repo);
        let session_key = key.clone();
        let memory_manager = self.memory_manager.clone();
        let llm_provider = self.llm_provider.clone();
        let tools = Arc::clone(&self.tools);
        let default_model = self.current_default_model();

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
                    if let Some(ref mm) = memory_manager {
                        let mm = Arc::clone(mm);
                        let user_text_clone = user_text.clone();
                        let assistant_text_clone = text.clone();
                        let llm = llm_provider.clone();
                        let tools = Arc::clone(&tools);
                        let model = default_model.clone();

                        tokio::spawn(async move {
                            if let Err(e) = memory_reflection(
                                &mm,
                                &llm,
                                &tools,
                                &model,
                                &user_text_clone,
                                &assistant_text_clone,
                            )
                            .await
                            {
                                tracing::warn!(error = %e, "memory reflection failed");
                            }
                        });
                    }
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

    // -- context compaction --------------------------------------------------

    /// Summarize the conversation history via LLM and replace it with a
    /// single `[Conversation Summary]` message.
    async fn compact_history(
        &self,
        session_key: &SessionKey,
        history: &[ChatMessage],
        model: &str,
    ) -> Result<Vec<ChatMessage>, ChatError> {
        info!(
            session = %session_key,
            original_messages = history.len(),
            "compacting conversation context"
        );

        // Build summarization prompt from history
        let history_text: String = history
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content.as_text()))
            .collect::<Vec<_>>()
            .join("\n");

        let summary_prompt = format!(
            "Summarize the following conversation history into a concise summary. Preserve key \
             facts, decisions, user preferences, and action items. Keep it under 500 words. \
             Respond in the same language as the conversation.\n\n{history_text}"
        );

        // Single-turn LLM call for summarization (no tools)
        let runner = AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .model_name(model.to_owned())
            .system_prompt(
                "You are a conversation summarizer. Be concise and preserve important details.",
            )
            .user_content(UserContent::Text(summary_prompt))
            .max_iterations(1_usize)
            .build();

        let empty_tools = ToolRegistry::default();
        let result = runner
            .run(&empty_tools, None)
            .await
            .map_err(|e| ChatError::AgentError {
                message: format!("compact failed: {e}"),
            })?;

        let summary = result
            .provider_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("[Summary unavailable]")
            .to_owned();

        // Create summary message
        let summary_msg = ChatMessage::assistant(format!("[Conversation Summary]\n{summary}"));

        // Replace old messages: clear, then insert the summary
        self.session_repo
            .clear_messages(session_key)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to clear messages during compaction: {e}"),
            })?;
        self.session_repo
            .append_message(session_key, &summary_msg)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to append summary during compaction: {e}"),
            })?;

        info!(
            session = %session_key,
            summary_len = summary.len(),
            "context compacted successfully"
        );

        Ok(vec![summary_msg])
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

/// Bundle returned by [`ChatService::prepare_agent_run`] containing
/// everything needed to execute the agent loop (sync or streaming).
struct AgentRunSetup {
    runner:          AgentRunner,
    effective_tools: Arc<ToolRegistry>,
    session:         SessionEntry,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

/// Convert a session [`ChatMessage`] to an
/// [`async_openai::types::ChatCompletionRequestMessage`].
///
/// Maps domain roles to async-openai message types and converts text /
/// multimodal content to the appropriate message variant. Tool-related
/// fields (`tool_call_id`, `tool_name`) are carried over when present.
pub fn to_chat_message(msg: &ChatMessage) -> ChatCompletionRequestMessage {
    match msg.role {
        MessageRole::System => {
            ChatCompletionRequestSystemMessageArgs::default()
                .content(msg.content.as_text())
                .build()
                .expect("system message build should not fail")
                .into()
        }
        MessageRole::User => {
            match &msg.content {
                MessageContent::Text(text) => {
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(text.as_str())
                        .build()
                        .expect("user message build should not fail")
                        .into()
                }
                MessageContent::Multimodal(blocks) => {
                    let parts: Vec<ChatCompletionRequestUserMessageContentPart> = blocks
                        .iter()
                        .map(|b| match b {
                            ContentBlock::Text { text } => {
                                ChatCompletionRequestUserMessageContentPart::Text(
                                    ChatCompletionRequestMessageContentPartText {
                                        text: text.clone(),
                                    },
                                )
                            }
                            ContentBlock::ImageUrl { url } => {
                                let image_url = ImageUrlArgs::default()
                                    .url(url.as_str())
                                    .build()
                                    .expect("image URL build should not fail");
                                ChatCompletionRequestUserMessageContentPart::ImageUrl(
                                    ChatCompletionRequestMessageContentPartImage {
                                        image_url,
                                    },
                                )
                            }
                        })
                        .collect();
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(parts)
                        .build()
                        .expect("multimodal user message build should not fail")
                        .into()
                }
            }
        }
        MessageRole::Assistant => {
            ChatCompletionRequestAssistantMessageArgs::default()
                .content(msg.content.as_text())
                .build()
                .expect("assistant message build should not fail")
                .into()
        }
        MessageRole::Tool | MessageRole::ToolResult => {
            let tool_call_id = msg
                .tool_call_id
                .as_deref()
                .unwrap_or("unknown");
            ChatCompletionRequestToolMessageArgs::default()
                .tool_call_id(tool_call_id)
                .content(msg.content.as_text())
                .build()
                .expect("tool message build should not fail")
                .into()
        }
    }
}

/// Run a lightweight memory reflection after a conversation turn.
async fn memory_reflection(
    _mm: &Arc<MemoryManager>,
    llm: &LlmProviderLoaderRef,
    tools: &Arc<ToolRegistry>,
    model: &str,
    user_text: &str,
    assistant_text: &str,
) -> Result<(), ChatError> {
    // Only expose memory-related tools for the reflection agent.
    let mut reflection_tools = ToolRegistry::default();
    if let Some(tool) = tools.get("memory_update_profile") {
        reflection_tools.register_service(Arc::clone(tool));
    }
    if let Some(tool) = tools.get("memory_write") {
        reflection_tools.register_service(Arc::clone(tool));
    }

    // If neither tool is available, skip reflection entirely.
    if reflection_tools.is_empty() {
        return Ok(());
    }

    let reflection_prompt = format!(
        "You are a memory maintenance agent. Based on the following exchange, extract any new \
         facts about the user (name, role, location, preferences, goals, important context). If \
         you learned something new, use memory_update_profile to update the relevant section \
         (\"Basic Info\", \"Preferences\", \"Current Goals\", or \"Key Context\"). If nothing new \
         was learned, do nothing — do NOT call any tools.\n\nKeep updates concise (3-5 bullet \
         points per section max). Only add genuinely useful information.\n\n## User \
         Message\n{user_text}\n\n## Assistant Response\n{assistant_text}"
    );

    let runner = AgentRunner::builder()
        .llm_provider(llm.clone())
        .model_name(model.to_owned())
        .system_prompt(
            "You are a silent memory maintenance agent. Your only job is to update the user \
             profile if new facts were learned. Never produce conversational output."
                .to_owned(),
        )
        .user_content(UserContent::Text(reflection_prompt))
        .max_iterations(1_usize)
        .build();

    let _result = runner
        .run(&reflection_tools, None)
        .await
        .map_err(|e| ChatError::AgentError {
            message: format!("memory reflection agent: {e}"),
        })?;

    tracing::debug!("memory reflection complete");
    Ok(())
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

// ---------------------------------------------------------------------------
// Token estimation
// ---------------------------------------------------------------------------

/// Rough token estimate: ~3 chars per token (balanced for mixed EN/CN content).
fn estimate_tokens(text: &str) -> usize { (text.chars().count() + 2) / 3 }

/// Estimate total tokens for a message history.
fn estimate_history_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| estimate_tokens(&m.content.as_text()) + 4)
        .sum()
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
    fn to_chat_message_text() {
        let msg = ChatMessage::user("hello");
        let converted = to_chat_message(&msg);
        assert!(matches!(
            converted,
            ChatCompletionRequestMessage::User(_)
        ));
    }

    #[test]
    fn to_chat_message_assistant() {
        let msg = ChatMessage::assistant("response");
        let converted = to_chat_message(&msg);
        assert!(matches!(
            converted,
            ChatCompletionRequestMessage::Assistant(_)
        ));
    }

    #[test]
    fn to_chat_message_system() {
        let msg = ChatMessage::system("you are helpful");
        let converted = to_chat_message(&msg);
        assert!(matches!(
            converted,
            ChatCompletionRequestMessage::System(_)
        ));
    }

    #[test]
    fn estimate_tokens_basic() {
        // "hello" = 5 chars -> (5 + 2) / 3 = 2 tokens
        assert_eq!(estimate_tokens("hello"), 2);
        // empty string -> (0 + 2) / 3 = 0 tokens
        assert_eq!(estimate_tokens(""), 0);
        // 300 chars -> (300 + 2) / 3 = 100 tokens
        let long = "a".repeat(300);
        assert_eq!(estimate_tokens(&long), 100);
    }

    #[test]
    fn estimate_history_tokens_sums_correctly() {
        let messages = vec![
            ChatMessage::user("hello"),       // 2 + 4 = 6
            ChatMessage::assistant("world!"), // 2 + 4 = 6
        ];
        let total = estimate_history_tokens(&messages);
        assert_eq!(total, 12);
    }
}
