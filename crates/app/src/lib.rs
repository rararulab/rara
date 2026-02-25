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

use opendal::Operator;
use rara_domain_shared::notify::client::NotifyClient;
use rara_server::{
    grpc::{GrpcServerConfig, hello::HelloService, start_grpc_server},
    http::{RestServerConfig, health_routes, start_rest_server},
};
use serde::Deserialize;
use snafu::{ResultExt, Whatever};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use yunara_store::{config::DatabaseConfig, db::DBStore};

// ---------------------------------------------------------------------------
// Static config types (immutable after startup)
// ---------------------------------------------------------------------------

/// Static application configuration — immutable after startup.
///
/// Loaded from Consul KV (when `CONSUL_HTTP_ADDR` is set) or
/// `RARA__`-prefixed environment variables (local dev fallback).
/// For runtime-changeable values (OpenRouter key, Telegram token, …) see
/// [`rara_backend_admin::settings::SettingsSvc`].
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
    pub object_store:           object_store::ObjectStoreConfig,
    /// Main service HTTP base URL (for telegram bot → main service calls).
    pub main_service_http_base: String,
    /// Memory backend configuration (static, not runtime settings).
    pub memory:                 MemoryConfig,
    /// Langfuse observability (host, API keys).
    pub langfuse:               LangfuseConfig,
    /// General OTLP telemetry (Alloy/Tempo).
    pub telemetry:              TelemetryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut object_store = object_store::ObjectStoreConfig::default();
        object_store.bucket = "rara".to_owned();
        Self {
            database: DatabaseConfig::default(),
            http: RestServerConfig::default(),
            grpc: GrpcServerConfig::default(),
            object_store,
            main_service_http_base: "http://127.0.0.1:25555".to_owned(),
            memory: MemoryConfig::default(),
            langfuse: LangfuseConfig::default(),
            telemetry: TelemetryConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub chroma_url:        String,
    pub chroma_collection: Option<String>,
    pub chroma_api_key:    Option<String>,
    pub mem0_base_url:     Option<String>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            chroma_url: "http://localhost:8000".to_owned(),
            chroma_collection: Some("job-memory".to_owned()),
            chroma_api_key: None,
            mem0_base_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LangfuseConfig {
    pub host: String,
    pub public_key: Option<String>,
    pub secret_key: Option<String>,
}

impl Default for LangfuseConfig {
    fn default() -> Self {
        Self {
            host: "http://localhost:3000".to_owned(),
            public_key: None,
            secret_key: None,
        }
    }
}

/// General OTLP telemetry configuration (non-Langfuse).
///
/// Used when traces should be sent to a generic OTLP collector such as
/// Alloy -> Tempo rather than to Langfuse.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    /// OTLP endpoint URL (e.g. `http://alloy:4318/v1/traces`).
    pub otlp_endpoint: Option<String>,
    /// Export protocol: `"http"` (default) or `"grpc"`.
    pub otlp_protocol: Option<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: None,
            otlp_protocol: None,
        }
    }
}

impl AppConfig {
    /// Load config from Consul KV or environment variables.
    ///
    /// When `CONSUL_HTTP_ADDR` is set:
    ///   1. Consul KV entries (highest priority)
    ///   2. Code defaults
    ///
    /// When Consul is NOT configured (local dev):
    ///   1. `RARA__`-prefixed environment variables (highest priority)
    ///   2. Code defaults
    pub async fn new() -> Result<Self, config::ConfigError> {
        let consul_http_addr = base::env::var("CONSUL_HTTP_ADDR")
            .map_err(|e| config::ConfigError::Message(e.to_string()))?;

        let builder = if let Some(addr) = consul_http_addr {
            tracing::info!(%addr, "Loading configuration from Consul KV");
            let consul_config = rara_consul::Config {
                address: addr,
                ..Default::default()
            };
            config::ConfigBuilder::<config::builder::AsyncState>::default()
                .add_async_source(rara_consul::ConsulSource::new(consul_config))
        } else {
            tracing::info!("Consul not configured, loading from environment variables");
            config::ConfigBuilder::<config::builder::AsyncState>::default().add_source(
                config::Environment::with_prefix("RARA")
                    .separator("__")
                    .try_parsing(true),
            )
        };

        let cfg = builder.build().await?;
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

        let app_state = rara_workers::worker_state::AppState::init(
            &db_store,
            object_store,
            notify_client.clone(),
            self.memory.chroma_url.clone(),
            self.memory.chroma_collection.clone(),
            self.memory.chroma_api_key.clone(),
        )
        .await
        .whatever_context("Failed to initialize application state")?;

        // -- HTTP routes -----------------------------------------------------

        let (domain_routes, openapi) = app_state.routes();
        let swagger_ui =
            utoipa_swagger_ui::SwaggerUi::new("/swagger-ui").url("/api/openapi.json", openapi);
        let routes_fn: Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync> =
            Box::new(move |router| {
                health_routes(router)
                    .merge(domain_routes.clone())
                    .merge(swagger_ui.clone())
            });

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
            common_runtime::RuntimeOptions::builder()
                .thread_name("raraworker".to_owned())
                .enable_io(true)
                .enable_time(true)
                .build()
                .create()
                .whatever_context("Failed to create worker runtime")?,
        );
        let manager_config = common_worker::ManagerConfig {
            runtime:          Some(worker_runtime),
            shutdown_timeout: Duration::from_secs(30),
        };
        let mut worker_manager =
            common_worker::Manager::with_state_and_config(app_state.clone(), manager_config);

