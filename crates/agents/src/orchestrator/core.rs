use std::sync::Arc;

use async_openai::types::chat::ChatCompletionRequestMessage;
use rara_domain_shared::settings::model::Settings;
use rara_mcp::{manager::mgr::McpManager, tool_bridge::McpToolBridge};
use rara_memory::MemoryManager;
use rara_sessions::types::ChatMessage;
use tokio::sync::watch;
use tracing::info;

use agent_core::{
    model::LlmProviderLoaderRef,
    runner::{AgentRunner, MAX_ITERATIONS, UserContent},
    tool_registry::ToolRegistry,
};

use super::{
    context::estimate_history_tokens,
    error::OrchestratorError,
};

/// Orchestrates agent creation and execution by assembling system prompts,
/// constructing tool registries, and managing conversation context.
#[derive(Clone)]
pub struct AgentOrchestrator {
    llm_provider:   LlmProviderLoaderRef,
    tools:          Arc<ToolRegistry>,
    mcp_manager:    McpManager,
    skill_registry: rara_skills::registry::InMemoryRegistry,
    memory_manager: Option<Arc<MemoryManager>>,
    settings_rx:    watch::Receiver<Settings>,
    prompt_repo:    Arc<dyn agent_core::prompt::PromptRepo>,
}

impl AgentOrchestrator {
    #[must_use]
    pub fn new(
        llm_provider: LlmProviderLoaderRef,
        tools: Arc<ToolRegistry>,
        mcp_manager: McpManager,
        skill_registry: rara_skills::registry::InMemoryRegistry,
        memory_manager: Option<Arc<MemoryManager>>,
        settings_rx: watch::Receiver<Settings>,
        prompt_repo: Arc<dyn agent_core::prompt::PromptRepo>,
    ) -> Self {
        Self {
            llm_provider,
            tools,
            mcp_manager,
            skill_registry,
            memory_manager,
            settings_rx,
            prompt_repo,
        }
    }

    // -- prompt construction ------------------------------------------------

    pub async fn build_chat_system_prompt(
        &self,
        base_prompt: &str,
        user_text: &str,
        history_len: usize,
    ) -> String {
        let soul = self.prompt_repo.get("agent/soul.md").await
            .map(|e| e.content)
            .unwrap_or_default();

        let mut system_prompt = if soul.trim().is_empty() {
            base_prompt.to_owned()
        } else {
            format!("{soul}\n\n# Chat Instructions\n{base_prompt}")
        };

        // Inject core user profile from mem0 facts.
        if let Some(ref mm) = self.memory_manager {
            if let Ok(facts) = mm.get_user_profile().await {
                if !facts.is_empty() {
                    let profile_section: String = facts
                        .iter()
                        .map(|m| format!("- {}", m.memory))
                        .collect::<Vec<_>>()
                        .join("\n");
                    system_prompt = format!(
                        "# User Profile\n{profile_section}\n\n---\n\n{system_prompt}"
                    );
                }
            }
        }

        // Pre-fetch relevant memory context for new / short sessions.
        if history_len < 3 {
            if let Some(ref mm) = self.memory_manager {
                match mm.search(user_text, 5).await {
                    Ok(results) if !results.is_empty() => {
                        system_prompt.push_str("\n\n## Relevant Memory Context\n");
                        for hit in &results {
                            system_prompt
                                .push_str(&format!("- [{:?}] {}\n", hit.source, hit.content));
                        }
                        info!(
                            hits = results.len(),
                            "memory pre-fetch injected into system prompt"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "memory pre-fetch failed, continuing without context"
                        );
                    }
                }
            }
        }

        // Inject skills listing.
        let all_skills = self.skill_registry.list_all();
        let skills_xml = rara_skills::prompt_gen::generate_skills_prompt(&all_skills);
        if !skills_xml.is_empty() {
            system_prompt.push_str(&format!("\n\n{skills_xml}"));
        }

        system_prompt
    }

    pub async fn build_worker_policy(&self) -> String {
        let policy = self.prompt_repo.get("workers/agent_policy.md").await
            .map(|e| e.content)
            .unwrap_or_default();
        let soul = self.prompt_repo.get("agent/soul.md").await
            .map(|e| e.content)
            .unwrap_or_default();

        if soul.trim().is_empty() {
            policy
        } else {
            format!("{soul}\n\n# Operational Policy\n{policy}")
        }
    }

