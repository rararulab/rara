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
use common_worker::{IntervalOrNotifyHandle, NotifyHandle};
use opendal::Operator;
use openrouter_rs::client::OpenRouterClient;
use snafu::{ResultExt, Whatever};
use tokio::sync::OnceCell;
use tracing::{info, warn};
use yunara_store::db::DBStore;

/// Shared application state used by workers and HTTP routes.
#[derive(Clone)]
pub struct AppState {
    // -- AI --
    pub ai_service: rara_ai::service::AiService,

    // -- domain services --
    pub resume_service:      rara_domain_resume::ResumeAppService,
    pub application_service: rara_domain_application::service::ApplicationService,
    pub interview_service:   rara_domain_interview::service::InterviewService,
    pub scheduler_service:   rara_domain_scheduler::service::SchedulerService,
    pub analytics_service:   rara_domain_analytics::service::AnalyticsService,
    pub job_service:         rara_domain_job::service::JobService,
    pub chat_service:        rara_domain_chat::service::ChatService,
    pub typst_service:       rara_domain_typst::service::TypstService,

    // -- shared --
    pub settings_svc:  rara_domain_shared::settings::SettingsSvc,
    pub notify_client: rara_domain_shared::notify::client::NotifyClient,

    // -- LLM provider --
    pub llm_provider: rara_agents::model::OpenRouterLoaderRef,

    // -- infra --
    pub object_store: Operator,
    pub crawl_client: crawl4ai::Crawl4AiClient,

    // -- memory --
    pub memory_manager: Arc<rara_memory::MemoryManager>,

    // -- agent scheduler --
    pub agent_scheduler: Arc<crate::agent_scheduler::AgentScheduler>,

    // -- skills --
    pub skill_registry: rara_skills::registry::InMemoryRegistry,

    // -- MCP --
    pub mcp_manager: Arc<rara_mcp::manager::mgr::McpManager>,

    // -- worker coordination --
    pub analyze_notify:   Arc<RwLock<Option<NotifyHandle>>>,
    pub proactive_notify: Arc<RwLock<Option<IntervalOrNotifyHandle>>>,
}

