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

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use opendal::Operator;
use snafu::{ResultExt, Whatever};
use tracing::info;
use yunara_store::db::DBStore;

/// Shared application state used by workers and HTTP routes.
#[derive(Clone)]
pub struct AppState {
    // -- domain services --
    pub resume_service:      rara_backend_admin::resume::ResumeAppService,
    pub application_service: rara_backend_admin::application::service::ApplicationService,
    pub interview_service:   rara_backend_admin::interview::service::InterviewService,
    pub scheduler_service:   rara_backend_admin::scheduler::service::SchedulerService,
    pub analytics_service:   rara_backend_admin::analytics::service::AnalyticsService,
    pub job_service:         rara_backend_admin::job::service::JobService,
    pub chat_service:        rara_backend_admin::chat::service::ChatService,
    pub session_repo:        Arc<dyn rara_sessions::repository::SessionRepository>,
    // -- shared --
    pub settings_svc:        rara_backend_admin::settings::SettingsSvc,
    pub notify_client:       rara_domain_shared::notify::client::NotifyClient,
    pub contact_repo:        rara_channels::telegram::contacts::repository::ContactRepository,

    // -- LLM provider --
    pub provider_registry: std::sync::Arc<rara_kernel::provider::ProviderRegistry>,

    // -- infra --
    pub object_store: Operator,

    // -- memory --
    pub memory_manager: Arc<rara_memory::MemoryManager>,

    // -- skills --
    pub skill_registry: rara_skills::registry::InMemoryRegistry,

    // -- MCP --
    pub mcp_manager: rara_mcp::manager::mgr::McpManager,

    // -- coding tasks --
    pub coding_task_service: rara_coding_task::service::CodingTaskService,

    // -- kernel --
    pub kernel: Arc<rara_kernel::Kernel>,

    // -- user store --
    pub user_store: Arc<dyn rara_kernel::process::user::UserStore>,

    // -- tool registry --
    pub tool_registry: Arc<rara_kernel::tool::ToolRegistry>,
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

        let settings_svc =
            rara_backend_admin::settings::SettingsSvc::load(db_store.kv_store(), pool.clone())
                .await
                .whatever_context("Failed to initialize runtime settings")?;
        let settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider> =
            Arc::new(settings_svc.clone());
        info!("Runtime settings service loaded");

        // -- LLM provider registry -------------------------------------------

        let provider_registry = build_provider_registry(&*settings_provider).await;

        // -- domain services -------------------------------------------------

