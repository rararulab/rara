use std::sync::Arc;

use agent_core::{
    context::{
        self, AgentContext, CompletionFeatures, MemoryFeatures, PromptFeatures, SessionFeatures,
        SettingsFeatures, ToolFeatures,
    },
    model::LlmProviderLoaderRef,
    runner::{AgentRunner, MAX_ITERATIONS, UserContent},
    tool_registry::ToolRegistry,
};
use async_openai::types::chat::ChatCompletionRequestMessage;
use async_trait::async_trait;
use base::shared_string::SharedString;
use rara_domain_shared::settings::model::Settings;
use rara_mcp::{manager::mgr::McpManager, tool_bridge::McpToolBridge};
use rara_memory::{
    MemoryManager, RecallStrategyEngine,
    recall_engine as mem_recall,
};
use tokio::sync::watch;
use tracing::info;


// ---------------------------------------------------------------------------
// Conversions between agent_core::context types and rara_memory types
// ---------------------------------------------------------------------------

fn to_mem_event_kind(e: context::EventKind) -> mem_recall::EventKind {
    match e {
        context::EventKind::Compaction => mem_recall::EventKind::Compaction,
        context::EventKind::NewSession => mem_recall::EventKind::NewSession,
        context::EventKind::SessionResume => mem_recall::EventKind::SessionResume,
    }
}

fn from_mem_inject_target(t: mem_recall::InjectTarget) -> context::InjectTarget {
    match t {
        mem_recall::InjectTarget::SystemPrompt => context::InjectTarget::SystemPrompt,
        mem_recall::InjectTarget::ContextMessage => context::InjectTarget::ContextMessage,
    }
}

fn to_mem_recall_context(ctx: &context::RecallContext) -> mem_recall::RecallContext {
    mem_recall::RecallContext {
        user_text:               ctx.user_text.clone(),
        turn_count:              ctx.turn_count,
        events:                  ctx.events.iter().map(|e| to_mem_event_kind(*e)).collect(),
        elapsed_since_last_secs: ctx.elapsed_since_last_secs,
        summary:                 ctx.summary.clone(),
        session_topic:           ctx.session_topic.clone(),
    }
}

fn from_mem_injection_payload(p: mem_recall::InjectionPayload) -> context::InjectionPayload {
    context::InjectionPayload {
        rule_name: p.rule_name,
        target:    from_mem_inject_target(p.target),
        content:   p.content,
    }
}

// ---------------------------------------------------------------------------
// AgentContextImpl
// ---------------------------------------------------------------------------

/// Implements all agent context feature traits by assembling system prompts,
/// constructing tool registries, and managing conversation context.
#[derive(Clone)]
pub struct AgentContextImpl {
    llm_provider:   LlmProviderLoaderRef,
    tools:          Arc<ToolRegistry>,
    mcp_manager:    McpManager,
    skill_registry: rara_skills::registry::InMemoryRegistry,
    memory_manager: Option<Arc<MemoryManager>>,
    recall_engine:  Option<Arc<RecallStrategyEngine>>,
    settings_rx:    watch::Receiver<Settings>,
    prompt_repo:    Arc<dyn agent_core::prompt::PromptRepo>,
}

impl AgentContextImpl {
    #[must_use]
    pub fn new(
        llm_provider: LlmProviderLoaderRef,
        tools: Arc<ToolRegistry>,
        mcp_manager: McpManager,
        skill_registry: rara_skills::registry::InMemoryRegistry,
        memory_manager: Option<Arc<MemoryManager>>,
        recall_engine: Option<Arc<RecallStrategyEngine>>,
        settings_rx: watch::Receiver<Settings>,
        prompt_repo: Arc<dyn agent_core::prompt::PromptRepo>,
    ) -> Self {
        Self {
            llm_provider,
            tools,
            mcp_manager,
            skill_registry,
            memory_manager,
            recall_engine,
            settings_rx,
            prompt_repo,
        }
    }

    // -- inherent accessors (not part of traits) ----------------------------

    /// Return a snapshot of current settings.
    #[must_use]
    pub fn settings(&self) -> Settings { self.settings_rx.borrow().clone() }