    // -- tool construction --------------------------------------------------

    pub async fn build_effective_tools(&self) -> Arc<ToolRegistry> {
        let mut registry = self.tools.filtered(&[]);

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
    }

    // -- runner construction ------------------------------------------------

    pub fn build_runner(
        &self,
        model: String,
        system_prompt: String,
        user_content: UserContent,
        chat_history: Vec<ChatCompletionRequestMessage>,
    ) -> AgentRunner {
        let (provider_hint, fallback_models, max_iterations) = {
            let settings = self.settings_rx.borrow();
            let provider_hint = settings.ai.provider.clone();
            let fallback_models = settings
                .ai
                .fallback_models
                .iter()
                .map(|s| s.clone().into())
                .collect();
            let max_iterations = settings.agent.max_iterations
                .map(|n| n as usize);
            (provider_hint, fallback_models, max_iterations)
        };

        AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .provider_hint(provider_hint.unwrap_or_default())
            .model_name(model)
            .system_prompt(system_prompt)
            .user_content(user_content)
            .history(chat_history)
            .max_iterations(max_iterations.unwrap_or(MAX_ITERATIONS))
            .fallback_models(fallback_models)
            .build()
    }

    // -- context management -------------------------------------------------

    pub async fn summarize_history(
        &self,
        history: &[ChatMessage],
        model: &str,
    ) -> Result<ChatMessage, OrchestratorError> {
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

        let runner = AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .provider_hint(
                self.settings_rx
                    .borrow()
                    .ai
                    .provider
                    .clone()
                    .unwrap_or_default(),
            )
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
            .map_err(|e| OrchestratorError::AgentError {
                message: format!("summarization failed: {e}"),
            })?;

        let summary = result
            .provider_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("[Summary unavailable]")
            .to_owned();

        Ok(ChatMessage::assistant(format!(
            "[Conversation Summary]\n{summary}"
        )))
    }

    #[must_use]
    pub fn needs_compaction(&self, history: &[ChatMessage], context_length: usize) -> bool {
        let tokens = estimate_history_tokens(history);
        let threshold = (context_length as f64 * 0.80) as usize;
        tokens > threshold
    }

    // -- memory reflection --------------------------------------------------

    pub fn spawn_memory_reflection(&self, user_text: &str, assistant_text: &str) {
        let Some(ref mm) = self.memory_manager else {
            return;
        };

        let mm = Arc::clone(mm);
        let user_text = user_text.to_owned();
        let assistant_text = assistant_text.to_owned();

        tokio::spawn(async move {
            if let Err(e) = mm.reflect_on_exchange(&user_text, &assistant_text).await {
                tracing::warn!(error = %e, "memory reflection failed");
            }
        });
    }

    // -- accessors ----------------------------------------------------------

    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.tools
    }

    pub fn llm_provider(&self) -> &LlmProviderLoaderRef {
        &self.llm_provider
    }

    /// Resolve the model for a given key from settings.
    ///
    /// Falls back to the `"default"` key, then to `"openai/gpt-4o"`.
    #[must_use]
    pub fn model_for_key(&self, key: &str) -> String {
        self.settings_rx.borrow().ai.model_for_key(key)
    }

    #[must_use]
    pub fn current_default_model(&self) -> String {
        self.model_for_key("chat")
    }

    #[must_use]
    pub fn settings(&self) -> Settings {
        self.settings_rx.borrow().clone()
    }

    /// Resolve the current system prompt asynchronously.
    ///
    /// Loads the base prompt and soul from the prompt repo and composes them.
    pub async fn current_system_prompt(&self) -> String {
        let base_prompt = self.prompt_repo.get("chat/default_system.md").await
            .map(|e| e.content)
            .unwrap_or_default();
        let soul = self.prompt_repo.get("agent/soul.md").await
            .map(|e| e.content)
            .unwrap_or_default();

        if soul.trim().is_empty() {
            base_prompt
        } else {
            format!("{soul}\n\n# Chat Instructions\n{base_prompt}")
        }
    }

    /// Return a reference to the prompt repository.
    pub fn prompt_repo(&self) -> &Arc<dyn agent_core::prompt::PromptRepo> {
        &self.prompt_repo
    }
}

impl std::fmt::Debug for AgentOrchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOrchestrator")
            .field("default_model", &self.current_default_model())
            .finish_non_exhaustive()
    }
}
