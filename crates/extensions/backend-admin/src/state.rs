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
//! object.  It wires session (chat), settings, and data feeds.

use std::sync::Arc;

use axum::http::{HeaderValue, Method, header};
use snafu::Whatever;
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::data_feeds::DataFeedRouterState;

/// Backend domain-service state.
///
/// Owns all domain services needed for HTTP admin routes.
#[derive(Clone)]
pub struct BackendState {
    pub session_service:   crate::chat::service::SessionService,
    pub settings_svc:      crate::settings::SettingsSvc,
    /// Data feed router state with both persistence and registry.
    pub feed_router_state: DataFeedRouterState,
}

impl BackendState {
    /// Initialize all backend domain services.
    ///
    /// The caller is expected to have loaded `SettingsSvc` already (since the
    /// settings provider is also needed by `RaraState`).
    pub async fn init(
        session_index: Arc<dyn rara_kernel::session::SessionIndex>,
        tape_service: rara_kernel::memory::TapeService,
        trace_service: rara_kernel::trace::TraceService,
        settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
        settings_svc: crate::settings::SettingsSvc,
        model_lister: rara_kernel::llm::LlmModelListerRef,
        feed_router_state: DataFeedRouterState,
    ) -> Result<Self, Whatever> {
        // -- domain services -------------------------------------------------

        // -- session service (renamed from ChatService) ----------------------

        let session_service = crate::chat::service::SessionService::new(
            session_index,
            tape_service,
            trace_service,
            settings_provider,
            model_lister,
        );
        info!("Session service initialized");

        Ok(Self {
            session_service,
            settings_svc,
            feed_router_state,
        })
    }

    /// Build all domain API routes and the OpenAPI spec.
    ///
    /// Kernel-dependent routes (`agents`, `kernel`) use the `KernelHandle`
    /// for read-only access and mutation through the event queue.
    /// Skill and MCP routes need their respective service
    /// handles from boot result (see `rara_app::boot`).
    pub fn routes(
        &self,
        kernel_handle: &rara_kernel::handle::KernelHandle,
        skill_registry: &rara_skills::registry::InMemoryRegistry,
        mcp_manager: &rara_mcp::manager::mgr::McpManager,
        auth_state: crate::auth::AuthState,
        _cors_allowed_origins: &[String],
    ) -> (axum::Router, utoipa::openapi::OpenApi) {
        let mut api = Self::api_doc();

        let mut router = axum::Router::new();
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
        merge_openapi_router(&mut router, &mut api, crate::auth::routes());

        // skill_routes returns a plain axum::Router (no OpenAPI metadata).
        router = router.merge(crate::skills::skill_routes(skill_registry.clone()));

        // MCP admin routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(crate::mcp::mcp_router(mcp_manager.clone()));

        // Agent registry routes (plain axum::Router, no OpenAPI metadata).
        router = router.merge(crate::agents::agent_routes(kernel_handle.clone()));

        // Kernel observability routes (stats, sessions, approvals, audit).
        router = router.merge(crate::kernel::router::kernel_routes(kernel_handle.clone()));

        // Scheduler admin routes — read-only curation of kernel jobs.
        router = router.merge(crate::scheduler::scheduler_routes(kernel_handle.clone()));

        // Data feed management routes (with registry sync).
        router = router.merge(crate::data_feeds::data_feed_routes(
            self.feed_router_state.clone(),
        ));

        // Subscription management routes — wrap the kernel's subscription
        // registry with a REST surface so operators can create the
        // subscriptions that drive ProactiveTurn / SilentAppend fan-out.
        router = router.merge(crate::subscriptions::subscription_routes(
            crate::subscriptions::SubscriptionRouterState {
                registry: kernel_handle.subscription_registry().clone(),
            },
        ));

        // Wrap every admin route with the bearer-auth middleware. Handlers
        // pull `Extension<Principal<Resolved>>` out of request extensions for
        // authorization checks; the middleware itself enforces the token.
        let router = router.layer(axum::middleware::from_fn_with_state(
            auth_state,
            crate::auth::auth_layer,
        ));

        // NOTE: the CORS layer is intentionally NOT applied here. CORS is a
        // cross-cutting concern for every public route (health, webhook,
        // kernel chat), not just admin. It is applied by `rara-app` to the
        // outermost composed router via [`build_cors_layer`] so the same
        // origin allow-list governs all browser-facing endpoints.

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
                (name = "settings", description = "Runtime settings"),
                (name = "system", description = "System utilities"),
                (name = "data-feeds", description = "Data feed management")
            )
        )]
        struct ApiDoc;
        ApiDoc::openapi()
    }
}

/// Build a [`CorsLayer`] allow-listing the given origins for every public
/// HTTP route.
///
/// Applied by `rara-app` to the outermost composed router so that health,
/// webhook, kernel chat, and admin endpoints share one consistent
/// origin allow-list. Browser preflight (`OPTIONS` without an
/// `Authorization` header) is answered here, before any auth middleware can
/// reject it with 401.
///
/// Panics when the allow-list is empty or contains an origin that is not a
/// valid HTTP header value. CORS misconfiguration is a boot-time error: the
/// frontend cannot reach the API without it, and silent fallback would delay
/// the failure to first browser request.
pub fn build_cors_layer(allowed_origins: &[String]) -> CorsLayer {
    assert!(
        !allowed_origins.is_empty(),
        "http.cors_allowed_origins must list at least one origin — see config.example.yaml",
    );

    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .map(|origin| {
            HeaderValue::from_str(origin).unwrap_or_else(|err| {
                panic!("invalid http.cors_allowed_origins entry {origin:?}: {err}")
            })
        })
        .collect();

    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
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