    /// Return a reference to the recall engine (if configured).
    pub fn recall_engine_ref(&self) -> Option<&Arc<RecallStrategyEngine>> {
        self.recall_engine.as_ref()
    }

    /// Return a watch::Receiver clone for settings (used by ChatService).
    pub fn settings_rx(&self) -> watch::Receiver<Settings> {
        self.settings_rx.clone()
    }

    // -- legacy helpers (kept for backward compat) --------------------------

    /// Legacy memory injection -- kept for backward-compat with callers that
    /// don't yet provide a RecallContext. Will be removed once all callers
    /// are migrated to the recall engine.
    async fn legacy_memory_injection(
        &self,
        system_prompt: &mut String,
        user_text: &str,
        history_len: usize,
    ) {
        // Inject core user profile from mem0 facts.
        if let Some(ref mm) = self.memory_manager {
            if let Ok(facts) = mm.get_user_profile().await {
                if !facts.is_empty() {
                    let profile_section: String = facts
                        .iter()
                        .map(|m| format!("- {}", m.memory))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let old = std::mem::take(system_prompt);
                    *system_prompt = format!("# User Profile\n{profile_section}\n\n---\n\n{old}");
                }
            }
        }

        // Pre-fetch relevant memory context for new / short sessions,
        // or every turn when `recall_every_turn` is enabled.
        let recall_every_turn = self.settings_rx.borrow().agent.memory.recall_every_turn;
        if history_len < 3 || recall_every_turn {
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
                            recall_every_turn, "memory pre-fetch injected into system prompt"
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
    }
}

// ---------------------------------------------------------------------------
// CompletionFeatures
// ---------------------------------------------------------------------------

#[async_trait]
impl CompletionFeatures for AgentContextImpl {
    fn llm_provider(&self) -> &LlmProviderLoaderRef { &self.llm_provider }

    async fn build_chat_system_prompt(
        &self,
        base_prompt: &str,
        user_text: &str,
        history_len: usize,
        recall_ctx: Option<&context::RecallContext>,
    ) -> String {
        let soul = self
            .prompt_repo
            .get("agent/soul.md")
            .await
            .map(|e| e.content)
            .unwrap_or_default();

        let mut system_prompt = if soul.trim().is_empty() {
            base_prompt.to_owned()
        } else {
            format!("{soul}\n\n# Chat Instructions\n{base_prompt}")
        };

        // Run the recall engine if available, otherwise fall back to legacy
        // hardcoded behavior.
        if let Some(ctx) = recall_ctx {
            let payloads = MemoryFeatures::run_recall_engine(self, ctx).await;
            for payload in &payloads {
                if matches!(payload.target, context::InjectTarget::SystemPrompt) {
                    system_prompt.push_str(&format!(
                        "\n\n## Memory: {}\n{}",
                        payload.rule_name, payload.content
                    ));
                }
            }
            if !payloads.is_empty() {
                info!(
                    rules = payloads.len(),
                    "recall engine injected memory into system prompt"
                );
            }
        } else {
            self.legacy_memory_injection(&mut system_prompt, user_text, history_len)
                .await;
        }

        // Inject skills listing.
        let all_skills = self.skill_registry.list_all();
        let skills_xml = rara_skills::prompt_gen::generate_skills_prompt(&all_skills);
        if !skills_xml.is_empty() {
            system_prompt.push_str(&format!("\n\n{skills_xml}"));
        }

        system_prompt
    }

    async fn build_worker_policy(&self) -> String {
        let policy = self
            .prompt_repo
            .get("workers/agent_policy.md")
            .await
            .map(|e| e.content)
            .unwrap_or_default();
        let soul = self
            .prompt_repo
            .get("agent/soul.md")
            .await
            .map(|e| e.content)
            .unwrap_or_default();

        if soul.trim().is_empty() {
            policy
        } else {
            format!("{soul}\n\n# Operational Policy\n{policy}")
        }
    }

