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

//! Unified application state shared by workers and routes.

use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use common_worker::IntervalOrNotifyHandle;
use opendal::Operator;
use snafu::{ResultExt, Whatever};
use tracing::info;
use yunara_store::db::DBStore;

/// Shared application state used by workers and HTTP routes.
#[derive(Clone)]
pub struct AppState {
    // -- AI --
    pub ai_service: rara_backend_admin::ai_tasks::TaskAgentService,

    // -- domain services --
    pub resume_service:      rara_backend_admin::resume::ResumeAppService,
    pub application_service: rara_backend_admin::application::service::ApplicationService,
    pub interview_service:   rara_backend_admin::interview::service::InterviewService,
    pub scheduler_service:   rara_backend_admin::scheduler::service::SchedulerService,
    pub analytics_service:   rara_backend_admin::analytics::service::AnalyticsService,
    pub job_service:         rara_backend_admin::job::service::JobService,
    pub chat_service:        rara_domain_chat::service::ChatService,
    // -- shared --
    pub settings_svc:        rara_backend_admin::settings::SettingsSvc,
    pub notify_client:       rara_domain_shared::notify::client::NotifyClient,
    pub contact_repo:        rara_telegram_bot::contacts::repository::ContactRepository,

    // -- LLM provider --
    pub llm_provider: rara_kernel::provider::LlmProviderLoaderRef,

    // -- infra --
    pub object_store: Operator,

    // -- memory --
    pub memory_manager: Arc<rara_memory::MemoryManager>,

    // -- agent scheduler --
    pub agent_scheduler: Arc<crate::agent_scheduler::AgentScheduler>,

    // -- skills --
    pub skill_registry: rara_skills::registry::InMemoryRegistry,

    // -- MCP --
    pub mcp_manager: rara_mcp::manager::mgr::McpManager,

    // -- agent context --
    pub agent_ctx: std::sync::Arc<dyn rara_kernel::agent_context::AgentContext>,

    // -- pipeline --
    pub pipeline_service: rara_backend_admin::pipeline::service::PipelineService,

    // -- coding tasks --
    pub coding_task_service: rara_coding_task::service::CodingTaskService,

    // -- dispatcher --
    pub dispatcher: Arc<rara_kernel::dispatcher::AgentDispatcher>,

    // -- prompt repo --
    pub prompt_repo: Arc<dyn rara_kernel::prompt::PromptRepo>,

    // -- worker coordination --
    pub proactive_notify: Arc<RwLock<Option<IntervalOrNotifyHandle>>>,
}

