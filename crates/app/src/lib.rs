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

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use job_domain_shared::notify::client::NotifyClient;
use job_server::{
    grpc::{GrpcServerConfig, hello::HelloService, start_grpc_server},
    http::{RestServerConfig, health_routes, start_rest_server},
};
use opendal::Operator;
use serde::Deserialize;
use snafu::{ResultExt, Whatever};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use yunara_store::{config::DatabaseConfig, db::DBStore};

// ---------------------------------------------------------------------------
// Static config types (immutable after startup)
// ---------------------------------------------------------------------------

/// Crawl4AI service configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Crawl4AiConfig {
    pub url: String,
}

impl Default for Crawl4AiConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:11235".to_owned(),
        }
    }
}

/// Static application configuration — immutable after startup.
///
/// Deserializable from `config.toml` + environment variables via the `config`
/// crate. For runtime-changeable values (OpenRouter key, Telegram token, …) see
/// [`job_domain_shared::settings::SettingsSvc`].
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Database connection pool.
    pub database:               DatabaseConfig,
    /// HTTP server bind / limits.
    pub http:                   RestServerConfig,
    /// gRPC server bind / limits.
    pub grpc:                   GrpcServerConfig,
    /// S3-compatible object store.
    #[serde(alias = "minio")]
    pub object_store:           job_object_store::ObjectStoreConfig,
    /// Crawl4AI service.
    pub crawl4ai:               Crawl4AiConfig,
    /// Saved-job GC interval in hours.
    pub gc_interval_hours:      u64,
    /// Main service HTTP base URL (for telegram bot → main service calls).
    pub main_service_http_base: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut object_store = job_object_store::ObjectStoreConfig::default();
        object_store.bucket = "job-markdown".to_owned();
        Self {
            database: DatabaseConfig::default(),
            http: RestServerConfig::default(),
            grpc: GrpcServerConfig::default(),
            object_store,
            crawl4ai: Crawl4AiConfig::default(),
            gc_interval_hours: 24,
            main_service_http_base: "http://127.0.0.1:3000".to_owned(),
        }
    }
}

impl AppConfig {
    /// Load config from config file + environment variables.
    ///
    /// Source priority (highest first):
    /// 1. `JOB__`-prefixed environment variables
    /// 2. `config.toml` file in the working directory
    /// 3. Code defaults
    pub fn new() -> Result<Self, config::ConfigError> {
        let builder = config::Config::builder()
            .add_source(config::File::with_name("config").required(false))
            .add_source(
                config::Environment::with_prefix("JOB")
                    .separator("__")
                    .try_parsing(true),
            );

        let cfg = builder.build()?;
        cfg.try_deserialize()
    }

    /// Initialize infrastructure, wire services, start servers & workers,
    /// and block until shutdown.
    pub async fn run(self) -> Result<(), Whatever> {
        let handle = self.start().await?;
        handle.wait_for_shutdown().await;
        Ok(())
    }

