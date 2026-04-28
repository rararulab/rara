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

use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::{DefaultBodyLimit, MatchedPath, Request},
    http::{HeaderName, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use base::readable_size::ReadableSize;
use opentelemetry::{KeyValue, global, metrics::Histogram, trace::TraceContextExt};
use rara_error::{ConnectionSnafu, ParseAddressSnafu, Result};
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use snafu::ResultExt;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};
use tracing::{Span, info};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Lowercase header name carrying the OTel trace_id as 32 hex chars.
///
/// Surfaced to the browser so a user reporting a bug can paste this ID
/// into Langfuse/Loki and resolve the trace in seconds. See
/// `specs/issue-1975-trace-id-response-header.spec.md`.
pub const TRACE_HEADER_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// W3C Trace Context header carrying the same trace_id alongside the
/// active span_id, formatted as `00-<32hex>-<16hex>-01`.
pub const TRACE_HEADER_TRACEPARENT: HeaderName = HeaderName::from_static("traceparent");

/// Inject `x-request-id` and `traceparent` response headers carrying the
/// OTel trace_id of the current `http_request` span.
///
/// Layered AFTER `TraceLayer::new_for_http()` so the span is already active
/// when this middleware reads the OTel context. If the span has no valid
/// trace_id (e.g. request rejected before the trace layer ran) the headers
/// are simply omitted — partial information beats no information, and an
/// empty header value is worse than the header being absent.
pub async fn inject_trace_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;

    let context = Span::current().context();
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if !span_context.is_valid() {
        return response;
    }

    let trace_id_hex = format!(
        "{:032x}",
        u128::from_be_bytes(span_context.trace_id().to_bytes())
    );
    if let Ok(value) = HeaderValue::from_str(&trace_id_hex) {
        response
            .headers_mut()
            .insert(TRACE_HEADER_REQUEST_ID, value);
    }

    let span_id_bytes = span_context.span_id().to_bytes();
    // span_id of all zeros is invalid even when trace_id is set; skip
    // traceparent rather than emit a malformed value.
    if span_id_bytes != [0u8; 8] {
        let span_id_hex = format!("{:016x}", u64::from_be_bytes(span_id_bytes));
        let traceparent = format!("00-{trace_id_hex}-{span_id_hex}-01");
        if let Ok(value) = HeaderValue::from_str(&traceparent) {
            response
                .headers_mut()
                .insert(TRACE_HEADER_TRACEPARENT, value);
        }
    }

    response
}

use super::ServiceHandler;

/// Default maximum HTTP request body size (100 MB)
pub const DEFAULT_MAX_HTTP_BODY_SIZE: ReadableSize = ReadableSize::mb(100);

/// Default request timeout in seconds.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 60;

/// HTTP server request duration histogram (OTel push-based).
static HTTP_SERVER_REQUEST_DURATION_SECONDS: LazyLock<Histogram<f64>> = LazyLock::new(|| {
    global::meter("rara-server")
        .f64_histogram("http.server.request.duration")
        .with_description("HTTP server request duration in seconds")
        .with_unit("s")
        .build()
});

async fn observe_http_metrics(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let started_at = Instant::now();

    let response = next.run(request).await;
    let status = response.status();

    HTTP_SERVER_REQUEST_DURATION_SECONDS.record(
        started_at.elapsed().as_secs_f64(),
        &[
            KeyValue::new("http.request.method", method.as_str().to_string()),
            KeyValue::new("http.route", route),
            KeyValue::new("http.response.status_code", status.as_str().to_string()),
        ],
    );

    response
}

/// Configuration options for a REST server
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, SmartDefault, bon::Builder)]
#[serde(default)]
pub struct RestServerConfig {
    /// The address to bind the REST server
    #[default = "127.0.0.1:25555"]
    pub bind_address:         String,
    /// Maximum HTTP request body size
    #[default(_code = "DEFAULT_MAX_HTTP_BODY_SIZE")]
    pub max_body_size:        ReadableSize,
    /// Request timeout in seconds
    #[default(DEFAULT_REQUEST_TIMEOUT_SECS)]
    pub request_timeout:      u64,
    /// Port for the web frontend dev server (optional).
    ///
    /// When set, the app spawns `bun run dev` in `web/`. Vite uses its
    /// own port (default 5173); this field gates whether to start it.
    pub web_port:             Option<u16>,
    /// Explicit CORS allow-list for the admin HTTP surface.
    ///
    /// Each entry is an origin string (scheme + host + port) permitted to
    /// call `/api/v1/*` from a browser, e.g. `http://localhost:5173`. An
    /// empty list is a hard boot-time error — no hardcoded default, no
    /// silent fallback (see `docs/guides/anti-patterns.md`).
    #[serde(default)]
    pub cors_allowed_origins: Vec<String>,
}

