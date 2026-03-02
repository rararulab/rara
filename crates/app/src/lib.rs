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
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use opendal::Operator;
use rara_domain_shared::notify::client::NotifyClient;
use rara_kernel::io::{egress::EndpointRegistry, ingress::IngressPipeline, stream::StreamHub};
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
/// Loaded exclusively from Consul KV. All fields are required;
/// missing keys cause startup failure with a clear error.
///
/// For runtime-changeable values (OpenRouter key, Telegram token, …) see
/// `rara_backend_admin::settings::SettingsSvc`.
#[derive(Debug, Clone, bon::Builder, Deserialize)]
#[builder(on(String, into))]
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
    /// Memory backend configuration.
    pub memory:                 MemoryConfig,
    /// Langfuse observability (host, API keys).
    pub langfuse:               LangfuseConfig,
    /// General OTLP telemetry (Alloy/Tempo).
    #[serde(default)]
    pub telemetry:              TelemetryConfig,
    /// JWT signing secret.
    pub jwt_secret:             Option<String>,
}

#[derive(Debug, Clone, bon::Builder, Deserialize)]
#[builder(on(String, into))]
pub struct MemoryConfig {
    pub mem0_base_url:      String,
    pub memos_base_url:     String,
    pub memos_token:        String,
    pub hindsight_base_url: String,
    pub hindsight_bank_id:  String,
}

#[derive(Debug, Clone, bon::Builder, Deserialize)]
#[builder(on(String, into))]
pub struct LangfuseConfig {
    pub host:       String,
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default)]
    pub secret_key: Option<String>,
}

/// General OTLP telemetry configuration (non-Langfuse).
#[derive(Debug, Clone, Default, bon::Builder, Deserialize)]
pub struct TelemetryConfig {
    /// OTLP endpoint URL (e.g. `http://alloy:4318/v1/traces`).
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
    /// Export protocol: `"http"` or `"grpc"`.
    #[serde(default)]
    pub otlp_protocol: Option<String>,
}

// ---------------------------------------------------------------------------
// StartOptions
// ---------------------------------------------------------------------------

/// Options for starting the application with custom adapters.
///
/// Used by `start_with_options` to inject pre-created adapters
/// (e.g. a [`TerminalAdapter`](rara_channels::terminal::TerminalAdapter)
/// for the CLI chat command).
#[derive(Default)]
pub struct StartOptions {
    /// CLI terminal adapter (if running in interactive CLI mode).
    pub cli_adapter: Option<Arc<rara_channels::terminal::TerminalAdapter>>,
}

impl AppConfig {
    /// Load config from Consul KV.
    ///
    /// Requires `CONSUL_HTTP_ADDR` environment variable to be set.
    /// All config keys must be present in Consul; missing keys cause
    /// a deserialization error at startup.
    pub async fn new() -> Result<Self, config::ConfigError> {
        let consul_addr = base::env::var("CONSUL_HTTP_ADDR")
            .map_err(|e| config::ConfigError::Message(e.to_string()))?
            .ok_or_else(|| {
                config::ConfigError::Message("CONSUL_HTTP_ADDR is required but not set".to_string())
            })?;

        tracing::info!(%consul_addr, "Loading configuration from Consul KV");
        let consul_config = rara_consul::Config {
            address: consul_addr,
            ..Default::default()
        };

        let cfg = config::ConfigBuilder::<config::builder::AsyncState>::default()
            .add_async_source(rara_consul::ConsulSource::new(consul_config))
            .build()
            .await?;

        tracing::info!(?cfg, "Raw configuration from Consul");
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
        self.start_with_options(StartOptions::default()).await
    }