    fn build_runner(
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
            let max_iterations = settings.agent.max_iterations.map(|n| n as usize);
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

    async fn summarize_history(
        &self,
        history_text: &str,
        model: &str,
    ) -> agent_core::err::Result<String> {
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
            .await?;

        let summary = result
            .provider_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("[Summary unavailable]")
            .to_owned();

        Ok(summary)
    }
}

// ---------------------------------------------------------------------------
// ToolFeatures
// ---------------------------------------------------------------------------

#[async_trait]
impl ToolFeatures for AgentContextImpl {
    fn tools(&self) -> &Arc<ToolRegistry> { &self.tools }

    async fn build_effective_tools(&self) -> Arc<ToolRegistry> {
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
}

// ---------------------------------------------------------------------------
// PromptFeatures
// ---------------------------------------------------------------------------

impl PromptFeatures for AgentContextImpl {
    fn prompt_repo(&self) -> &Arc<dyn agent_core::prompt::PromptRepo> { &self.prompt_repo }
}

// ---------------------------------------------------------------------------
// MemoryFeatures
// ---------------------------------------------------------------------------

#[async_trait]
impl MemoryFeatures for AgentContextImpl {
    async fn run_recall_engine(
        &self,
        ctx: &context::RecallContext,
    ) -> Vec<context::InjectionPayload> {
        let (Some(engine), Some(mm)) = (&self.recall_engine, &self.memory_manager) else {
            return vec![];
        };
        let mem_ctx = to_mem_recall_context(ctx);
        engine
            .run(&mem_ctx, mm)
            .await
            .into_iter()
            .map(from_mem_injection_payload)
            .collect()
    }

    fn spawn_session_consolidation(&self, exchanges: Vec<(String, String)>) {
        let Some(ref mm) = self.memory_manager else {
            return;
        };

        let mm = Arc::clone(mm);

        tokio::spawn(async move {
            if let Err(e) = mm.consolidate_session(&exchanges).await {
                tracing::warn!(error = %e, "session consolidation failed");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// SettingsFeatures
// ---------------------------------------------------------------------------

impl SettingsFeatures for AgentContextImpl {
    fn model_for_key(&self, key: &str) -> String {
        self.settings_rx.borrow().ai.model_for_key(key)
    }

    fn current_default_model(&self) -> String { self.model_for_key("chat") }

    fn provider_hint(&self) -> Option<String> {
        self.settings_rx.borrow().ai.provider.clone()
    }

    fn max_iterations(&self, _key: &str) -> usize {
        self.settings_rx
            .borrow()
            .agent
            .max_iterations
            .map(|n| n as usize)
            .unwrap_or(MAX_ITERATIONS)
    }

    fn fallback_models(&self) -> Vec<SharedString> {
        self.settings_rx
            .borrow()
            .ai
            .fallback_models
            .iter()
            .map(|s| s.clone().into())
            .collect()
    }

    fn needs_compaction(&self, history_tokens: usize, context_length: usize) -> bool {
        let threshold = (context_length as f64 * 0.80) as usize;
        history_tokens > threshold
    }
}

// ---------------------------------------------------------------------------
// SessionFeatures
// ---------------------------------------------------------------------------

#[async_trait]
impl SessionFeatures for AgentContextImpl {
    async fn current_system_prompt(&self) -> String {
        let base_prompt = self
            .prompt_repo
            .get("chat/default_system.md")
            .await
            .map(|e| e.content)
            .unwrap_or_default();
        let soul = self
            .prompt_repo
            .get("agent/soul.md")
            .await
            .map(|e| e.content)
            .unwrap_or_default();

        if soul.trim().is_empty() {
            base_prompt
        } else {
            format!("{soul}\n\n# Chat Instructions\n{base_prompt}")
        }
    }
}

// ---------------------------------------------------------------------------
// Debug
// ---------------------------------------------------------------------------

impl std::fmt::Debug for AgentContextImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentContextImpl")
            .field("default_model", &SettingsFeatures::current_default_model(self))
            .finish_non_exhaustive()
    }
}

// Ensure AgentContextImpl satisfies AgentContext (compile-time check).
const _: () = {
    fn _assert_agent_context<T: AgentContext>() {}
    fn _check() { _assert_agent_context::<AgentContextImpl>() }
};
