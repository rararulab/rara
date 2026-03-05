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

pub mod flatten;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rara_kernel::channel::{adapter::ChannelAdapter, types::ChannelType};
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
/// Loaded from a YAML config file (see [`rara_paths::config_file()`]).
/// All required fields must be present; missing keys cause startup
/// failure with a clear error.
///
/// For runtime-changeable values (OpenRouter key, Telegram token, …) see
/// `rara_backend_admin::settings::SettingsSvc`.
#[derive(Debug, Clone, bon::Builder, Deserialize)]
#[builder(on(String, into))]
pub struct AppConfig {
    /// Database connection pool (optional — defaults to max_connections=5).
    #[serde(default = "default_database_config")]
    pub database:    DatabaseConfig,
    /// HTTP server bind / limits.
    pub http:        RestServerConfig,
    /// gRPC server bind / limits.
    pub grpc:        GrpcServerConfig,
    /// General OTLP telemetry (Alloy/Tempo).
    #[serde(default)]
    pub telemetry:   TelemetryConfig,
    /// Static bearer token for owner authentication (Web UI).
    pub owner_token: Option<String>,
    /// LLM provider configuration (seeded to settings store at startup).
    #[serde(default)]
    pub llm:         Option<flatten::LlmConfig>,
    /// Telegram bot configuration (seeded to settings store at startup).
    #[serde(default)]
    pub telegram:    Option<flatten::TelegramConfig>,
    /// Configured users with platform identity mappings (required).
    pub users:       Vec<rara_boot::user_store::UserConfig>,
}

/// General OTLP telemetry configuration.
#[derive(Debug, Clone, Default, bon::Builder, Deserialize)]
pub struct TelemetryConfig {
    /// OTLP endpoint URL (e.g. `http://alloy:4318/v1/traces`).
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
    /// Export protocol: `"http"` or `"grpc"`.
    #[serde(default)]
    pub otlp_protocol: Option<String>,
}

fn default_database_config() -> DatabaseConfig { DatabaseConfig::builder().build() }

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
    /// Load config from YAML files.
    ///
    /// Sources (later sources override earlier ones):
    /// - **release**: `~/.config/job/config.yaml` (global) → `./config.yaml`
    ///   (local override)
    /// - **debug**: `./config.yaml` only
    ///
    /// All required fields must be present after merging; missing
    /// keys cause a deserialization error at startup.
    pub fn new() -> Result<Self, config::ConfigError> {
        let mut builder = config::Config::builder();

        // Global config path only in release mode.
        #[cfg(not(debug_assertions))]
        {
            builder = builder.add_source(
                config::File::from(rara_paths::config_file().as_path())
                    .format(config::FileFormat::Yaml)
                    .required(false),
            );
        }

        builder = builder
            .add_source(config::File::new("config", config::FileFormat::Yaml).required(true));

        let cfg = builder.build()?;
        tracing::info!(?cfg, "Raw configuration");
        cfg.try_deserialize()
    }
}

/// Initialize infrastructure, wire services, start servers & workers,
/// and block until shutdown.
pub async fn run(config: AppConfig) -> Result<(), Whatever> {
    let handle = start(config).await?;
    handle.wait_for_shutdown().await;
    Ok(())
}

/// Initialize infrastructure, wire services, start servers & workers,
/// and return a handle for controlling the running application.
pub async fn start(config: AppConfig) -> Result<AppHandle, Whatever> {
    start_with_options(config, StartOptions::default()).await
}