    /// Initialize infrastructure, wire services, start servers & workers,
    /// and return a handle for controlling the running application.
    ///
    /// Accepts [`StartOptions`] for injecting pre-created adapters.
    pub async fn start_with_options(self, options: StartOptions) -> Result<AppHandle, Whatever> {
        info!("Initializing job application");

        // -- infrastructure --------------------------------------------------

        let (object_store, db_store, notify_client) = self
            .init_infra()
            .await
            .whatever_context("Failed to initialize infrastructure services")?;

        let pool = db_store.pool().clone();

        // -- runtime settings (needed before RaraState for settings_provider) -

        let settings_svc =
            rara_backend_admin::settings::SettingsSvc::load(db_store.kv_store(), pool.clone())
                .await
                .whatever_context("Failed to initialize runtime settings")?;
        let settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider> =
            Arc::new(settings_svc.clone());
        info!("Runtime settings service loaded");

        // -- RaraState (kernel deps) -----------------------------------------

        let rara = rara_boot::state::RaraState::init(
            pool.clone(),
            object_store,
            notify_client.clone(),
            settings_provider.clone(),
            self.memory.mem0_base_url.clone(),
            self.memory.memos_base_url.clone(),
            self.memory.memos_token.clone(),
            self.memory.hindsight_base_url.clone(),
            self.memory.hindsight_bank_id.clone(),
        )
        .await
        .whatever_context("Failed to initialize RaraState")?;

        // -- BackendState (domain services) ----------------------------------

        let backend = rara_backend_admin::state::BackendState::init(
            pool.clone(),
            notify_client.clone(),
            rara.session_repo.clone(),
            settings_provider.clone(),
            settings_svc.clone(),
        )
        .await
        .whatever_context("Failed to initialize BackendState")?;

        // -- Kernel (boot) ---------------------------------------------------

        // Ensure root + system users exist before booting the kernel.
        rara_boot::user_store::ensure_default_users(&pool)
            .await
            .whatever_context("Failed to ensure default kernel users")?;

        // PathGuard wraps NoopGuard with file-system access control.
        let workspace_path = std::env::current_dir()
            .whatever_context("Failed to determine current working directory")?;
        let sandbox_config =
            rara_boot::guard::sandbox_config_from_settings(settings_provider.as_ref()).await;
        let path_guard = Arc::new(rara_kernel::guard::path_guard::PathGuard::new(
            sandbox_config,
            workspace_path,
            Box::new(rara_kernel::defaults::noop_guard::NoopGuard),
        ));

        // Hot-reload: subscribe to settings changes and update PathGuard
        {
            let guard_ref = path_guard.clone();
            let settings_ref = settings_provider.clone();
            tokio::spawn(async move {
                let mut rx = settings_ref.subscribe();
                while rx.changed().await.is_ok() {
                    let new_config =
                        rara_boot::guard::sandbox_config_from_settings(settings_ref.as_ref()).await;
                    guard_ref.update_config(new_config);
                    tracing::info!("PathGuard sandbox config reloaded from settings");
                }
            });
        }

        // -- AgentFS for persistent KV + tool call audit ---------------------

        let data_dir = rara_paths::data_dir();
        let (kv_backend, tool_recorder): (
            Option<Arc<dyn rara_kernel::kv::KvBackend>>,
            Option<Arc<dyn rara_kernel::audit::ToolCallRecorder>>,
        ) = match rara_boot::agentfs::init_agentfs(&data_dir).await {
            Ok(agentfs) => {
                let agentfs = Arc::new(agentfs);
                let agentfs_path = data_dir.join("agentfs");
                info!("AgentFS initialized at {}", agentfs_path.display());
                (
                    Some(Arc::new(rara_boot::agentfs::AgentFsKv::new(
                        agentfs.clone(),
                    ))),
                    Some(Arc::new(rara_boot::agentfs::AgentFsToolCallRecorder::new(
                        agentfs,
                    ))),
                )
            }
            Err(e) => {
                warn!(error = %e, "AgentFS init failed, falling back to in-memory defaults");
                (None, None)
            }
        };
        let mut kernel = rara_boot::kernel::boot(rara_boot::kernel::BootConfig {
            provider_registry: rara.provider_registry.clone(),
            tool_registry: rara.tool_registry.clone(),
            agent_registry: Arc::new(rara_boot::manifests::load_default_registry()),
            user_store: rara.user_store.clone(),
            session_repo: rara.session_repo.clone(),
            settings: settings_provider.clone(),
            guard: Some(path_guard as Arc<dyn rara_kernel::guard::Guard>),
            kv_backend,
            tool_call_recorder: tool_recorder,
            ..Default::default()
        });

        // -- HTTP routes (need kernel Arc for agent/kernel routes) -----------

        // Create WebAdapter early so we can mount its router into the HTTP server.
        let web_adapter = Arc::new(rara_channels::web::WebAdapter::new());
        let web_router = web_adapter.router();

        // -- telegram adapter (optional) --------------------------------------

        let telegram_adapter = match Self::try_build_telegram(
            &backend.settings_svc,
            &backend.contact_repo,
            pool.clone(),
            &self.main_service_http_base,
        )
        .await
        {
            Ok(Some(adapter)) => {
                info!("Telegram adapter built");
                Some(adapter)
            }
            Ok(None) => {
                info!("Telegram not configured (bot_token unset in settings), skipping");
                None
            }
            Err(e) => {
                warn!(error = %e, "Failed to build Telegram adapter, skipping");
                None
            }
        };

        // Register egress adapters.
        if let Some(ref tg) = telegram_adapter {
            use rara_kernel::{channel::types::ChannelType, io::egress::EgressAdapter};
            kernel.register_adapter(ChannelType::Telegram, tg.clone() as Arc<dyn EgressAdapter>);
        }
        {
            use rara_kernel::{channel::types::ChannelType, io::egress::EgressAdapter};
            kernel.register_adapter(
                ChannelType::Web,
                web_adapter.clone() as Arc<dyn EgressAdapter>,
            );
        }
        if let Some(ref cli) = options.cli_adapter {
            use rara_kernel::{channel::types::ChannelType, io::egress::EgressAdapter};
            kernel.register_adapter(ChannelType::Cli, cli.clone() as Arc<dyn EgressAdapter>);
        }

        // Inject StreamHub / EndpointRegistry into WebAdapter before start.
        web_adapter
            .set_stream_hub(kernel.stream_hub().clone())
            .await;
        web_adapter
            .set_endpoint_registry(kernel.endpoint_registry().clone())
            .await;

        // Start kernel I/O subsystem (TickLoop + Egress).
        // start() consumes self and returns (Arc<Kernel>, KernelHandle).
        let cancellation_token = CancellationToken::new();
        let (_kernel_arc, kernel_handle) = kernel.start(cancellation_token.clone());

        // Now build routes with the KernelHandle.
        let (domain_routes, openapi) = backend.routes(
            &kernel_handle,
            &rara.skill_registry,
            &rara.mcp_manager,
            &rara.coding_task_service,
        );
        let swagger_ui =
            utoipa_swagger_ui::SwaggerUi::new("/swagger-ui").url("/api/openapi.json", openapi);

        // -- Auth service + routes -------------------------------------------
        let jwt_secret = self.jwt_secret.clone().unwrap_or_else(|| {
            warn!("JWT_SECRET not configured, using default secret 'rara'");
            "rara".to_string()
        });
        let jwt_config = rara_domain_user::jwt::JwtConfig::new(jwt_secret.clone());
        let auth_service =
            rara_domain_user::service::AuthService::new(pool.clone(), jwt_config.clone());
        let auth_routes =
            rara_domain_user::router::auth_routes(auth_service).layer(axum::Extension(jwt_config));

        // Inject JWT secret into WebAdapter for WebSocket auth.
        web_adapter.set_jwt_secret(jwt_secret).await;

        let routes_fn: Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync> =
            Box::new(move |router| {
                health_routes(router)
                    .merge(domain_routes.clone())
                    .merge(auth_routes.clone())
                    .merge(swagger_ui.clone())
                    .nest("/api/v1/kernel/chat", web_router.clone())
            });

        info!("Application initialized successfully");

        // -- start servers ---------------------------------------------------

        let running = Arc::new(AtomicBool::new(true));

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

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
        let mut worker_manager = common_worker::Manager::with_state_and_config((), manager_config);

        // Start channel adapters with the kernel's ingress pipeline.
        if let Some(ref tg_adapter) = telegram_adapter {
            use rara_kernel::channel::adapter::ChannelAdapter as _;
            match tg_adapter
                .start(kernel_handle.ingress_pipeline().clone())
                .await
            {
                Ok(()) => info!("Telegram adapter started"),
                Err(e) => warn!(
                    error = %e,
                    "Failed to start Telegram adapter"
                ),
            }
        }
        {
            use rara_kernel::channel::adapter::ChannelAdapter as _;
            match web_adapter
                .start(kernel_handle.ingress_pipeline().clone())
                .await
            {
                Ok(()) => info!("WebAdapter started"),
                Err(e) => warn!(
                    error = %e,
                    "Failed to start WebAdapter"
                ),
            }
        }
        info!("Kernel I/O subsystem running");

        info!("Application started successfully");

        // -- build app handle ------------------------------------------------

        let app_handle = AppHandle {
            shutdown_tx:        Some(shutdown_tx),
            running:            Arc::clone(&running),
            cancellation_token: cancellation_token.clone(),
            ingress_pipeline:   Some(kernel_handle.ingress_pipeline().clone()),
            endpoint_registry:  Some(kernel_handle.endpoint_registry().clone()),
            stream_hub:         Some(kernel_handle.stream_hub().clone()),
        };

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
                use rara_kernel::channel::adapter::ChannelAdapter as _;
                info!("Shutting down Telegram adapter");
                let _ = adapter.stop().await;
            }