        // Read worker interval settings (applied at startup; restart to change).
        let worker_cfg = app_state.settings_svc.current().workers;
        info!(
            agent_scheduler_secs  = worker_cfg.agent_scheduler_interval_secs,
            pipeline_scheduler_secs = worker_cfg.pipeline_scheduler_interval_secs,
            memory_sync_secs      = worker_cfg.memory_sync_interval_secs,
            proactive_hours       = worker_cfg.proactive_agent_interval_hours,
            "Worker intervals from settings"
        );

        let proactive_handle = worker_manager
            .fallible_worker(rara_workers::proactive::ProactiveAgentWorker)
            .name("proactive-agent")
            .eager()
            .interval_or_notify(Duration::from_secs(
                worker_cfg.proactive_agent_interval_hours * 3600,
            ))
            .spawn();
        if let Ok(mut guard) = app_state.proactive_notify.write() {
            *guard = Some(proactive_handle);
        }

        // -- memory sync worker -----------------------------------------------

        let _memory_sync_handle = worker_manager
            .fallible_worker(rara_workers::memory_sync::MemorySyncWorker)
            .name("memory-sync")
            .interval(Duration::from_secs(worker_cfg.memory_sync_interval_secs))
            .spawn();

        // -- agent scheduler worker -------------------------------------------

        let _scheduler_handle = worker_manager
            .fallible_worker(rara_workers::scheduled_agent::AgentSchedulerWorker::new(
                app_state.agent_scheduler.clone(),
            ))
            .name("agent-scheduler")
            .interval(Duration::from_secs(worker_cfg.agent_scheduler_interval_secs))
            .spawn();

        // -- pipeline scheduler worker (checks cron from settings) ------------

        let _pipeline_scheduler_handle = worker_manager
            .fallible_worker(
                rara_workers::pipeline_scheduler::PipelineSchedulerWorker::new(),
            )
            .name("pipeline-scheduler")
            .interval(Duration::from_secs(
                worker_cfg.pipeline_scheduler_interval_secs,
            ))
            .spawn();

        // -- telegram bot (optional) -----------------------------------------

        let bot_handle = match Self::try_start_bot(
            &cancellation_token,
            &notify_client,
            app_state.settings_svc.subscribe(),
            &self.main_service_http_base,
            db_store.pool().clone(),
        )
        .await
        {
            Ok(Some(handle)) => {
                info!("Telegram bot started");
                Some(handle)
            }
            Ok(None) => {
                info!("Telegram bot not configured, skipping");
                None
            }
            Err(e) => {
                warn!(error = %e, "Failed to start telegram bot, skipping");
                None
            }
        };

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

            // Extract the worker runtime so it is NOT dropped inside this async
            // context.  Dropping a Tokio `Runtime` from within an async task
            // panics with "Cannot drop a runtime in a context where blocking is
            // not allowed".  Move it to a blocking thread for safe teardown.
            let worker_rt = worker_manager.take_runtime();
            drop(worker_manager);
            if let Some(rt) = worker_rt {
                tokio::task::spawn_blocking(move || drop(rt));
            }

            if let Some(bot) = bot_handle {
                info!("Shutting down telegram bot");
                bot.shutdown().await;
            }

            info!("Shutting down servers");
            grpc_handle.shutdown();
            http_handle.shutdown();

            info!("Application shutdown complete");
        });

        Ok(app_handle)
    }

    /// Try to start the Telegram bot using shared infrastructure.
    ///
    /// Returns `Ok(Some(handle))` if the bot started successfully,
    /// `Ok(None)` if Telegram is not configured, or `Err` on failure.
    async fn try_start_bot(
        cancel: &CancellationToken,
        notify_client: &NotifyClient,
        settings_rx: tokio::sync::watch::Receiver<rara_domain_shared::settings::model::Settings>,
        main_service_http_base: &str,
        pool: sqlx::PgPool,
    ) -> Result<Option<rara_telegram_bot::BotHandle>, Whatever> {
        let telegram_config = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .map(|token| rara_telegram_bot::TelegramConfig { bot_token: token });

        let bot_app = rara_telegram_bot::BotApp::from_shared(
            cancel.child_token(),
            settings_rx,
            Arc::new(notify_client.clone()),
            telegram_config,
            main_service_http_base.to_owned(),
            pool,
        )
        .await
        .whatever_context("Failed to initialize telegram bot")?;

        Ok(bot_app.map(rara_telegram_bot::BotApp::spawn))
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
        assert_eq!(config.object_store.bucket, "raramarkdown");
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
