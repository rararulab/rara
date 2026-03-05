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
//! object.  It wires scheduler, session (chat), and settings.

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
}

impl BackendState {
    /// Initialize all backend domain services.
    ///
    /// The caller is expected to have loaded `SettingsSvc` already (since the
    /// settings provider is also needed by `RaraState`).
    pub async fn init(
        pool: sqlx::SqlitePool,
        session_index: Arc<dyn rara_kernel::session::SessionIndex>,
        tape_store: Arc<rara_memory::tape::FileTapeStore>,
        settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
        settings_svc: crate::settings::SettingsSvc,
    ) -> Result<Self, Whatever> {
        // -- domain services -------------------------------------------------

        let scheduler_service = crate::scheduler::wire_scheduler_service(pool.clone());

        // -- session service (renamed from ChatService) ----------------------

        let session_service = crate::chat::service::SessionService::new(
            session_index,
            tape_store,
            settings_provider,
        );
        info!("Session service initialized");

        Ok(Self {
            scheduler_service,
            session_service,
            settings_svc,
        })
    }

    /// Build all domain API routes and the OpenAPI spec.
    ///
    /// Kernel-dependent routes (`agents`, `kernel`) use the `KernelHandle`
    /// for read-only access and mutation through the event queue.
    /// Skill and MCP routes need their respective service
    /// handles from [`RaraState`](rara_boot::state::RaraState).
    pub fn routes(
        &self,
        kernel_handle: &rara_kernel::handle::KernelHandle,
        skill_registry: &rara_skills::registry::InMemoryRegistry,
        mcp_manager: &rara_mcp::manager::mgr::McpManager,
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
            crate::chat::routes(self.session_service.clone()),
        );
        merge_openapi_router(&mut router, &mut api, crate::system_routes::routes());

        // skill_routes returns a plain axum::Router (no OpenAPI metadata).
        router = router.merge(crate::skills::skill_routes(skill_registry.clone()));

        // MCP admin routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(crate::mcp::mcp_router(mcp_manager.clone()));

        // Agent registry routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(crate::agents::agent_routes(kernel_handle.clone()));

        // Kernel observability routes (stats, sessions, approvals, audit).
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
