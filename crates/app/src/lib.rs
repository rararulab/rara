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

mod io_pipeline;
mod resolvers;

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
use rara_kernel::channel::adapter::ChannelAdapter as _;
use yunara_store::{config::DatabaseConfig, db::DBStore};

// ---------------------------------------------------------------------------
// Static config types (immutable after startup)
// ---------------------------------------------------------------------------

/// Static application configuration — immutable after startup.
///
/// Loaded from Consul KV (when `CONSUL_HTTP_ADDR` is set) or
/// `RARA__`-prefixed environment variables (local dev fallback).
/// For runtime-changeable values (OpenRouter key, Telegram token, …) see
/// `rara_backend_admin::settings::SettingsSvc`.
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
    /// mem0 base URL -- used in non-k8s mode (direct connection).
    pub mem0_base_url:      String,
    pub memos_base_url:     String,
    pub memos_token:        String,
    pub hindsight_base_url: String,
    pub hindsight_bank_id:  String,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            mem0_base_url:      "http://localhost:8080".to_owned(),
            memos_base_url:     "http://localhost:5230".to_owned(),
            memos_token:        String::new(),
            hindsight_base_url: "http://localhost:8888".to_owned(),
            hindsight_bank_id:  "default".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LangfuseConfig {
    pub host:       String,
    pub public_key: Option<String>,
    pub secret_key: Option<String>,
}

impl Default for LangfuseConfig {
    fn default() -> Self {
        Self {
            host:       "http://localhost:3000".to_owned(),
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
            self.memory.mem0_base_url.clone(),
            self.memory.memos_base_url.clone(),
            self.memory.memos_token.clone(),
            self.memory.hindsight_base_url.clone(),
            self.memory.hindsight_bank_id.clone(),
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
            agent_scheduler_secs = worker_cfg.agent_scheduler_interval_secs,
            pipeline_scheduler_secs = worker_cfg.pipeline_scheduler_interval_secs,
            proactive_hours = worker_cfg.proactive_agent_interval_hours,
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

        // -- agent scheduler worker -------------------------------------------

        let _scheduler_handle = worker_manager
            .fallible_worker(rara_workers::scheduled_agent::AgentSchedulerWorker::new(
                app_state.agent_scheduler.clone(),
            ))
            .name("agent-scheduler")
            .interval(Duration::from_secs(
                worker_cfg.agent_scheduler_interval_secs,
            ))
            .spawn();

        // -- pipeline scheduler worker (checks cron from settings) ------------

        let _pipeline_scheduler_handle = worker_manager
            .fallible_worker(rara_workers::pipeline_scheduler::PipelineSchedulerWorker::new())
            .name("pipeline-scheduler")
            .interval(Duration::from_secs(
                worker_cfg.pipeline_scheduler_interval_secs,
            ))
            .spawn();

        // -- telegram adapter (optional) --------------------------------------

        let telegram_adapter = match Self::try_start_telegram(&app_state).await {
            Ok(Some(adapter)) => {
                info!("Telegram adapter started");
                Some(adapter)
            }
            Ok(None) => {
                info!("Telegram not configured (TELEGRAM_BOT_TOKEN unset), skipping");
                None
            }
            Err(e) => {
                warn!(error = %e, "Failed to start Telegram adapter, skipping");
                None
            }
        };

        // -- I/O Bus pipeline (new path, alongside ChatService) ---------------

        let io_pipeline =
            io_pipeline::init_io_pipeline(telegram_adapter.clone());

        // Start TickLoop in the background.
        // The TickLoop drains the InboundBus and dispatches messages through
        // the SessionScheduler to the AgentExecutor.
        let tick_loop = io_pipeline.tick_loop;
        tokio::spawn({
            let token = cancellation_token.clone();
            async move {
                tick_loop.run(token).await;
            }
        });

        // NOTE: Egress is NOT spawned yet because `Egress` holds a
        // `Box<dyn OutboundSubscriber>` which is `Send` but not `Sync`,
        // making it incompatible with `tokio::spawn`. This will be fixed
        // in the kernel crate (make OutboundSubscriber Sync or restructure
        // Egress). For now, outbound delivery is handled by the legacy
        // ChatService path. The `_egress` binding keeps it alive for
        // future wiring.
        let _egress = io_pipeline.egress;

        // If Telegram is running, also start the sink-mode polling alongside
        // the legacy bridge-mode polling.  Messages will flow into the
        // InboundBus (new path) in parallel with the ChatService path.
        if let Some(ref tg_adapter) = telegram_adapter {
            match tg_adapter
                .start_with_sink(io_pipeline.ingress_pipeline.clone())
                .await
            {
                Ok(()) => info!("Telegram adapter sink-mode (I/O Bus) started"),
                Err(e) => warn!(
                    error = %e,
                    "Failed to start Telegram adapter in sink-mode, I/O Bus ingress inactive"
                ),
            }
        }

        info!(
            inbound_pending = io_pipeline.inbound_bus.pending_count(),
            "I/O Bus pipeline running"
        );

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

            if let Some(adapter) = telegram_adapter {
                info!("Shutting down Telegram adapter");
                let _ = adapter.stop().await;
            }

            info!("Shutting down servers");
            grpc_handle.shutdown();
            http_handle.shutdown();

            info!("Application shutdown complete");
        });

