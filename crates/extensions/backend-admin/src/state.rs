// Copyright 2025 Rararulab
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

//! Backend domain-service state — holds all HTTP admin services and routes.
//!
//! [`BackendState`] is the domain-service half of the old `AppState` god
//! object.  It wires scheduler, session (chat), settings, contacts, and
//! notification services.

use std::sync::Arc;

use snafu::Whatever;
use tracing::info;

/// Backend domain-service state.
///
/// Owns all domain services needed for HTTP admin routes.
#[derive(Clone)]
pub struct BackendState {
    pub scheduler_service: crate::scheduler::service::SchedulerService,
    pub session_service:   crate::chat::service::SessionService,
    pub settings_svc:      crate::settings::SettingsSvc,
    pub contact_repo:      rara_channels::telegram::contacts::repository::ContactRepository,
    pub notify_client:     rara_domain_shared::notify::client::NotifyClient,
}

impl BackendState {
    /// Initialize all backend domain services.
    ///
    /// The caller is expected to have loaded `SettingsSvc` already (since the
    /// settings provider is also needed by `RaraState`).
    pub async fn init(
        pool: sqlx::PgPool,
        notify_client: rara_domain_shared::notify::client::NotifyClient,
        session_repo: Arc<dyn rara_sessions::repository::SessionRepository>,
        settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
        settings_svc: crate::settings::SettingsSvc,
    ) -> Result<Self, Whatever> {
        // -- domain services -------------------------------------------------

        let scheduler_service = crate::scheduler::wire_scheduler_service(pool.clone());

        // -- session service (renamed from ChatService) ----------------------

        let session_service =
            crate::chat::service::SessionService::new(session_repo, settings_provider);
        info!("Session service initialized");

        // -- contacts ---------------------------------------------------------

        let contact_repo =
            rara_channels::telegram::contacts::repository::ContactRepository::new(pool);

        Ok(Self {
            scheduler_service,
            session_service,
            settings_svc,
            contact_repo,
            notify_client,
        })
    }

    /// Build all domain API routes and the OpenAPI spec.
    ///
    /// Kernel-dependent routes (`agents`, `kernel`) use the `KernelHandle`
    /// for read-only access and mutation through the event queue.
    /// Skill, MCP, and coding-task routes need their respective service
    /// handles from [`RaraState`](rara_boot::state::RaraState).
    pub fn routes(
        &self,
        kernel_handle: &rara_kernel::KernelHandle,
        skill_registry: &rara_skills::registry::InMemoryRegistry,
        mcp_manager: &rara_mcp::manager::mgr::McpManager,
        coding_task_service: &rara_coding_task::service::CodingTaskService,
    ) -> (axum::Router, utoipa::openapi::OpenApi) {
        let mut api = Self::api_doc();

        let mut router = axum::Router::new();
        merge_openapi_router(
            &mut router,
            &mut api,
            crate::scheduler::routes(self.scheduler_service.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            crate::settings::routes(self.settings_svc.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            crate::contacts::routes(self.contact_repo.clone()),
        );
        merge_openapi_router(
            &mut router,
            &mut api,
            crate::chat::routes(self.session_service.clone()),
        );
        merge_openapi_router(&mut router, &mut api, crate::system_routes::routes());

        // skill_routes returns a plain axum::Router (no OpenAPI metadata).
        router = router.merge(crate::skills::skill_routes(skill_registry.clone()));

        // MCP admin routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(crate::mcp::mcp_router(mcp_manager.clone()));

        // Coding task routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(crate::coding_task::routes(coding_task_service.clone()));

        // Agent registry routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(crate::agents::agent_routes(kernel_handle.clone()));

        // Kernel observability routes (stats, processes, approvals, audit).
        router = router.merge(crate::kernel::router::kernel_routes(kernel_handle.clone()));

        (router, api)
    }

    fn api_doc() -> utoipa::openapi::OpenApi {
        use utoipa::OpenApi;
        #[derive(OpenApi)]
        #[openapi(
            info(
                title = "Rara API",
                description = "AI Agent Platform API",
                version = "0.0.17"
            ),
            tags(
                (name = "chat", description = "Chat sessions and messaging"),
                (name = "scheduler", description = "Task scheduling"),
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