/// Initialize infrastructure, wire services, start servers & workers,
/// and return a handle for controlling the running application.
///
/// Accepts [`StartOptions`] for injecting pre-created adapters.
pub async fn start_with_options(
    config: AppConfig,
    options: StartOptions,
) -> Result<AppHandle, Whatever> {
    info!("Initializing job application");

    let db_store = init_infra(&config)
        .await
        .whatever_context("Failed to initialize infrastructure services")?;
    let pool = db_store.pool().clone();

    let settings_svc =
        rara_backend_admin::settings::SettingsSvc::load(db_store.kv_store(), pool.clone())
            .await
            .whatever_context("Failed to initialize runtime settings")?;
    let config_defaults = flatten::flatten_config_sections(&config);
    if !config_defaults.is_empty() {
        settings_svc
            .seed_defaults(config_defaults)
            .await
            .whatever_context("Failed to seed config defaults")?;
    }

    let settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider> =
        Arc::new(settings_svc.clone());
    info!("Runtime settings service loaded");

    let rara =
        rara_boot::state::RaraState::init(pool.clone(), settings_provider.clone(), &config.users)
            .await
            .whatever_context("Failed to initialize RaraState")?;

    let backend = rara_backend_admin::state::BackendState::init(
        rara.session_index.clone(),
        rara.tape_service.clone(),
        settings_provider.clone(),
        settings_svc.clone(),
    )
    .await
    .whatever_context("Failed to initialize BackendState")?;

    let identity_resolver: Arc<dyn rara_kernel::io::IdentityResolver> = Arc::new(
        rara_boot::resolvers::PlatformIdentityResolver::new(&config.users),
    );
    let session_resolver = Arc::new(rara_boot::resolvers::DefaultSessionResolver::new(
        rara.session_index.clone(),
    ));
    let web_adapter = Arc::new(rara_channels::web::WebAdapter::new(
        config.owner_token.clone(),
    ));
    let web_router = web_adapter.router();

    let telegram_adapter = match try_build_telegram(&backend.settings_svc).await {
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

    // Build IOSubsystem with all adapters before passing to Kernel.
    let mut io = rara_kernel::io::IOSubsystem::new(identity_resolver, session_resolver);
    if let Some(ref tg) = telegram_adapter {
        io.register_adapter(ChannelType::Telegram, tg.clone() as Arc<dyn ChannelAdapter>);
    }
    io.register_adapter(
        ChannelType::Web,
        web_adapter.clone() as Arc<dyn ChannelAdapter>,
    );
    if let Some(ref cli) = options.cli_adapter {
        io.register_adapter(ChannelType::Cli, cli.clone() as Arc<dyn ChannelAdapter>);
    }

    let kernel = rara_kernel::kernel::Kernel::new(
        Default::default(),
        rara.driver_registry.clone(),
        rara.tool_registry.clone(),
        Arc::new(rara_boot::manifests::load_default_registry()),
        rara.session_index.clone(),
        rara.tape_service.clone(),
        settings_provider.clone(),
        Arc::new(rara_kernel::security::SecuritySubsystem::new(
            rara.user_store.clone(),
            Arc::new(rara_kernel::security::ApprovalManager::new(
                rara_kernel::security::ApprovalPolicy::default(),
            )),
        )),
        io,
    );

    let cancellation_token = CancellationToken::new();
    let (_kernel_arc, kernel_handle) = kernel.start(cancellation_token.clone());

    let (domain_routes, openapi) =
        backend.routes(&kernel_handle, &rara.skill_registry, &rara.mcp_manager);
    let swagger_ui =
        utoipa_swagger_ui::SwaggerUi::new("/swagger-ui").url("/api/openapi.json", openapi);

    let routes_fn: Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync> =
        Box::new(move |router| {
            health_routes(router)
                .merge(domain_routes.clone())
                .merge(swagger_ui.clone())
                .nest("/api/v1/kernel/chat", web_router.clone())
        });

    info!("Application initialized successfully");

    let running = Arc::new(AtomicBool::new(true));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let mut grpc_handle = start_grpc_server(&config.grpc, &[Arc::new(HelloService)])
        .whatever_context("Failed to start gRPC server")?;
    info!("starting rest server ...");
    let mut http_handle = start_rest_server(config.http.clone(), vec![routes_fn])
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

    if let Some(ref tg_adapter) = telegram_adapter {
        use rara_kernel::channel::adapter::ChannelAdapter as _;
        match tg_adapter.start(kernel_handle.clone()).await {
            Ok(()) => info!("Telegram adapter started"),
            Err(e) => warn!(error = %e, "Failed to start Telegram adapter"),
        }
    }
    {
        use rara_kernel::channel::adapter::ChannelAdapter as _;
        match web_adapter.start(kernel_handle.clone()).await {
            Ok(()) => info!("WebAdapter started"),
            Err(e) => warn!(error = %e, "Failed to start WebAdapter"),
        }
    }
    info!("Kernel I/O subsystem running");
    info!("Application started successfully");

    let app_handle = AppHandle {
        shutdown_tx:        Some(shutdown_tx),
        running:            Arc::clone(&running),
        cancellation_token: cancellation_token.clone(),
        kernel_handle:      Some(kernel_handle),
    };

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

async fn try_build_telegram(
    settings_svc: &rara_backend_admin::settings::SettingsSvc,
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

    let adapter = Arc::new(
        rara_channels::telegram::TelegramAdapter::with_proxy(&token, vec![], proxy.as_deref())
            .whatever_context("failed to build telegram adapter")?
            .with_config(tg_config),
    );

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

async fn init_infra(config: &AppConfig) -> Result<DBStore, Whatever> {
    let db_dir = rara_paths::database_dir();
    std::fs::create_dir_all(db_dir).whatever_context("Failed to create database directory")?;
    let database_url = format!("sqlite:{}/rara.db?mode=rwc", db_dir.display());
    let db_store = config
        .database
        .open(&database_url)
        .await
        .whatever_context("Failed to initialize database")?;
    sqlx::migrate!("../rara-model/migrations")
        .run(db_store.pool())
        .await
        .whatever_context("Failed to run database migrations")?;
    info!("Database initialized");
    Ok(db_store)
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
    /// Kernel handle (for injecting inbound messages, accessing stream hub,
    /// endpoint registry, etc.).
    pub kernel_handle:  Option<rara_kernel::handle::KernelHandle>,
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