        Ok(app_handle)
    }

    /// Try to start the Telegram adapter using shared infrastructure.
    ///
    /// Returns `Ok(Some(adapter))` if the adapter started successfully,
    /// `Ok(None)` if Telegram is not configured, or `Err` on failure.
    async fn try_start_telegram(
        state: &rara_workers::worker_state::AppState,
    ) -> Result<Option<Arc<rara_channels::telegram::TelegramAdapter>>, Whatever> {
        let token = match std::env::var("TELEGRAM_BOT_TOKEN") {
            Ok(t) if !t.is_empty() => t,
            _ => return Ok(None),
        };

        let bot = teloxide::Bot::new(&token);

        // Read initial settings for primary/group chat IDs.
        let settings = state.settings_svc.current();
        let tg = &settings.telegram;

        let mut tg_config = rara_channels::telegram::TelegramConfig::default();
        tg_config.primary_chat_id = tg.chat_id;
        tg_config.allowed_group_chat_id = tg.allowed_group_chat_id;

        // Build the bridge that routes messages to the chat service.
        let bridge: Arc<dyn rara_kernel::channel::bridge::ChannelBridge> =
            Arc::new(ChatServiceBridge {
                chat_service: state.chat_service.clone(),
                user_store: state.user_store.clone(),
            });

        // Build contact tracker from the contact repository.
        let contact_tracker: Arc<dyn rara_channels::telegram::contacts::ContactTracker> =
            Arc::new(ContactRepoTracker {
                repo: state.contact_repo.clone(),
            });

        let adapter = Arc::new(
            rara_channels::telegram::TelegramAdapter::new(bot, vec![])
                .with_config(tg_config)
                .with_contact_tracker(contact_tracker),
        );

        // Start long-polling.
        adapter
            .start(bridge)
            .await
            .whatever_context("Failed to start Telegram adapter")?;

        // Spawn a background task to hot-reload config from settings.
        let config_handle = adapter.config_handle();
        let mut settings_rx = state.settings_svc.subscribe();
        tokio::spawn(async move {
            while settings_rx.changed().await.is_ok() {
                let s = settings_rx.borrow_and_update();
                let mut cfg = config_handle.write().unwrap_or_else(|e| e.into_inner());
                cfg.primary_chat_id = s.telegram.chat_id;
                cfg.allowed_group_chat_id = s.telegram.allowed_group_chat_id;
            }
        });

        Ok(Some(adapter))
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

// ---------------------------------------------------------------------------
// ChatServiceBridge — routes ChannelMessages to the ChatService
// ---------------------------------------------------------------------------

/// Bridge implementation that dispatches inbound channel messages to the
/// [`ChatService`](rara_domain_chat::service::ChatService).
///
/// The session key from the [`ChannelMessage`] is used directly — the
/// Telegram adapter already formats it as `tg:<chat_id>`.
struct ChatServiceBridge {
    chat_service: rara_domain_chat::service::ChatService,
    user_store: std::sync::Arc<dyn rara_kernel::process::user::UserStore>,
}

#[async_trait::async_trait]
impl rara_kernel::channel::bridge::ChannelBridge for ChatServiceBridge {
    async fn dispatch(
        &self,
        message: rara_kernel::channel::types::ChannelMessage,
    ) -> Result<String, rara_kernel::error::KernelError> {
        // Validate that the platform user is registered.
        let platform = message.channel_type.label();
        let platform_user_id = &message.user.platform_id;
        let _user = self
            .user_store
            .get_by_platform(platform, platform_user_id)
            .await?
            .ok_or(rara_kernel::error::KernelError::UserNotFound {
                name: format!("{platform}:{platform_user_id}"),
            })?;

        let text = message.content.as_text();

        if text.is_empty() {
            return Ok(String::new());
        }

        let session_key = rara_sessions::types::SessionKey::from_raw(message.session_key.clone());

        // Extract image URLs for multimodal messages.
        let image_urls = match &message.content {
            rara_kernel::channel::types::MessageContent::Multimodal(blocks) => {
                let urls: Vec<String> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        rara_kernel::channel::types::ContentBlock::ImageUrl { url } => {
                            Some(url.clone())
                        }
                        _ => None,
                    })
                    .collect();
                if urls.is_empty() { None } else { Some(urls) }
            }
            _ => None,
        };

        // Ensure session exists (creates on first message).
        let _ = self
            .chat_service
            .ensure_session(&session_key, None, None, None)
            .await
            .map_err(|e| rara_kernel::error::KernelError::Other {
                message: format!("session error: {e}").into(),
            })?;

        let response = self
            .chat_service
            .send_message(&session_key, text, image_urls)
            .await
            .map_err(|e| rara_kernel::error::KernelError::Other {
                message: format!("chat error: {e}").into(),
            })?;

        Ok(response.content.as_text())
    }
}

// ---------------------------------------------------------------------------
// ContactRepoTracker — implements ContactTracker via ContactRepository
// ---------------------------------------------------------------------------

/// Bridges [`ContactTracker`](rara_channels::telegram::contacts::ContactTracker)
/// to the PostgreSQL-backed [`ContactRepository`].
struct ContactRepoTracker {
    repo: rara_channels::telegram::contacts::repository::ContactRepository,
}

#[async_trait::async_trait]
impl rara_channels::telegram::contacts::ContactTracker for ContactRepoTracker {
    async fn track(&self, username: &str, chat_id: i64) {
        if let Err(e) = self.repo.set_chat_id(username, chat_id).await {
            tracing::debug!(username, chat_id, error = %e, "failed to track contact");
        }
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
