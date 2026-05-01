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

use axum::http::{HeaderName, HeaderValue, Method, header};
use snafu::Whatever;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::info;
use url::Url;

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
    /// Kernel driver registry, threaded into the settings router so a
    /// `PATCH /api/v1/settings { llm.default_provider }` can swap the
    /// active driver at runtime (#2014).
    pub driver_registry:   rara_kernel::llm::DriverRegistryRef,
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
        driver_registry: rara_kernel::llm::DriverRegistryRef,
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
            driver_registry,
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
            crate::settings::routes(
                self.settings_svc.clone(),
                self.driver_registry.clone(),
                self.session_service.model_catalog().clone(),
            ),
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
/// Each entry is either an exact origin (`http://localhost:5173`) or a
/// loopback-only port wildcard (`http://localhost:*`,
/// `http://127.0.0.1:*`, `http://[::1]:*`). Wildcards are intentionally
/// restricted to loopback hosts: admin endpoints carry bearer tokens, so LAN
/// IPs and public domains must keep explicit ports.
///
/// Panics when the allow-list is empty, contains an origin that is not a valid
/// HTTP header value, or uses an unsafe wildcard. CORS misconfiguration is a
/// boot-time error: the frontend cannot reach the API without it, and silent
/// fallback would delay the failure to first browser request.
pub fn build_cors_layer(allowed_origins: &[String]) -> CorsLayer {
    assert!(
        !allowed_origins.is_empty(),
        "http.cors_allowed_origins must list at least one origin — see config.example.yaml",
    );

    let origins = allowed_origins
        .iter()
        .map(|origin| parse_cors_origin_config(origin))
        .collect::<Vec<_>>();

    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, _parts| {
            origins.iter().any(|allowed| allowed.matches(origin))
        }))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        // Surface the OTel-derived trace headers written by
        // `rara_server::http::inject_trace_headers` so the browser fetch API
        // can read them. Without `expose_headers`, the browser hides every
        // non-CORS-safelisted response header. See
        // `specs/issue-1975-trace-id-response-header.spec.md`.
        .expose_headers([
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("traceparent"),
        ])
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CorsOriginConfig {
    Exact(HeaderValue),
    LoopbackPortWildcard { scheme: String, host: String },
}

impl CorsOriginConfig {
    fn matches(&self, origin: &HeaderValue) -> bool {
        match self {
            Self::Exact(expected) => origin == expected,
            Self::LoopbackPortWildcard { scheme, host } => {
                origin_matches_loopback_port_wildcard(origin, scheme, host)
            }
        }
    }
}

fn parse_cors_origin_config(origin: &str) -> CorsOriginConfig {
    if let Some(pattern) = origin.strip_suffix(":*") {
        return parse_loopback_port_wildcard(origin, pattern);
    }

    CorsOriginConfig::Exact(
        HeaderValue::from_str(origin).unwrap_or_else(|err| {
            panic!("invalid http.cors_allowed_origins entry {origin:?}: {err}")
        }),
    )
}

fn parse_loopback_port_wildcard(origin: &str, pattern: &str) -> CorsOriginConfig {
    let url = Url::parse(pattern).unwrap_or_else(|err| {
        panic!("invalid http.cors_allowed_origins port wildcard {origin:?}: {err}")
    });
    assert_origin_url_has_no_path(origin, &url);
    assert!(
        matches!(url.scheme(), "http" | "https"),
        "invalid http.cors_allowed_origins port wildcard {origin:?}: scheme must be http or https",
    );
    assert!(
        url.port().is_none(),
        "invalid http.cors_allowed_origins port wildcard {origin:?}: wildcard must replace the \
         port",
    );

    let host = url.host_str().unwrap_or_else(|| {
        panic!("invalid http.cors_allowed_origins port wildcard {origin:?}: missing host")
    });
    assert!(
        is_loopback_host(host),
        "invalid http.cors_allowed_origins port wildcard {origin:?}: only localhost, 127.0.0.1, \
         and [::1] may use :*",
    );

    CorsOriginConfig::LoopbackPortWildcard {
        scheme: url.scheme().to_string(),
        host:   normalize_loopback_host(host),
    }
}