impl AppState {
    /// Initialize all domain services and build the shared application state.
    pub async fn init(
        db_store: &DBStore,
        object_store: Operator,
        notify_client: rara_domain_shared::notify::client::NotifyClient,
        crawl4ai_url: &str,
    ) -> Result<Self, Whatever> {
        let pool = db_store.pool().clone();

        // -- runtime settings ------------------------------------------------

        let settings_svc = rara_domain_shared::settings::SettingsSvc::load(db_store.kv_store())
            .await
            .whatever_context("Failed to initialize runtime settings")?;
        info!("Runtime settings service loaded");

        // -- AI service ------------------------------------------------------

        let ai_service = rara_ai::service::AiService::new(settings_svc.clone());
        if settings_svc.current().ai.openrouter_api_key.is_some() {
            info!("AI service configured from runtime settings");
        } else {
            warn!("AI service not configured yet; set it via POST /api/v1/settings");
        }

        // -- domain services -------------------------------------------------

        let resume_service = rara_domain_resume::wire_resume_service(pool.clone());
        let application_service = rara_domain_application::wire(pool.clone());
        let interview_service = rara_domain_interview::wire_interview_service(pool.clone());
        let scheduler_service = rara_domain_scheduler::wire_scheduler_service(pool.clone());
        let analytics_service = rara_domain_analytics::wire_analytics_service(pool.clone());
        let job_service = rara_domain_job::wire_job_service(pool.clone(), ai_service.clone())
            .whatever_context("Failed to initialize job service")?;
        info!("Job service initialized");

        // -- typst service ---------------------------------------------------

        let typst_service =
            rara_domain_typst::wire_typst_service(pool.clone(), object_store.clone());
        info!("Typst service initialized");

        // -- infra clients ---------------------------------------------------

        let crawl_client = crawl4ai::Crawl4AiClient::new(crawl4ai_url);
        info!("Crawl4AI client configured");

        // -- chat service ----------------------------------------------------

        let session_repo = Arc::new(
            rara_sessions::pg_repository::PgSessionRepository::new(
                pool.clone(),
                rara_paths::sessions_dir(),
            )
            .await
            .whatever_context("Failed to initialize session repository")?,
        );
        let llm_provider: rara_agents::model::OpenRouterLoaderRef =
            Arc::new(SettingsOpenRouterLoader::new(settings_svc.clone()));
        let mut tool_registry = rara_agents::tool_registry::ToolRegistry::with_defaults();
        let memory_settings = settings_svc.current().agent.memory;
        let chroma_url = memory_settings
            .chroma_url
            .clone()
            .unwrap_or_else(|| "http://localhost:8000".to_owned());
        let chroma = rara_memory::ChromaClient::new(
            chroma_url,
            memory_settings.chroma_collection.clone(),
            memory_settings.chroma_api_key.clone(),
        )
        .expect("chroma URL should not be empty after defaulting");
        let memory_manager = Arc::new(
            rara_memory::MemoryManager::new(rara_paths::memory_dir().clone(), pool.clone(), chroma)
                .whatever_context("Failed to initialize memory manager")?,
        );
        info!("memory manager initialized");
        if let Err(err) = memory_manager.sync().await {
            warn!(error = %err, "Failed to sync memory index; continuing startup");
        }

        // Layer 1: Primitives
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::DbQueryTool::new(
            pool.clone(),
        )));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::DbMutateTool::new(
            pool.clone(),
        )));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::NotifyTool::new(
            notify_client.clone(),
            settings_svc.clone(),
        )));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::StorageReadTool::new(
            object_store.clone(),
        )));

        // Layer 2: Services
        tool_registry.register_service(Arc::new(crate::tools::services::JobPipelineTool::new(
            job_service.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::ListResumesTool::new(
            resume_service.clone(),
        )));
        tool_registry.register_service(Arc::new(
            crate::tools::services::GetResumeContentTool::new(resume_service.clone()),
        ));
        tool_registry.register_service(Arc::new(crate::tools::services::AnalyzeResumeTool::new(
            resume_service.clone(),
            job_service.clone(),
            ai_service.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::MemorySearchTool::new(
            Arc::clone(&memory_manager),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::MemoryGetTool::new(
            Arc::clone(&memory_manager),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::MemoryWriteTool::new(
            Arc::clone(&memory_manager),
        )));
        tool_registry.register_service(Arc::new(
            crate::tools::services::MemoryUpdateProfileTool::new(Arc::clone(&memory_manager)),
        ));
        tool_registry.register_service(Arc::new(
            crate::tools::services::ListTypstProjectsTool::new(typst_service.clone()),
        ));
        tool_registry.register_service(Arc::new(crate::tools::services::ListTypstFilesTool::new(
            typst_service.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::ReadTypstFileTool::new(
            typst_service.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::UpdateTypstFileTool::new(
            typst_service.clone(),
        )));
        tool_registry.register_service(Arc::new(
            crate::tools::services::CompileTypstProjectTool::new(typst_service.clone()),
        ));

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

        // -- codex agent dispatch ----------------------------------------
        let task_store = crate::tools::services::AgentTaskStore::new();
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        tool_registry.register_service(Arc::new(crate::tools::services::CodexRunTool::new(
            task_store.clone(),
            notify_client.clone(),
            settings_svc.clone(),
            project_root.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::ScreenshotTool::new(
            notify_client.clone(),
            settings_svc.clone(),
            project_root,
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::CodexStatusTool::new(
            task_store.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::CodexListTool::new(
            task_store.clone(),
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
        let mcp_manager = Arc::new(rara_mcp::manager::mgr::McpManager::new(
            Arc::new(mcp_registry),
            rara_mcp::oauth::OAuthCredentialsStoreMode::default(),
        ));
        let started = mcp_manager.start_enabled().await;
        if started.is_empty() {
            info!("No MCP servers to start");
        } else {
            info!(servers = ?started, "MCP servers started");
        }

        let tools = Arc::new(tool_registry);

        let chat_service = rara_domain_chat::service::ChatService::new(
            session_repo,
            llm_provider.clone(),
            tools,
            settings_svc.subscribe(),
            Some(Arc::clone(&memory_manager)),
            settings_svc.clone(),
            skill_registry.clone(),
        );
        info!("Chat service initialized");

        Ok(Self {
            ai_service,
            resume_service,
            application_service,
            interview_service,
            scheduler_service,
            analytics_service,
            job_service,
            chat_service,
            typst_service,
            settings_svc,
            notify_client,
            llm_provider,
            object_store,
            crawl_client,
            memory_manager,
            agent_scheduler,
            skill_registry,
            mcp_manager,
            analyze_notify: Arc::new(RwLock::new(None)),
            proactive_notify: Arc::new(RwLock::new(None)),
        })
    }

    /// Build all domain API routes and the OpenAPI spec.
    ///
    /// Each domain `OpenApiRouter` is split immediately into a plain
    /// `axum::Router` + `utoipa::openapi::OpenApi` and merged separately.
    /// This avoids the deeply-nested `Merge<Merge<Merge<…>>>` generic type
    /// that the chained `.merge()` calls on `OpenApiRouter` would produce,
    /// which previously caused a runtime stack overflow.
    pub fn routes(&self) -> (axum::Router, utoipa::openapi::OpenApi) {
        let mut api = Self::api_doc();

        let mut router = axum::Router::new();
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_resume::routes::routes(
                self.resume_service.clone(),
                self.object_store.clone(),
            ),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_application::routes::routes(self.application_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_interview::routes::routes(self.interview_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_scheduler::routes::routes(self.scheduler_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_analytics::routes::routes(self.analytics_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_job::routes::management_routes(
                self.job_service.clone(),
                self.object_store.clone(),
            ),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_job::routes::discovery_routes(self.job_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_shared::settings::router::routes(self.settings_svc.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_shared::notify::routes::routes(self.notify_client.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            rara_domain_chat::router::routes(self.chat_service.clone()),
        );
        router = router.merge(rara_domain_typst::router::plain_routes(
            self.typst_service.clone(),
        ));
        merge_openapi_router(&mut router, &mut api, crate::system_routes::routes());
        merge_openapi_router(
            &mut router,
            &mut api,
            crate::agent_scheduler_routes::routes(self.agent_scheduler.clone()),
        );

        // skill_routes returns a plain axum::Router (no OpenAPI metadata).
        router = router.merge(rara_domain_skill::router::skill_routes(
            self.skill_registry.clone(),
        ));

        // MCP admin routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(rara_mcp_admin::router(self.mcp_manager.clone()));

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
                (name = "typst", description = "Typst document management"),
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
// SettingsOpenRouterLoader
// ---------------------------------------------------------------------------

/// [`OpenRouterLoader`](rara_agents::model::OpenRouterLoader) implementation
/// that reads the API key from
/// [`SettingsSvc`](rara_domain_shared::settings::SettingsSvc) runtime settings
/// rather than from environment variables.
///
/// The client is lazily initialized on the first `acquire_client` call and
/// cached for subsequent calls via [`OnceCell`].
struct SettingsOpenRouterLoader {
    settings: rara_domain_shared::settings::SettingsSvc,
    client:   OnceCell<OpenRouterClient>,
}

impl SettingsOpenRouterLoader {
    fn new(settings: rara_domain_shared::settings::SettingsSvc) -> Self {
        Self {
            settings,
            client: OnceCell::new(),
        }
    }
}

#[async_trait]
impl rara_agents::model::OpenRouterLoader for SettingsOpenRouterLoader {
    async fn acquire_client(&self) -> rara_agents::err::Result<OpenRouterClient> {
        let client_ref = self
            .client
            .get_or_try_init(|| async {
                let api_key = self
                    .settings
                    .current()
                    .ai
                    .openrouter_api_key
                    .clone()
                    .ok_or(rara_agents::err::OpenRouterNotConfiguredSnafu.build())?;

                Self::build_client(api_key)
            })
            .await?;

        Ok(client_ref.clone())
    }
}