impl AppState {
    /// Initialize all domain services and build the shared application state.
    pub async fn init(
        db_store: &DBStore,
        object_store: Operator,
        notify_client: rara_domain_shared::notify::client::NotifyClient,
        mem0_base_url: String,
        memos_base_url: String,
        memos_token: String,
        hindsight_base_url: String,
        hindsight_bank_id: String,
    ) -> Result<Self, Whatever> {
        let pool = db_store.pool().clone();

        // -- runtime settings ------------------------------------------------

        let settings_svc = rara_backend_admin::settings::SettingsSvc::load(db_store.kv_store())
            .await
            .whatever_context("Failed to initialize runtime settings")?;
        info!("Runtime settings service loaded");

        // -- prompt repo -------------------------------------------------------

        let prompt_repo: Arc<dyn rara_kernel::prompt::PromptRepo> = Arc::new(
            rara_kernel::prompt::BuiltinPromptRepo::new(rara_kernel::prompt::all_builtin_prompts()),
        );
        info!("Prompt repository initialized");

        // -- LLM provider ----------------------------------------------------

        let llm_provider: rara_kernel::provider::LlmProviderLoaderRef =
            Arc::new(SettingsLlmProviderLoader::new(settings_svc.clone()));

        // -- AI task agents --------------------------------------------------

        let ai_service = rara_backend_admin::ai_tasks::TaskAgentService::new(
            settings_svc.subscribe(),
            llm_provider.clone(),
            prompt_repo.clone(),
        );

        // -- domain services -------------------------------------------------

        let resume_service = rara_backend_admin::resume::wire_resume_service(pool.clone());
        let application_service = rara_backend_admin::application::wire(pool.clone());
        let interview_service = rara_backend_admin::interview::wire_interview_service(pool.clone());
        let scheduler_service = rara_backend_admin::scheduler::wire_scheduler_service(pool.clone());
        let analytics_service = rara_backend_admin::analytics::wire_analytics_service(pool.clone());
        let job_service =
            rara_backend_admin::job::wire_job_service(pool.clone(), ai_service.clone())
                .whatever_context("Failed to initialize job service")?;
        info!("Job service initialized");

        // -- chat service ----------------------------------------------------

        let session_repo = Arc::new(
            rara_sessions::pg_repository::PgSessionRepository::new(
                pool.clone(),
                rara_paths::sessions_dir(),
            )
            .await
            .whatever_context("Failed to initialize session repository")?,
        );
        let composio_auth_provider: Arc<dyn rara_composio::ComposioAuthProvider> =
            Arc::new(SettingsComposioAuthProvider::new(settings_svc.clone()));
        let contact_repo =
            rara_telegram_bot::contacts::repository::ContactRepository::new(pool.clone());
        let contact_lookup: Arc<dyn tool_core::contact_lookup::ContactLookup> =
            Arc::new(contact_repo.clone());
        let mut tool_registry = rara_kernel::tool::ToolRegistry::new();
        for tool in tool_core::default_primitives(tool_core::PrimitiveDeps {
            pool:                   pool.clone(),
            notify_client:          notify_client.clone(),
            settings_rx:            settings_svc.subscribe(),
            object_store:           object_store.clone(),
            composio_auth_provider: composio_auth_provider.clone(),
            contact_lookup:         contact_lookup.clone(),
        }) {
            tool_registry.register_primitive(tool);
        }
        // mem0 now always uses the configured service URL (Consul/runtime settings),
        // never the legacy on-demand pod mode.
        info!("mem0 using direct connection to {}", mem0_base_url);
        let mem0 = rara_memory::Mem0Client::new(mem0_base_url);
        let memos = rara_memory::MemosClient::new(memos_base_url, memos_token);
        let hindsight = rara_memory::HindsightClient::new(hindsight_base_url, hindsight_bank_id);
        let memory_manager = Arc::new(rara_memory::MemoryManager::new(
            mem0,
            memos,
            hindsight,
            "default".to_owned(),
        ));
        info!("memory manager initialized");

        // Layer 2: Services
        tool_registry.register_service(Arc::new(crate::tools::services::MemorySearchTool::new(
            Arc::clone(&memory_manager),
        )));
        tool_registry.register_service(Arc::new(
            crate::tools::services::MemoryDeepRecallTool::new(Arc::clone(&memory_manager)),
        ));
        tool_registry.register_service(Arc::new(crate::tools::services::MemoryWriteTool::new(
            Arc::clone(&memory_manager),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::MemoryAddFactTool::new(
            Arc::clone(&memory_manager),
        )));
        // -- agent scheduler -------------------------------------------------
        let agent_scheduler = Arc::new(crate::agent_scheduler::AgentScheduler::new(
            rara_paths::agent_jobs_file().clone(),
        ));
        agent_scheduler.load().await.ok(); // tolerate missing file
        info!("Agent scheduler loaded");

        tool_registry.register_service(Arc::new(crate::tools::services::ScheduleAddTool::new(
            agent_scheduler.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::ScheduleListTool::new(
            agent_scheduler.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::ScheduleRemoveTool::new(
            agent_scheduler.clone(),
        )));

        // -- codex agent dispatch (PG-backed via rara-coding-task) --------
        let workspace_manager =
            rara_workspace::WorkspaceManager::new(rara_paths::workspaces_dir().clone());
        let default_repo_url = std::env::var("RARA_DEFAULT_REPO_URL")
            .unwrap_or_else(|_| "https://github.com/crrow/job".to_owned());
        let coding_task_service = rara_coding_task::service::wire(
            pool.clone(),
            workspace_manager,
            notify_client.clone(),
            settings_svc.subscribe(),
            default_repo_url,
        );
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        tool_registry.register_service(Arc::new(crate::tools::services::CodexRunTool::new(
            coding_task_service.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::ScreenshotTool::new(
            notify_client.clone(),
            settings_svc.subscribe(),
            project_root,
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::CodexStatusTool::new(
            coding_task_service.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::CodexListTool::new(
            coding_task_service.clone(),
        )));

        // -- skills registry (PG cache + incremental FS sync) --------------------
        let skill_registry = rara_skills::registry::InMemoryRegistry::new();
        rara_skills::cache::spawn_background_sync(pool.clone(), skill_registry.clone());

        tool_registry.register_service(Arc::new(crate::tools::services::ListSkillsTool::new(
            skill_registry.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::CreateSkillTool::new(
            skill_registry.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::DeleteSkillTool::new(
            skill_registry.clone(),
        )));

        // -- MCP manager -------------------------------------------------------

        let mcp_registry_path = rara_paths::config_dir().join("mcp-servers.json");
        let mcp_registry = rara_mcp::manager::registry::FSMcpRegistry::load(&mcp_registry_path)
            .await
            .whatever_context("Failed to load MCP registry")?;
        let mcp_manager = rara_mcp::manager::mgr::McpManager::new(
            Arc::new(mcp_registry),
            rara_mcp::oauth::OAuthCredentialsStoreMode::default(),
        );
        let started = mcp_manager.start_enabled().await;
        if started.is_empty() {
            info!("No MCP servers to start");
        } else {
            info!(servers = ?started, "MCP servers started");
        }

        // -- MCP management tools -----------------------------------------------
        tool_registry.register_service(Arc::new(
            crate::tools::services::InstallMcpServerTool::new(mcp_manager.clone()),
        ));
        tool_registry.register_service(Arc::new(crate::tools::services::ListMcpServersTool::new(
            mcp_manager.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::RemoveMcpServerTool::new(
            mcp_manager.clone(),
        )));

        // -- pipeline service ---------------------------------------------------

        let pipeline_service = rara_backend_admin::pipeline::service::PipelineService::new(
            settings_svc.clone(),
            llm_provider.clone(),
            ai_service.clone(),
            job_service.clone(),
            pool.clone(),
            notify_client.clone(),
            composio_auth_provider,
            mcp_manager.clone(),
            prompt_repo.clone(),
            contact_lookup,
        );
        info!("Pipeline service initialized");

        // Register pipeline control tools on the main rara agent.
        rara_backend_admin::pipeline::register_rara_tools(
            &mut tool_registry,
            &pipeline_service,
            &settings_svc,
        );

        // -- subagent tool -------------------------------------------------------

        // Snapshot current tools BEFORE adding SubagentTool (prevents recursion).
        let subagent_parent_tools = Arc::new(tool_registry.filtered(&[]));

        // Load agent definitions: bundled first, then user-defined (override).
        let agent_defs = {
            let mut registry = rara_kernel::subagent::AgentDefinitionRegistry::new();
            // Bundled agent definitions (embedded at compile time)
            for def in rara_kernel::subagent::all_bundled_agents() {
                registry.register(def);
            }
            // User-defined agent definitions (override bundled)
            let user_dir = rara_paths::data_dir().join("agents");
            if let Ok(user) = rara_kernel::subagent::AgentDefinitionRegistry::load_dir(&user_dir) {
                for def in user.list() {
                    registry.register(def.clone());
                }
            }
            let count = registry.list().len();
            if count > 0 {
                info!(count, "agent definitions loaded");
            }
            Arc::new(registry)
        };

        // Default model for sub-agents: use the chat model from settings.
        let subagent_default_model = {
            let s = settings_svc.current();
            s.ai.model_for_key("chat")
        };

        tool_registry.register_service(Arc::new(rara_kernel::subagent::SubagentTool::new(
            llm_provider.clone(),
            agent_defs,
            subagent_parent_tools,
            subagent_default_model,
        )));

        // -- recall strategy engine -------------------------------------------

        let recall_engine = Arc::new(rara_memory::RecallStrategyEngine::new(
            rara_memory::recall_engine::default_rules(),
        ));
        info!("Recall strategy engine initialized with default rules");

        // Register recall strategy tools.
        tool_registry.register_service(Arc::new(
            crate::tools::services::RecallStrategyAddTool::new(Arc::clone(&recall_engine)),
        ));
        tool_registry.register_service(Arc::new(
            crate::tools::services::RecallStrategyListTool::new(Arc::clone(&recall_engine)),
        ));
        tool_registry.register_service(Arc::new(
            crate::tools::services::RecallStrategyUpdateTool::new(Arc::clone(&recall_engine)),
        ));
        tool_registry.register_service(Arc::new(
            crate::tools::services::RecallStrategyRemoveTool::new(Arc::clone(&recall_engine)),
        ));

        let tools = Arc::new(tool_registry);

        let agent_ctx: Arc<dyn rara_kernel::agent_context::AgentContext> = Arc::new(
            crate::orchestrator::AgentContextImpl::new(
                llm_provider.clone(),
                tools.clone(),
                mcp_manager.clone(),
                skill_registry.clone(),
                Some(Arc::clone(&memory_manager)),
                Some(recall_engine),
                settings_svc.subscribe(),
                prompt_repo.clone(),
            ),
        );

        let chat_agent = rara_domain_chat::agent::ChatAgent::new(Arc::clone(&agent_ctx));
        let chat_service = rara_domain_chat::service::ChatService::new(
            session_repo,
            Arc::new(settings_svc.clone())
                as Arc<dyn rara_domain_shared::settings::SettingsUpdater>,
            settings_svc.subscribe(),
            chat_agent,
        );
        info!("Chat service initialized");

        // -- agent dispatcher ---------------------------------------------------

        let session_persister: Arc<dyn crate::task_executor::SessionPersister> =
            Arc::new(ChatServicePersister(chat_service.clone()));
        let job_callback: Arc<dyn crate::task_executor::ScheduledJobCallback> =
            Arc::new(AgentSchedulerCallback(agent_scheduler.clone()));
        let executor: Arc<dyn rara_kernel::dispatcher::TaskExecutor> =
            Arc::new(crate::task_executor::WorkerTaskExecutor::new(
                Arc::clone(&agent_ctx),
                session_persister,
                job_callback,
            ));
        let log_store: Arc<dyn rara_kernel::dispatcher::DispatcherLogStore> =
            Arc::new(rara_kernel::dispatcher::InMemoryLogStore::new(200));
        let dispatcher = Arc::new(rara_kernel::dispatcher::AgentDispatcher::new(
            executor,
            log_store,
        ));
        info!("Agent dispatcher initialized");

        Ok(Self {
            ai_service,
            resume_service,
            application_service,
            interview_service,
            scheduler_service,
            analytics_service,
            job_service,
            chat_service,
            settings_svc,
            notify_client,
            contact_repo,
            llm_provider,
            object_store,
            memory_manager,
            agent_scheduler,
            skill_registry,
            mcp_manager,
            agent_ctx,
            pipeline_service,
            coding_task_service,
            dispatcher,
            prompt_repo,
            proactive_notify: Arc::new(RwLock::new(None)),
        })
    }

    /// Build all domain API routes and the OpenAPI spec.
    pub fn routes(&self) -> (axum::Router, utoipa::openapi::OpenApi) {
        let mut api = Self::api_doc();

        let mut router = axum::Router::new();
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::resume::routes(self.resume_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::application::routes(self.application_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::interview::routes(self.interview_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::scheduler::routes(self.scheduler_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::analytics::routes(self.analytics_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::job::discovery_routes(self.job_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::job::bot_routes(self.job_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::settings::routes(self.settings_svc.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::contacts::routes(self.contact_repo.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::chat::routes(self.chat_service.clone()),
        );
        merge_openapi_router(&mut router, &mut api, crate::system_routes::routes());
        merge_openapi_router(
            &mut router,
            &mut api,
            crate::agent_scheduler_routes::routes(self.agent_scheduler.clone()),
        );

        // skill_routes returns a plain axum::Router (no OpenAPI metadata).
        router = router.merge(rara_backend_admin::skills::skill_routes(
            self.skill_registry.clone(),
        ));

        // MCP admin routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(rara_backend_admin::mcp::mcp_router(
            self.mcp_manager.clone(),
        ));

        // Coding task routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(rara_backend_admin::coding_task::routes(
            self.coding_task_service.clone(),
        ));

        // Pipeline routes (OpenAPI).
        let (pipeline_router, pipeline_api) =
            rara_backend_admin::pipeline::routes(self.pipeline_service.clone()).split_for_parts();
        router = router.merge(pipeline_router);
        api.merge(pipeline_api);

        // Model admin routes (OpenAPI).
        let model_repo: std::sync::Arc<dyn rara_kernel::model_repo::ModelRepo> = std::sync::Arc::new(
            rara_backend_admin::models::SettingsModelRepo::new(self.settings_svc.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_backend_admin::models::routes(model_repo),
        );

        // Dispatcher routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(crate::dispatcher_routes::dispatcher_router(
            self.dispatcher.clone(),
        ));

        // Prompt admin routes.
        let (prompt_router, prompt_api) =
            rara_backend_admin::prompts::routes(self.prompt_repo.clone()).split_for_parts();
        router = router.merge(prompt_router);
        api.merge(prompt_api);

        (router, api)
    }

    fn api_doc() -> utoipa::openapi::OpenApi {
        use utoipa::OpenApi;
        #[derive(OpenApi)]
        #[openapi(
            info(
                title = "Rara API",
                description = "AI Job Automation Platform API",
                version = "0.0.17"
            ),
            tags(
                (name = "applications", description = "Application lifecycle management"),
                (name = "chat", description = "Chat sessions and messaging"),
                (name = "resumes", description = "Resume management"),
                (name = "interviews", description = "Interview management"),
                (name = "jobs", description = "Job discovery and management"),
                (name = "scheduler", description = "Task scheduling"),
                (name = "analytics", description = "Analytics and metrics"),
                (name = "settings", description = "Runtime settings"),
                (name = "notifications", description = "Notification queue"),
                (name = "contacts", description = "Telegram contacts allowlist"),
                (name = "system", description = "System utilities")
            )
        )]
        struct ApiDoc;
        ApiDoc::openapi()
    }
}

fn merge_openapi_router(
    router: &mut axum::Router,
    api: &mut utoipa::openapi::OpenApi,
    domain_router: utoipa_axum::router::OpenApiRouter,
) {
    let (r, a) = domain_router.split_for_parts();
    *router = std::mem::take(router).merge(r);
    api.merge(a);
}

// ---------------------------------------------------------------------------
// SettingsLlmProviderLoader
// ---------------------------------------------------------------------------

/// [`LlmProviderLoader`](rara_kernel::provider::LlmProviderLoader)
/// implementation that reads the API key from
/// [`SettingsSvc`](rara_backend_admin::settings::SettingsSvc) runtime settings
/// rather than from environment variables.
///
/// A fresh [`OpenAiProvider`](rara_kernel::provider::OpenAiProvider) is created
/// on every call so that runtime API-key changes take effect immediately.
struct SettingsLlmProviderLoader {
    settings:           rara_backend_admin::settings::SettingsSvc,
    codex_refresh_lock: Arc<tokio::sync::Mutex<()>>,
}

/// Composio auth provider that reads credentials from runtime settings.
#[derive(Clone)]
struct SettingsComposioAuthProvider {
    settings: rara_backend_admin::settings::SettingsSvc,
}

impl SettingsComposioAuthProvider {
    fn new(settings: rara_backend_admin::settings::SettingsSvc) -> Self { Self { settings } }
}

#[async_trait]
impl rara_composio::ComposioAuthProvider for SettingsComposioAuthProvider {
    async fn acquire_auth(&self) -> anyhow::Result<rara_composio::ComposioAuth> {
        let current = self.settings.current();
        let composio = current.agent.composio;
        let api_key = composio
            .api_key
            .ok_or_else(|| anyhow::anyhow!("composio.api_key is not configured in settings"))?;
        Ok(rara_composio::ComposioAuth::new(
            api_key,
            composio.entity_id.as_deref(),
        ))
    }
}

impl SettingsLlmProviderLoader {
    fn new(settings: rara_backend_admin::settings::SettingsSvc) -> Self {
        Self {
            settings,
            codex_refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatcher trait adapters
// ---------------------------------------------------------------------------

/// Adapter that implements [`SessionPersister`] by delegating to
/// [`ChatService`].
struct ChatServicePersister(rara_domain_chat::service::ChatService);

#[async_trait]
impl crate::task_executor::SessionPersister for ChatServicePersister {
    async fn persist_messages(
        &self,
        session_key: &str,
        user_text: &str,
        assistant_text: &str,
    ) -> Result<(), String> {
        let key = rara_sessions::types::SessionKey::from_raw(session_key);
        self.0
            .append_messages(&key, user_text, assistant_text)
            .await
            .map_err(|e| e.to_string())
    }

    async fn persist_role_message(
        &self,
        session_key: &str,
        role: &str,
        text: &str,
    ) -> Result<(), String> {
        let key = rara_sessions::types::SessionKey::from_raw(session_key);
        let msg = match role {
            "user" => rara_sessions::types::ChatMessage::user(text),
            "assistant" => rara_sessions::types::ChatMessage::assistant(text),
            "system" => rara_sessions::types::ChatMessage::system(text),
            _ => rara_sessions::types::ChatMessage::user(text),
        };
        self.0
            .append_message_raw(&key, &msg)
            .await
            .map_err(|e| e.to_string())
            .map(|_| ())
    }

    async fn ensure_session(&self, session_key: &str) {
        let key = rara_sessions::types::SessionKey::from_raw(session_key);
        let _ = self.0.ensure_session(&key, None, None, None).await;
    }
}

/// Adapter that implements [`ScheduledJobCallback`] by delegating to
/// [`AgentScheduler`].
struct AgentSchedulerCallback(Arc<crate::agent_scheduler::AgentScheduler>);

#[async_trait]
impl crate::task_executor::ScheduledJobCallback for AgentSchedulerCallback {
    async fn mark_executed(&self, job_id: &str) -> Result<(), String> {
        self.0
            .mark_executed(job_id)
            .await
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl rara_kernel::provider::LlmProviderLoader for SettingsLlmProviderLoader {
    async fn acquire_provider(
        &self,
    ) -> rara_kernel::error::Result<Arc<dyn rara_kernel::provider::LlmProvider>> {
        let settings = self.settings.current();
        match settings.ai.provider.as_deref().unwrap_or("openrouter") {
            "ollama" => {
                let base_url = settings
                    .ai
                    .ollama_base_url
                    .as_deref()
                    .unwrap_or("http://localhost:11434");
                let config = async_openai::config::OpenAIConfig::new()
                    .with_api_base(format!("{}/v1", base_url))
                    .with_api_key("ollama");
                Ok(Arc::new(rara_kernel::provider::OpenAiProvider::with_config(
                    config,
                )))
            }
            "codex" => {
                // Token persistence and refresh rules live in integration layer.
                // Worker only orchestrates load -> maybe refresh -> construct provider.
                let mut tokens = rara_codex_oauth::load_tokens()
                    .map_err(|e| rara_kernel::KernelError::Provider { message: e.into() })?
                    .ok_or(rara_kernel::error::ProviderNotConfiguredSnafu.build())?;

                if rara_codex_oauth::should_refresh_token(tokens.expires_at_unix) {
                    let _guard = self.codex_refresh_lock.lock().await;
                    // Double-check in case another request refreshed already.
                    tokens = rara_codex_oauth::load_tokens()
                        .map_err(|e| rara_kernel::KernelError::Provider { message: e.into() })?
                        .ok_or(rara_kernel::error::ProviderNotConfiguredSnafu.build())?;
                    if rara_codex_oauth::should_refresh_token(tokens.expires_at_unix) {
                        let refreshed_tokens = rara_codex_oauth::refresh_tokens(&tokens)
                            .await
                            .map_err(|e| rara_kernel::KernelError::Provider { message: e.into() })?;
                        rara_codex_oauth::save_tokens(&refreshed_tokens).map_err(|e| {
                            rara_kernel::KernelError::Provider {
                                message: format!("failed to persist refreshed codex token: {e}")
                                    .into(),
                            }
                        })?;
                        tokens = refreshed_tokens;
                    }
                }

                let config =
                    async_openai::config::OpenAIConfig::new().with_api_key(tokens.access_token);
                Ok(Arc::new(rara_kernel::provider::OpenAiProvider::with_config(
                    config,
                )))
            }
            _ => {
                let api_key = settings
                    .ai
                    .openrouter_api_key
                    .clone()
                    .ok_or(rara_kernel::error::ProviderNotConfiguredSnafu.build())?;
                Ok(Arc::new(rara_kernel::provider::OpenAiProvider::new(api_key)))
            }
        }
    }
}