    /// Initialize infrastructure, wire services, start servers & workers,
    /// and return a handle for controlling the running application.
    pub async fn start(self) -> Result<AppHandle, Whatever> {
        info!("Initializing job application");

        // -- infrastructure --------------------------------------------------

        let (object_store, db_store, notify_client) = self
            .init_infra()
            .await
            .whatever_context("Failed to initialize infrastructure services")?;

        let app_state = job_workers::worker_state::AppState::init(
            &db_store,
            object_store,
            notify_client,
            &self.crawl4ai.url,
        )
        .await
        .whatever_context("Failed to initialize application state")?;

        // -- HTTP routes -----------------------------------------------------

        let domain_routes = app_state.routes();
        let routes_fn: Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync> =
            Box::new(move |router| health_routes(router).merge(domain_routes.clone()));

        info!("Application initialized successfully");

        // -- start servers ---------------------------------------------------

        let running = Arc::new(AtomicBool::new(true));
        let cancellation_token = CancellationToken::new();

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let app_handle = AppHandle {
            shutdown_tx:        Some(shutdown_tx),
            running:            Arc::clone(&running),
            cancellation_token: cancellation_token.clone(),
        };

        let mut grpc_handle = start_grpc_server(&self.grpc, &[Arc::new(HelloService)])
            .whatever_context("Failed to start gRPC server")?;

        info!("starting rest server ...");
        let mut http_handle = start_rest_server(self.http.clone(), vec![routes_fn])
            .await
            .whatever_context("Failed to start REST server")?;

        grpc_handle
            .wait_for_start()
            .await
            .whatever_context("gRPC server failed to report started")?;
        http_handle
            .wait_for_start()
            .await
            .whatever_context("REST server failed to report started")?;

        // -- background workers ----------------------------------------------

        let worker_runtime = Arc::new(
            job_common_runtime::RuntimeOptions::builder()
                .thread_name("job-worker".to_owned())
                .enable_io(true)
                .enable_time(true)
                .build()
                .create()
                .whatever_context("Failed to create worker runtime")?,
        );
        let manager_config = job_common_worker::ManagerConfig {
            runtime:          Some(worker_runtime),
            shutdown_timeout: Duration::from_secs(30),
        };
        let mut worker_manager =
            job_common_worker::Manager::with_state_and_config(app_state.clone(), manager_config);

        let analyze_handle = worker_manager
            .fallible_worker(job_workers::saved_job_analyze::SavedJobAnalyzeWorker)
            .name("saved-job-analyze")
            .eager()
            .on_notify()
            .spawn();

        if let Ok(mut guard) = app_state.analyze_notify.write() {
            *guard = Some(analyze_handle);
        }

        let crawl_handle = worker_manager
            .fallible_worker(job_workers::saved_job_crawl::SavedJobCrawlWorker)
            .name("saved-job-crawl")
            .eager()
            .on_notify()
            .spawn();
        app_state.saved_job_service.set_notify_trigger(crawl_handle);

        let gc_interval_secs = self.gc_interval_hours * 3600;
        let _gc_handle = worker_manager
            .fallible_worker(job_workers::saved_job_gc::SavedJobGcWorker::new(
                job_workers::saved_job_gc::GcConfig::default(),
            ))
            .name("saved-job-gc")
            .interval(std::time::Duration::from_secs(gc_interval_secs))
            .spawn();

        info!("Application started successfully");

        // -- shutdown loop ---------------------------------------------------

        let running_clone = Arc::clone(&running);
        let ct_clone = cancellation_token.clone();

        tokio::spawn(async move {
            shutdown_signal(shutdown_rx).await;

            running_clone.store(false, Ordering::SeqCst);
            ct_clone.cancel();

            info!("Shutting down background workers");
            if tokio::time::timeout(
                std::time::Duration::from_secs(10),
                worker_manager.shutdown(),
            )
            .await
            .is_err()
            {
                error!("Worker manager shutdown timed out; continuing shutdown");
            }

            info!("Shutting down servers");
            grpc_handle.shutdown();
            http_handle.shutdown();

            info!("Application shutdown complete");
        });

        Ok(app_handle)
    }

    async fn init_infra(&self) -> Result<(Operator, DBStore, NotifyClient), Whatever> {
        let object_store = self
            .object_store
            .open()
            .whatever_context("Failed to initialize object store")?;
        info!("Object store (MinIO) configured");

        let db_store = self
            .database
            .open()
            .await
            .whatever_context("Failed to initialize database")?;
        info!("Database initialized");

        let notify_client = NotifyClient::new(db_store.clone())
            .await
            .whatever_context("Failed to initialize notify queue client")?;
        info!("Notify queue client initialized");

        Ok((object_store, db_store, notify_client))
    }
}

// ---------------------------------------------------------------------------
// AppHandle
// ---------------------------------------------------------------------------

/// Handle for controlling a running application.
#[allow(dead_code)]
pub struct AppHandle {
    shutdown_tx:        Option<oneshot::Sender<()>>,
    running:            Arc<AtomicBool>,
    cancellation_token: CancellationToken,
}

#[allow(dead_code)]
impl AppHandle {
    /// Gracefully shutdown the application.
    pub fn shutdown(&mut self) {
        info!("Initiating graceful shutdown");
        self.running.store(false, Ordering::SeqCst);
        self.cancellation_token.cancel();

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Check if the application is still running.
    #[must_use]
    pub fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }

    /// Wait for the application to shutdown.
    pub async fn wait_for_shutdown(&self) { self.cancellation_token.cancelled().await; }
}

async fn shutdown_signal(shutdown_rx: oneshot::Receiver<()>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => { info!("Received Ctrl+C signal"); },
        () = terminate => { info!("Received terminate signal"); },
        _ = shutdown_rx => { info!("Received shutdown signal"); },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = AppConfig::default();
        assert_eq!(config.object_store.endpoint, "http://localhost:9000");
        assert_eq!(config.object_store.bucket, "job-markdown");
        assert_eq!(config.crawl4ai.url, "http://localhost:11235");
        assert_eq!(config.gc_interval_hours, 24);
    }

    #[tokio::test]
    async fn test_app_handle_shutdown() {
        let config = AppConfig::default();
        let Ok(handle) = config.start().await else {
            return;
        };

        let mut handle = handle;
        assert!(handle.is_running());

        handle.shutdown();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert!(!handle.is_running());
    }
}
