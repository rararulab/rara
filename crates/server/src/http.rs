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

use axum::{
    Router, extract::DefaultBodyLimit, http::StatusCode, response::IntoResponse, routing::get,
};
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use rsketch_base::readable_size::ReadableSize;
use rsketch_error::{ParseAddressSnafu, Result};
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use snafu::ResultExt;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use super::ServiceHandler;

/// Default maximum HTTP request body size (100 MB)
pub const DEFAULT_MAX_HTTP_BODY_SIZE: ReadableSize = ReadableSize::mb(100);

/// Configuration options for a REST server
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, SmartDefault, bon::Builder)]
pub struct RestServerConfig {
    /// The address to bind the REST server
    #[default = "127.0.0.1:3000"]
    pub bind_address:  String,
    /// Maximum HTTP request body size
    #[default(_code = "DEFAULT_MAX_HTTP_BODY_SIZE")]
    pub max_body_size: ReadableSize,
    /// Whether to enable CORS
    #[default = true]
    pub enable_cors:   bool,
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
/// use rsketch_server::http::{RestServerConfig, start_rest_server};
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
///
/// # Panics
///
/// May panic if TcpListener binding fails within the spawn context.
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

    // Build the router with middleware
    let mut router = Router::new()
        .route("/health", get(health_check))
        .layer(OtelInResponseLayer)
        .layer(OtelAxumLayer::default())
        .layer({
            #[allow(clippy::cast_possible_truncation)]
            DefaultBodyLimit::max(config.max_body_size.as_bytes() as usize)
        });

    // Add CORS if enabled
    if config.enable_cors {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);
        router = router.layer(cors);
    }

    // Register route handlers
    for handler in &route_handlers {
        info!("Registering REST route handler");
        router = handler(router);
    }

    // Spawn the server task
    let cancellation_token = CancellationToken::new();
    let (join_handle, started_rx) = {
        let (started_tx, started_rx) = oneshot::channel::<()>();
        let cancellation_token_clone = cancellation_token.clone();
        let join_handle = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(bind_addr).await.unwrap();
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    info!("REST server (on {}) starting", bind_addr);
                    let _ = started_tx.send(());
                    info!("REST server (on {}) started", bind_addr);
                    cancellation_token_clone.cancelled().await;
                    info!("REST server (on {}) received shutdown signal", bind_addr);
                })
                .await;

            info!(
                "REST server (on {}) task completed: {:?}",
                bind_addr, result
            );
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

/// Health check handler that returns detailed health information
async fn api_health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "service": "rsketch",
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

#[cfg(test)]
mod tests {
    use axum::{Json, routing::get};

    use super::*;

    fn init_test_logging() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("debug")
            .try_init();
    }

    /// Helper function to get an available port by binding to port 0
    async fn get_available_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // Release the port
        port
    }

    #[tokio::test]
    async fn test_rest_server_lifecycle() {
        init_test_logging();

        let port = get_available_port().await;
        let config = RestServerConfig {
            bind_address: format!("127.0.0.1:{port}"),
            ..RestServerConfig::default()
        };
        let handlers: Vec<fn(Router) -> Router> = vec![health_routes];

        let mut handler = start_rest_server(config, handlers).await.unwrap();

        // Wait for server to start
        handler.wait_for_start().await.unwrap();

        // Test that the server is running by making a request
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        let response = client
            .get(format!("http://127.0.0.1:{port}/api/v1/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        // Shutdown the server
        handler.shutdown();
        handler.wait_for_stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_rest_server_without_cors() {
        init_test_logging();

        let port = get_available_port().await;
        let config = RestServerConfig {
            bind_address: format!("127.0.0.1:{port}"),
            enable_cors: false,
            ..RestServerConfig::default()
        };
        let handlers = vec![health_routes];

        let mut handler = start_rest_server(config, handlers).await.unwrap();
        handler.wait_for_start().await.unwrap();

        // Test that the server is running
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        handler.shutdown();
        handler.wait_for_stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_multiple_route_handlers() {
        init_test_logging();

        async fn goodbye_handler() -> Json<&'static str> { Json("Goodbye, World!") }

        fn goodbye_routes(router: Router) -> Router {
            router.route("/api/v1/goodbye", get(goodbye_handler))
        }

        let port = get_available_port().await;
        let config = RestServerConfig {
            bind_address: format!("127.0.0.1:{port}"),
            ..RestServerConfig::default()
        };
        let handlers = vec![health_routes, goodbye_routes];

        let mut handler = start_rest_server(config, handlers).await.unwrap();
        handler.wait_for_start().await.unwrap();

        // Test both routes
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{port}/api/v1/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        let response = client
            .get(format!("http://127.0.0.1:{port}/api/v1/goodbye"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        handler.shutdown();
        handler.wait_for_stop().await.unwrap();
    }
}
