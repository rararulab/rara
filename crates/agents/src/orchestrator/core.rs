use std::sync::Arc;

use async_openai::types::chat::ChatCompletionRequestMessage;
use rara_domain_shared::settings::model::{ModelScenario, Settings};
use rara_mcp::{manager::mgr::McpManager, tool_bridge::McpToolBridge};
use rara_memory::MemoryManager;
use rara_sessions::types::ChatMessage;
use tokio::sync::watch;
use tracing::info;

use crate::{
    model::LlmProviderLoaderRef,
    runner::{AgentRunner, UserContent},
    tool_registry::ToolRegistry,
};

use super::{
    context::estimate_history_tokens,
    error::OrchestratorError,
    prompt::{compose_system_prompt, resolve_soul_prompt},
    reflection,
};

/// Default system prompt used when no custom prompt is configured.
const SYSTEM_PROMPT_FILE: &str = "chat/default_system.md";
const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../../../../prompts/chat/default_system.md");

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
    ) -> Self {
        Self {
            llm_provider,
            tools,
            mcp_manager,
            skill_registry,
            memory_manager,
            settings_rx,
        }
    }

    // -- prompt construction ------------------------------------------------

    pub async fn build_chat_system_prompt(
        &self,
        base_prompt: &str,
        user_text: &str,
        history_len: usize,
    ) -> String {
        let soul_prompt = {
            let settings = self.settings_rx.borrow();
            resolve_soul_prompt(&settings)
        };
        let mut system_prompt = compose_system_prompt(base_prompt, soul_prompt.as_deref());

        // Inject core user profile.
        if let Some(ref mm) = self.memory_manager {
            if let Ok(profile) = mm.read_core_profile().await {
                if !profile.trim().is_empty() {
                    system_prompt = format!("{profile}\n\n---\n\n{system_prompt}");
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
                                .push_str(&format!("- [{}] {}\n", hit.path, hit.snippet));
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
        let settings = self.settings_rx.borrow().clone();
        super::prompt::load_agent_policy(&settings).await
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
        let fallback_models = {
            let settings = self.settings_rx.borrow();
            let chain = settings.ai.fallback_chain(ModelScenario::Chat);
            chain
                .into_iter()
                .skip(1)
                .map(|s| s.to_owned().into())
                .collect()
        };

        AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .model_name(model)
            .system_prompt(system_prompt)
            .user_content(user_content)
            .history(chat_history)
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
        let llm = self.llm_provider.clone();
        let tools = Arc::clone(&self.tools);
        let model = self.current_default_model();
        let user_text = user_text.to_owned();
        let assistant_text = assistant_text.to_owned();

        tokio::spawn(async move {
            if let Err(e) = reflection::memory_reflection(
                &mm,
                &llm,
                &tools,
                &model,
                &user_text,
                &assistant_text,
            )
            .await
            {
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

    #[must_use]
    pub fn current_default_model(&self) -> String {
        self.settings_rx
            .borrow()
            .ai
            .model_for(ModelScenario::Chat)
            .to_owned()
    }

    #[must_use]
    pub fn settings(&self) -> Settings {
        self.settings_rx.borrow().clone()
    }

    #[must_use]
    pub fn current_system_prompt(&self) -> String {
        let settings = self.settings_rx.borrow();
        let base_prompt =
            rara_paths::load_prompt_markdown(SYSTEM_PROMPT_FILE, DEFAULT_SYSTEM_PROMPT);
        let soul_prompt = resolve_soul_prompt(&settings);
        compose_system_prompt(&base_prompt, soul_prompt.as_deref())
    }
}

impl std::fmt::Debug for AgentOrchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOrchestrator")
            .field("default_model", &self.current_default_model())
            .finish_non_exhaustive()
    }
}