        let resume_service = rara_backend_admin::resume::wire_resume_service(pool.clone());
        let application_service = rara_backend_admin::application::wire(pool.clone());
        let interview_service = rara_backend_admin::interview::wire_interview_service(pool.clone());
        let scheduler_service = rara_backend_admin::scheduler::wire_scheduler_service(pool.clone());
        let analytics_service = rara_backend_admin::analytics::wire_analytics_service(pool.clone());
        let job_service =
            rara_backend_admin::job::wire_job_service(pool.clone())
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
            Arc::new(SettingsComposioAuthProvider::new(settings_provider.clone()));
        let contact_repo =
            rara_channels::telegram::contacts::repository::ContactRepository::new(pool.clone());
        let mut tool_registry = rara_kernel::tool::ToolRegistry::new();
        for tool in rara_boot::tools::default_primitives(rara_boot::tools::PrimitiveDeps {
            settings:               settings_provider.clone(),
            object_store:           object_store.clone(),
            composio_auth_provider: composio_auth_provider.clone(),
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
        // -- codex agent dispatch (PG-backed via rara-coding-task) --------
        let workspace_manager =
            rara_workspace::WorkspaceManager::new(rara_paths::workspaces_dir().clone());
        let default_repo_url = std::env::var("RARA_DEFAULT_REPO_URL")
            .unwrap_or_else(|_| "https://github.com/crrow/job".to_owned());
        let coding_task_service = rara_coding_task::service::wire(
            pool.clone(),
            workspace_manager,
            notify_client.clone(),
            settings_provider.clone(),
            default_repo_url,
        );
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        tool_registry.register_service(Arc::new(crate::tools::services::CodexRunTool::new(
            coding_task_service.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::ScreenshotTool::new(
            notify_client.clone(),
            settings_provider.clone(),
            project_root,
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::CodexStatusTool::new(
            coding_task_service.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::CodexListTool::new(
            coding_task_service.clone(),
        )));

        // -- skills registry (PG cache + incremental FS sync) --------------------
        let skill_registry = rara_boot::skills::init_skill_registry(pool.clone());

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

        let mcp_manager = rara_boot::mcp::init_mcp_manager()
            .await
            .whatever_context("Failed to initialize MCP manager")?;

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

        let session_repo: Arc<dyn rara_sessions::repository::SessionRepository> = session_repo;
        let chat_service = rara_backend_admin::chat::service::ChatService::new(
            session_repo.clone(),
            settings_provider.clone(),
        );
        info!("Chat service initialized");

        // -- kernel ---------------------------------------------------------------

        let agent_registry = rara_boot::manifests::load_default_registry();

        // User store — PgUserStore backed by the shared pool
        let user_store: Arc<dyn rara_kernel::process::user::UserStore> =
            Arc::new(rara_boot::user_store::PgUserStore::new(pool.clone()));
        rara_boot::user_store::ensure_default_users(&pool)
            .await
            .whatever_context("Failed to ensure default users")?;

        let kernel = Arc::new(rara_boot::kernel::boot(rara_boot::kernel::BootConfig {
            kernel_config: rara_kernel::KernelConfig {
                max_concurrency:        16,
                default_child_limit:    4,
                default_max_iterations: 25,
                memory_quota_per_agent: 1000,
                ..Default::default()
            },
            provider_registry: provider_registry.clone(),
            tool_registry:    tools.clone(),
            agent_registry,
            user_store:       user_store.clone(),
            session_repo:     session_repo.clone(),
            settings:         settings_provider.clone(),
            ..Default::default()
        }));
        info!("Kernel initialized");

        Ok(Self {
            resume_service,
            application_service,
            interview_service,
            scheduler_service,
            analytics_service,
            job_service,
            chat_service,
            session_repo,
            settings_svc,
            notify_client,
            contact_repo,
            provider_registry,
            object_store,
            memory_manager,
            skill_registry,
            mcp_manager,
            coding_task_service,
            kernel,
            user_store,
            tool_registry: tools,
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

        // Agent registry routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(rara_backend_admin::agents::agent_routes(
            self.kernel.clone(),
        ));

        // Kernel observability routes (stats, processes, approvals, audit).
        router = router.merge(rara_backend_admin::kernel::router::kernel_routes(
            self.kernel.clone(),
        ));

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
// ProviderRegistry builder from settings
// ---------------------------------------------------------------------------

/// Build a [`ProviderRegistry`] from runtime settings.
///
/// Reads `llm.provider` (default: `"openrouter"`) and `llm.models.default`
/// to determine the default provider and model. Then registers all
/// available providers based on configured API keys / base URLs.
async fn build_provider_registry(
    settings: &dyn rara_domain_shared::settings::SettingsProvider,
) -> Arc<rara_kernel::provider::ProviderRegistry> {
    use rara_domain_shared::settings::keys;
    use rara_kernel::provider::{OpenAiProvider, ProviderRegistryBuilder};

    let default_provider = settings
        .get(keys::LLM_PROVIDER)
        .await
        .unwrap_or_else(|| "openrouter".to_owned());
    let default_model = settings
        .get(keys::LLM_MODELS_DEFAULT)
        .await
        .unwrap_or_else(|| "openai/gpt-4o-mini".to_owned());

    let mut builder = ProviderRegistryBuilder::new(&default_provider, &default_model);

    // -- openrouter ---------------------------------------------------------
    if let Some(api_key) = settings.get(keys::LLM_OPENROUTER_API_KEY).await {
        builder = builder.provider(
            "openrouter",
            Arc::new(OpenAiProvider::new(api_key)),
        );
    }

    // -- ollama -------------------------------------------------------------
    {
        let base_url = settings
            .get(keys::LLM_OLLAMA_BASE_URL)
            .await
            .unwrap_or_else(|| "http://localhost:11434".to_owned());
        let config = async_openai::config::OpenAIConfig::new()
            .with_api_base(format!("{}/v1", base_url))
            .with_api_key("ollama");
        builder = builder.provider(
            "ollama",
            Arc::new(OpenAiProvider::with_config(config)),
        );
    }

    // -- codex (OpenAI via OAuth) -------------------------------------------
    if let Ok(Some(tokens)) = rara_codex_oauth::load_tokens() {
        let config =
            async_openai::config::OpenAIConfig::new().with_api_key(&tokens.access_token);
        builder = builder.provider(
            "codex",
            Arc::new(OpenAiProvider::with_config(config)),
        );
    }

    info!("provider registry: default_provider={default_provider}, default_model={default_model}");
    Arc::new(builder.build())
}

/// Composio auth provider that reads credentials from runtime settings.
#[derive(Clone)]
struct SettingsComposioAuthProvider {
    settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
}

impl SettingsComposioAuthProvider {
    fn new(settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>) -> Self {
        Self { settings }
    }
}

#[async_trait]
impl rara_composio::ComposioAuthProvider for SettingsComposioAuthProvider {
    async fn acquire_auth(&self) -> anyhow::Result<rara_composio::ComposioAuth> {
        use rara_domain_shared::settings::keys;
        let api_key = self
            .settings
            .get(keys::COMPOSIO_API_KEY)
            .await
            .ok_or_else(|| anyhow::anyhow!("composio.api_key is not configured in settings"))?;
        let entity_id = self.settings.get(keys::COMPOSIO_ENTITY_ID).await;
        Ok(rara_composio::ComposioAuth::new(
            api_key,
            entity_id.as_deref(),
        ))
    }
}