fn origin_matches_loopback_port_wildcard(
    origin: &HeaderValue,
    expected_scheme: &str,
    expected_host: &str,
) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Ok(url) = Url::parse(origin) else {
        return false;
    };
    if !origin_url_has_no_path(&url) || url.scheme() != expected_scheme || url.port().is_none() {
        return false;
    }

    url.host_str()
        .map(normalize_loopback_host)
        .is_some_and(|host| host == expected_host)
}

fn assert_origin_url_has_no_path(origin: &str, url: &Url) {
    assert!(
        origin_url_has_no_path(url),
        "invalid http.cors_allowed_origins entry {origin:?}: origin must not include path, query, \
         or fragment",
    );
}

fn origin_url_has_no_path(url: &Url) -> bool {
    url.path() == "/" && url.query().is_none() && url.fragment().is_none()
}

fn normalize_loopback_host(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

fn is_loopback_host(host: &str) -> bool {
    matches!(
        normalize_loopback_host(host).as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
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

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        routing::get,
    };
    use tower::ServiceExt;

    use super::*;

    /// Spec scenario `cors_exposes_trace_headers`: the `CorsLayer` produced
    /// by [`build_cors_layer`] must list exactly `x-request-id` and
    /// `traceparent` on its `expose_headers` allow-list, so the browser
    /// fetch API can read those headers across the dev-proxy boundary.
    ///
    /// The behavioral check (instead of poking private fields on
    /// `CorsLayer`) is what tower-http's API supports: send a request with a
    /// matching `Origin`, read `access-control-expose-headers` off the
    /// response.
    #[tokio::test]
    async fn cors_exposes_trace_headers() {
        let layer = build_cors_layer(&["http://localhost:5173".to_string()]);
        let app = Router::new()
            .route("/probe", get(|| async { StatusCode::OK }))
            .layer(layer);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/probe")
                    .header(header::ORIGIN, "http://localhost:5173")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let exposed = response
            .headers()
            .get("access-control-expose-headers")
            .expect("expose-headers must be present on a CORS-matched response")
            .to_str()
            .unwrap()
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .collect::<Vec<_>>();

        assert_eq!(
            exposed,
            vec!["x-request-id".to_string(), "traceparent".to_string()],
            "expose_headers must list exactly x-request-id and traceparent"
        );
    }

    #[tokio::test]
    async fn cors_loopback_port_wildcard_allows_dev_server_ports() {
        let layer = build_cors_layer(&[
            "http://localhost:*".to_string(),
            "http://127.0.0.1:*".to_string(),
            "http://[::1]:*".to_string(),
        ]);
        let app = Router::new()
            .route("/probe", get(|| async { StatusCode::OK }))
            .layer(layer);

        for origin in [
            "http://localhost:5173",
            "http://localhost:5175",
            "http://127.0.0.1:5175",
            "http://[::1]:5175",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/probe")
                        .header(header::ORIGIN, origin)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(
                response
                    .headers()
                    .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .and_then(|header| header.to_str().ok()),
                Some(origin),
                "loopback wildcard must mirror allowed origin {origin}",
            );
        }
    }

    #[tokio::test]
    async fn cors_loopback_port_wildcard_rejects_other_origins() {
        let layer = build_cors_layer(&["http://localhost:*".to_string()]);
        let app = Router::new()
            .route("/probe", get(|| async { StatusCode::OK }))
            .layer(layer);

        for origin in [
            "https://localhost:5175",
            "http://127.0.0.1:5175",
            "http://10.0.0.183:5175",
            "http://localhost",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/probe")
                        .header(header::ORIGIN, origin)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert!(
                response
                    .headers()
                    .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .is_none(),
                "origin {origin} must not match http://localhost:*",
            );
        }
    }

    #[test]
    #[should_panic(expected = "only localhost, 127.0.0.1, and [::1] may use :*")]
    fn cors_port_wildcard_rejects_non_loopback_hosts() {
        let _ = build_cors_layer(&["http://10.0.0.183:*".to_string()]);
    }
}