/// Starts the REST server and returns a handle for managing its lifecycle.
///
/// This method:
/// 1. Sets up the Axum router with middleware (CORS, body size limits)
/// 2. Registers all provided route handlers
/// 3. Parses and binds to the configured address
/// 4. Spawns the server in a background task
/// 5. Returns a handle for lifecycle management
///
/// The server will automatically register all provided route handlers and
/// supports graceful shutdown through the returned handle.
///
/// # Arguments
/// * `config` - Configuration for the REST server
/// * `route_handlers` - Vector of functions that take a Router and return a
///   modified Router
///
/// # Errors
/// Returns an error if the bind address cannot be parsed.
///
/// # Example
///
/// ```rust,ignore
/// use axum::{Router, routing::get};
/// use job_server::http::{RestServerConfig, start_rest_server};
///
/// fn my_routes(router: Router) -> Router {
///     router.route("/api/v1/hello", get(|| async { "Hello, World!" }))
/// }
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = RestServerConfig::default();
///     let handlers = vec![my_routes];
///     let handle = start_rest_server(config, handlers).await?;
///     Ok(())
/// }
/// ```
///
/// # Errors
///
/// Returns an error if server binding fails or graceful shutdown encounters
/// issues.
#[allow(clippy::unused_async)]
pub async fn start_rest_server<F>(
    config: RestServerConfig,
    route_handlers: Vec<F>,
) -> Result<ServiceHandler>
where
    F: Fn(Router) -> Router + Send + Sync + 'static,
{
    // Parse bind address
    let bind_addr = config
        .bind_address
        .parse::<std::net::SocketAddr>()
        .context(ParseAddressSnafu {
            addr: config.bind_address.clone(),
        })?;

    // Register route handlers FIRST, then apply layers.
    // In axum, .layer() only applies to routes that already exist on the router.
    let mut api_router = Router::new();
    for handler in &route_handlers {
        info!("Registering REST route handler");
        api_router = handler(api_router);
    }

    // Apply request-scoped middleware to the API router.
    api_router = api_router
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(config.request_timeout),
        ))
        .layer({
            #[allow(clippy::cast_possible_truncation)]
            DefaultBodyLimit::max(config.max_body_size.as_bytes() as usize)
        });

    // CORS is now owned by each domain router (see `rara_backend_admin::state`)
    // so the allow-list can be configured per surface and preflight (OPTIONS)
    // responses bypass downstream auth middleware.

    // Build the final router: merge API routes, add /health and fallback,
    // then apply TraceLayer as the outermost layer so it observes every
    // request — including merged domain routes and timeout responses.
    let router = Router::new()
        .route("/health", get(health_check))
        .merge(api_router)
        .fallback(route_not_found)
        .layer(middleware::from_fn(observe_http_metrics))
        .layer(middleware::from_fn(inject_trace_headers))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<_>| {
                    let path = request.uri().path();
                    if path == "/health" || path.ends_with("/health") {
                        tracing::debug_span!(
                            "http_request",
                            method = %request.method(),
                            path = %path,
                        )
                    } else {
                        tracing::info_span!(
                            "http_request",
                            method = %request.method(),
                            path = %path,
                        )
                    }
                })
                .on_response(
                    |response: &axum::http::Response<_>, latency: Duration, span: &Span| {
                        let is_debug = span
                            .metadata()
                            .map_or(false, |m| *m.level() == tracing::Level::DEBUG);
                        if is_debug {
                            tracing::debug!(
                                status = response.status().as_u16(),
                                latency_ms = latency.as_millis(),
                                "response"
                            );
                        } else {
                            tracing::info!(
                                status = response.status().as_u16(),
                                latency_ms = latency.as_millis(),
                                "response"
                            );
                        }
                    },
                )
                .on_failure(
                    |error: tower_http::classify::ServerErrorsFailureClass,
                     latency: Duration,
                     _span: &Span| {
                        tracing::error!(
                            error = %error,
                            latency_ms = latency.as_millis(),
                            "request failed"
                        );
                    },
                ),
        );

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .context(ConnectionSnafu {
            addr: config.bind_address.clone(),
        })?;

    // Spawn the server task
    let cancellation_token = CancellationToken::new();
    let (join_handle, started_rx) = {
        let (started_tx, started_rx) = oneshot::channel::<()>();
        let cancellation_token_clone = cancellation_token.clone();
        let join_handle = tokio::spawn(async move {
            info!("REST server (on {})", bind_addr);
            let _ = started_tx.send(());
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    cancellation_token_clone.cancelled().await;
                    info!("REST server (on {}) received shutdown signal", bind_addr);
                })
                .await;
            info!("REST server finished: {:?}", result);
        });
        (join_handle, started_rx)
    };

    Ok(ServiceHandler {
        join_handle,
        cancellation_token,
        started_rx: Some(started_rx),
        reporter_handles: Vec::new(), // No readiness reporting for simple route handlers
    })
}

/// Health check endpoint for the REST server
async fn health_check() -> impl IntoResponse { (StatusCode::OK, "OK") }

/// Default fallback for unmatched HTTP routes.
async fn route_not_found(method: Method, uri: Uri) -> impl IntoResponse {
    let body = axum::Json(serde_json::json!({
        "error": "route_not_found",
        "message": "Route not found",
        "method": method.as_str(),
        "path": uri.path(),
    }));
    (StatusCode::NOT_FOUND, body)
}

/// Health check handler that returns detailed health information
async fn api_health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "healthy",
        "timestamp": jiff::Timestamp::now().to_string(),
        "service": "job",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Add health routes to the router
///
/// This function adds health check endpoints for API monitoring and readiness
/// checks. It provides both simple health check and detailed health information
/// endpoints.
pub fn health_routes(router: Router) -> Router {
    router
        .route("/api/v1/health", get(api_health_handler))
        .route("/api/health", get(api_health_handler))
}
