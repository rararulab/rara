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

use std::sync::Arc;

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
    pub provider_registry: Arc<rara_kernel::provider::ProviderRegistry>,

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

        let provider_registry =
            rara_boot::providers::build_provider_registry(&*settings_provider).await;

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
        let composio_auth_provider =
            rara_boot::providers::composio_auth_provider(settings_provider.clone());
        let contact_repo =
            rara_channels::telegram::contacts::repository::ContactRepository::new(pool.clone());

        // -- primitive tools (Layer 1) ----------------------------------------

        let mut tool_registry = rara_kernel::tool::ToolRegistry::new();
        for tool in rara_boot::tools::default_primitives(rara_boot::tools::PrimitiveDeps {
            settings:               settings_provider.clone(),
            object_store:           object_store.clone(),
            composio_auth_provider,
        }) {
            tool_registry.register_primitive(tool);
        }

        // -- memory -----------------------------------------------------------

        let memory_manager = rara_boot::memory::init_memory_manager(
            mem0_base_url,
            memos_base_url,
            memos_token,
            hindsight_base_url,
            hindsight_bank_id,
        );
        let recall_engine = rara_boot::memory::init_recall_engine();

        // -- coding task service ----------------------------------------------

        let default_repo_url = std::env::var("RARA_DEFAULT_REPO_URL")
            .unwrap_or_else(|_| "https://github.com/crrow/job".to_owned());
        let coding_task_service = rara_boot::coding_task::init_coding_task_service(
            pool.clone(),
            notify_client.clone(),
            settings_provider.clone(),
            default_repo_url,
        );

        // -- skills registry --------------------------------------------------

        let skill_registry = rara_boot::skills::init_skill_registry(pool.clone());

        // -- MCP manager ------------------------------------------------------

        let mcp_manager = rara_boot::mcp::init_mcp_manager()
            .await
            .whatever_context("Failed to initialize MCP manager")?;

        // -- service tools (Layer 2) ------------------------------------------

        rara_boot::tools::register_service_tools(
            &mut tool_registry,
            rara_boot::tools::ServiceToolDeps {
                memory_manager:     memory_manager.clone(),
                recall_engine,
                coding_task_service: coding_task_service.clone(),
                skill_registry:     skill_registry.clone(),
                mcp_manager:        mcp_manager.clone(),
                notify_client:      notify_client.clone(),
                settings:           settings_provider.clone(),
            },
        );

        let tools = Arc::new(tool_registry);

        let session_repo: Arc<dyn rara_sessions::repository::SessionRepository> = session_repo;
        let chat_service = rara_backend_admin::chat::service::ChatService::new(
            session_repo.clone(),
            settings_provider.clone(),
        );
        info!("Chat service initialized");

        // -- kernel -----------------------------------------------------------

        let agent_registry = rara_boot::manifests::load_default_registry();

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