            {
                use rara_kernel::channel::adapter::ChannelAdapter as _;
                info!("Shutting down WebAdapter");
                let _ = web_adapter.stop().await;
            }

            info!("Shutting down servers");
            grpc_handle.shutdown();
            http_handle.shutdown();

            info!("Application shutdown complete");
        });

        Ok(app_handle)
    }

    /// Try to build the Telegram adapter using shared infrastructure.
    ///
    /// Returns `Ok(Some(adapter))` if the adapter was built successfully,
    /// `Ok(None)` if Telegram is not configured, or `Err` on failure.
    ///
    /// The adapter is NOT started here — the caller is responsible for
    /// calling `adapter.start(sink)` after the I/O pipeline is ready.
    async fn try_build_telegram(
        settings_svc: &rara_backend_admin::settings::SettingsSvc,
        contact_repo: &rara_channels::telegram::contacts::repository::ContactRepository,
        pool: sqlx::PgPool,
        main_service_http_base: &str,
    ) -> Result<Option<Arc<rara_channels::telegram::TelegramAdapter>>, Whatever> {
        use rara_domain_shared::settings::{SettingsProvider, keys};

        let settings: Arc<dyn SettingsProvider> = Arc::new(settings_svc.clone());
        let token = match settings.get(keys::TELEGRAM_BOT_TOKEN).await {
            Some(t) if !t.is_empty() => t,
            _ => return Ok(None),
        };

        let proxy = std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .or_else(|_| std::env::var("ALL_PROXY"))
            .ok()
            .filter(|v| !v.is_empty());
        if let Some(ref p) = proxy {
            info!(proxy = %p, "telegram adapter: using proxy");
        }

        // Read initial settings for primary/group chat IDs.
        let chat_id: Option<i64> = settings
            .get(keys::TELEGRAM_CHAT_ID)
            .await
            .and_then(|v| v.parse().ok());
        let group_id: Option<i64> = settings
            .get(keys::TELEGRAM_ALLOWED_GROUP_CHAT_ID)
            .await
            .and_then(|v| v.parse().ok());

        let mut tg_config = rara_channels::telegram::TelegramConfig::default();
        tg_config.primary_chat_id = chat_id;
        tg_config.allowed_group_chat_id = group_id;

        // Build contact tracker from the contact repository.
        let contact_tracker: Arc<dyn rara_channels::telegram::contacts::ContactTracker> =
            Arc::new(ContactRepoTracker {
                repo: contact_repo.clone(),
            });

        // Build link service for /link command handling.
        let link_service = rara_channels::telegram::TelegramLinkService::new(
            pool,
            main_service_http_base.to_string(),
        );

        let adapter = Arc::new(
            rara_channels::telegram::TelegramAdapter::with_proxy(&token, vec![], proxy.as_deref())
                .whatever_context("failed to build telegram adapter")?
                .with_config(tg_config)
                .with_contact_tracker(contact_tracker)
                .with_link_service(link_service),
        );

        // Spawn a background task to hot-reload config from settings.
        let config_handle = adapter.config_handle();
        let mut settings_rx = settings.subscribe();
        tokio::spawn(async move {
            while settings_rx.changed().await.is_ok() {
                let new_chat_id: Option<i64> = settings
                    .get(keys::TELEGRAM_CHAT_ID)
                    .await
                    .and_then(|v| v.parse().ok());
                let new_group_id: Option<i64> = settings
                    .get(keys::TELEGRAM_ALLOWED_GROUP_CHAT_ID)
                    .await
                    .and_then(|v| v.parse().ok());
                let mut cfg = config_handle.write().unwrap_or_else(|e| e.into_inner());
                cfg.primary_chat_id = new_chat_id;
                cfg.allowed_group_chat_id = new_group_id;
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
    shutdown_tx:           Option<oneshot::Sender<()>>,
    running:               Arc<AtomicBool>,
    cancellation_token:    CancellationToken,
    /// The ingress pipeline (for injecting inbound messages).
    pub ingress_pipeline:  Option<Arc<IngressPipeline>>,
    /// Per-user endpoint registry (for registering CLI endpoints).
    pub endpoint_registry: Option<Arc<EndpointRegistry>>,
    /// Ephemeral stream hub (for subscribing to real-time deltas).
    pub stream_hub:        Option<Arc<StreamHub>>,
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

    fn test_config() -> AppConfig {
        AppConfig::builder()
            .database(
                DatabaseConfig::builder()
                    .database_url("postgres://postgres:postgres@localhost:5432/rara_test")
                    .migration_dir("crates/rara-model/migrations")
                    .build(),
            )
            .http(RestServerConfig::default())
            .grpc(GrpcServerConfig::default())
            .object_store(object_store::ObjectStoreConfig::default())
            .main_service_http_base("http://127.0.0.1:25555")
            .memory(
                MemoryConfig::builder()
                    .mem0_base_url("http://localhost:8080")
                    .memos_base_url("http://localhost:5230")
                    .memos_token("")
                    .hindsight_base_url("http://localhost:8888")
                    .hindsight_bank_id("default")
                    .build(),
            )
            .langfuse(
                LangfuseConfig::builder()
                    .host("http://localhost:3000")
                    .build(),
            )
            .telemetry(TelemetryConfig::builder().build())
            .build()
    }

    #[tokio::test]
    async fn test_app_handle_shutdown() {
        let config = test_config();
        let Ok(handle) = config.start().await else {
            return;
        };

        let mut handle: AppHandle = handle;
        assert!(handle.is_running());

        handle.shutdown();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert!(!handle.is_running());
    }
}
