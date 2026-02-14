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

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use common_worker::NotifyHandle;
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
    pub resume_service: rara_domain_resume::ResumeAppService,
    pub application_service: rara_domain_application::service::ApplicationService,
    pub interview_service: rara_domain_interview::service::InterviewService,
    pub scheduler_service: rara_domain_scheduler::service::SchedulerService,
    pub analytics_service: rara_domain_analytics::service::AnalyticsService,
    pub job_service: rara_domain_job::service::JobService,
    pub chat_service: rara_domain_chat::service::ChatService,
    pub typst_service: rara_domain_typst::service::TypstService,

    // -- shared --
    pub settings_svc: rara_domain_shared::settings::SettingsSvc,
    pub notify_client: rara_domain_shared::notify::client::NotifyClient,

    // -- LLM provider --
    pub llm_provider: rara_agents::model::OpenRouterLoaderRef,

    // -- infra --
    pub object_store: Operator,
    pub crawl_client: crawl4ai::Crawl4AiClient,

    // -- worker coordination --
    pub analyze_notify: Arc<RwLock<Option<NotifyHandle>>>,
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
        let mut tool_registry = rara_agents::tool_registry::ToolRegistry::default();
        let memory_backend = settings_svc.current().agent.memory.storage_backend;
        let memory_backend = if memory_backend.trim().is_empty() {
            "postgres".to_owned()
        } else {
            memory_backend.to_ascii_lowercase()
        };
        let memory_manager = Arc::new(match memory_backend.as_str() {
            "sqlite" => match rara_memory::MemoryManager::open(
                rara_paths::memory_dir().clone(),
                rara_paths::memory_index_db_file().clone(),
            ) {
                Ok(manager) => manager,
                Err(primary_err) => {
                    warn!(
                        error = %primary_err,
                        "sqlite memory manager init failed, falling back to postgres"
                    );
                    rara_memory::MemoryManager::open_postgres(
                        rara_paths::memory_dir().clone(),
                        pool.clone(),
                    )
                    .whatever_context(
                        "Failed to initialize postgres memory manager after sqlite fallback",
                    )?
                }
            },
            _ => match rara_memory::MemoryManager::open_postgres(
                rara_paths::memory_dir().clone(),
                pool.clone(),
            ) {
                Ok(manager) => manager,
                Err(primary_err) => {
                    warn!(
                        error = %primary_err,
                        "postgres memory manager init failed, falling back to sqlite"
                    );
                    rara_memory::MemoryManager::open(
                        rara_paths::memory_dir().clone(),
                        rara_paths::memory_index_db_file().clone(),
                    )
                    .whatever_context(
                        "Failed to initialize sqlite memory manager after postgres fallback",
                    )?
                }
            },
        });
        memory_manager.apply_runtime_settings(&settings_svc.current().agent.memory);
        info!(
            storage_backend = memory_manager.storage_backend(),
            vector_backend = memory_manager.vector_backend(),
            "memory manager initialized"
        );
        let _ = memory_manager
            .sync()
            .await
            .whatever_context("Failed to sync memory index")?;

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
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::HttpFetchTool::new()));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::StorageReadTool::new(
            object_store.clone(),
        )));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::BashTool::new()));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::ReadFileTool::new()));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::WriteFileTool::new()));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::EditFileTool::new()));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::FindFilesTool::new()));
        tool_registry.register_primitive(Arc::new(crate::tools::primitives::GrepTool::new()));
        tool_registry
            .register_primitive(Arc::new(crate::tools::primitives::ListDirectoryTool::new()));

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
            settings_svc.clone(),
        )));
        tool_registry.register_service(Arc::new(crate::tools::services::MemoryGetTool::new(
            Arc::clone(&memory_manager),
            settings_svc.clone(),
        )));
        tool_registry.register_service(Arc::new(
            crate::tools::services::ListTypstProjectsTool::new(typst_service.clone()),
        ));
        tool_registry.register_service(Arc::new(
            crate::tools::services::ListTypstFilesTool::new(typst_service.clone()),
        ));
        tool_registry.register_service(Arc::new(
            crate::tools::services::ReadTypstFileTool::new(typst_service.clone()),
        ));
        tool_registry.register_service(Arc::new(
            crate::tools::services::UpdateTypstFileTool::new(typst_service.clone()),
        ));
        tool_registry.register_service(Arc::new(
            crate::tools::services::CompileTypstProjectTool::new(typst_service.clone()),
        ));
        let tools = Arc::new(tool_registry);
        let chat_service = rara_domain_chat::service::ChatService::new(
            session_repo,
            llm_provider.clone(),
            tools,
            settings_svc.subscribe(),
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
            analyze_notify: Arc::new(RwLock::new(None)),
        })
    }

    /// Build an [`axum::Router`] with all domain API routes.
    pub fn routes(&self) -> axum::Router {
        use rara_server::dedup_layer::{DedupLayer, DedupLayerConfig};

        axum::Router::new()
            .merge(rara_domain_resume::routes::routes(
                self.resume_service.clone(),
                self.object_store.clone(),
            ))
            .merge(rara_domain_application::routes::routes(
                self.application_service.clone(),
            ))
            .merge(rara_domain_interview::routes::routes(
                self.interview_service.clone(),
            ))
            .merge(rara_domain_scheduler::routes::routes(
                self.scheduler_service.clone(),
            ))
            .merge(rara_domain_analytics::routes::routes(
                self.analytics_service.clone(),
            ))
            .merge(rara_domain_job::routes::management_routes(
                self.job_service.clone(),
                self.object_store.clone(),
            ))
            .merge(
                rara_domain_job::routes::discovery_routes(self.job_service.clone())
                    .layer(DedupLayer::new(DedupLayerConfig::default())),
            )
            .merge(rara_domain_shared::settings::router::routes(
                self.settings_svc.clone(),
            ))
            .merge(rara_domain_shared::notify::routes::routes(
                self.notify_client.clone(),
            ))
            .merge(rara_domain_chat::router::routes(self.chat_service.clone()))
            .merge(rara_domain_typst::router::routes(
                self.typst_service.clone(),
            ))
    }
}

// ---------------------------------------------------------------------------
// SettingsOpenRouterLoader
// ---------------------------------------------------------------------------

/// [`OpenRouterLoader`](rara_agents::model::OpenRouterLoader) implementation
/// that reads the API key from [`SettingsSvc`](rara_domain_shared::settings::SettingsSvc)
/// runtime settings rather than from environment variables.
///
/// The client is lazily initialized on the first `acquire_client` call and
/// cached for subsequent calls via [`OnceCell`].
struct SettingsOpenRouterLoader {
    settings: rara_domain_shared::settings::SettingsSvc,
    client: OnceCell<OpenRouterClient>,
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
