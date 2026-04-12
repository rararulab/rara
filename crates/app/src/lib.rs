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

mod boot;
pub mod config_sync;
pub mod flatten;
pub mod gateway;
// Re-export `rara_kernel::tool` so the `ToolDef` proc macro can resolve
// `crate::tool::AgentTool` in derived impls.
pub(crate) use rara_kernel::tool;
mod tools;
mod web_server;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rara_kernel::channel::{
    adapter::ChannelAdapter,
    types::{ChannelType, GroupPolicy},
};
use rara_server::{
    grpc::{GrpcServerConfig, hello::HelloService, start_grpc_server},
    http::{RestServerConfig, health_routes, start_rest_server},
};
use serde::{Deserialize, Serialize};
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
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
#[builder(on(String, into))]
pub struct AppConfig {
    /// Database connection pool (optional — defaults to max_connections=5).
    #[serde(default = "default_database_config")]
    pub database:               DatabaseConfig,
    /// HTTP server bind / limits.
    pub http:                   RestServerConfig,
    /// gRPC server bind / limits.
    pub grpc:                   GrpcServerConfig,
    /// General OTLP telemetry (Alloy/Tempo).
    #[serde(default)]
    pub telemetry:              TelemetryConfig,
    /// Static bearer token for owner authentication (Web UI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_token:            Option<String>,
    /// LLM provider configuration (seeded to settings store at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm:                    Option<flatten::LlmConfig>,
    /// Telegram bot configuration (seeded to settings store at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram:               Option<flatten::TelegramConfig>,
    /// WeChat iLink Bot configuration (seeded to settings store at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wechat:                 Option<flatten::WechatConfig>,
    /// Composio credentials (seeded to settings store at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composio:               Option<flatten::ComposioConfig>,
    /// Configured users with platform identity mappings (required).
    pub users:                  Vec<crate::boot::UserConfig>,
    /// Maximum ingress messages per user per minute (rate limiting).
    #[serde(default = "default_max_ingress_per_minute")]
    pub max_ingress_per_minute: u32,
    /// Mita proactive agent configuration (required).
    pub mita:                   MitaConfig,
    /// Knowledge layer configuration (seeded to settings store at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge:              Option<flatten::KnowledgeConfig>,
    /// Speech-to-Text configuration (optional).
    /// When present, `base_url` is required — startup fails if missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stt:                    Option<rara_stt::SttConfig>,
    /// Text-to-Speech configuration (optional).
    /// When present, voice replies are enabled for channels that support it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts:                    Option<rara_tts::TtsConfig>,
    /// Gateway supervisor configuration (optional — used by `rara gateway`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway:                Option<GatewayConfig>,
    /// Symphony autonomous coding agent orchestrator (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symphony:               Option<rara_symphony::SymphonyConfig>,
    /// Context folding (auto-anchor) configuration for the kernel.
    #[serde(default)]
    pub context_folding:        rara_kernel::kernel::ContextFoldingConfig,
    /// Lightpanda browser subsystem (optional).
    ///
    /// When present, rara starts a Lightpanda CDP server and registers all
    /// browser tools (`browser-navigate`, `browser-click`, etc.). When absent
    /// or when the binary is not installed, browser tools are not available and
    /// rara falls back to `http-fetch` for web access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser:                Option<rara_browser::BrowserConfig>,
}

/// Configuration for the Mita background proactive agent.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct MitaConfig {
    /// Heartbeat interval (e.g. "30m", "1800s").
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub heartbeat_interval: Duration,
}

/// Configuration for the gateway supervisor.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Upstream check interval (e.g. "5m", "300s").
    #[serde(
        default = "gateway_defaults::check_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub check_interval:       Duration,
    /// Total health confirmation timeout in seconds.
    #[serde(default = "gateway_defaults::health_timeout")]
    pub health_timeout:       u64,
    /// HTTP health poll interval (e.g. "2s").
    #[serde(
        default = "gateway_defaults::health_poll_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub health_poll_interval: Duration,
    /// Max consecutive restart failures before giving up.
    #[serde(default = "gateway_defaults::max_restart_attempts")]
    pub max_restart_attempts: u32,
    /// Whether to auto-apply upstream updates.
    #[serde(default = "gateway_defaults::auto_update")]
    pub auto_update:          bool,
    /// Bind address for the gateway admin HTTP API.
    #[serde(default = "gateway_defaults::bind_address")]
    pub bind_address:         String,
    /// Repository URL for commit links in notifications (e.g. "<https://github.com/rararulab/rara>").
    pub repo_url:             String,
    /// Telegram bot token for the gateway management bot (separate from rara's
    /// bot).
    pub bot_token:            String,
    /// Telegram chat ID for the gateway bot (typically the admin's private
    /// chat).
    pub chat_id:              i64,
}

mod gateway_defaults {
    use std::time::Duration;
    pub fn check_interval() -> Duration { Duration::from_secs(300) }
    pub fn health_timeout() -> u64 { 30 }
    pub fn health_poll_interval() -> Duration { Duration::from_secs(2) }
    pub fn max_restart_attempts() -> u32 { 3 }
    pub fn auto_update() -> bool { true }
    pub fn bind_address() -> String { "127.0.0.1:25556".to_owned() }
}

/// General OTLP telemetry configuration.
#[derive(Debug, Clone, Default, bon::Builder, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// OTLP endpoint URL (e.g. `http://alloy:4318/v1/traces`).
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
    /// Export protocol: `"http"` or `"grpc"`.
    #[serde(default)]
    pub otlp_protocol: Option<String>,
}

fn default_database_config() -> DatabaseConfig { DatabaseConfig::builder().build() }
fn default_max_ingress_per_minute() -> u32 { 30 }

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
    /// - global: [`rara_paths::config_file()`]
    /// - local override: `./config.yaml`
    ///
    /// All required fields must be present after merging; missing
    /// keys cause a deserialization error at startup.
    pub fn new() -> Result<Self, config::ConfigError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::load_from_paths(
            rara_paths::config_file().as_path(),
            &cwd.join("config.yaml"),
        )
    }

    fn load_from_paths(global_path: &Path, local_path: &Path) -> Result<Self, config::ConfigError> {
        if !(global_path.is_file() || local_path.is_file()) {
            return Err(config::ConfigError::Message(format!(
                "No config.yaml found. Looked for {} and {}",
                local_path.display(),
                global_path.display()
            )));
        }

        let cfg = config::Config::builder()
            .add_source(
                config::File::from(global_path)
                    .format(config::FileFormat::Yaml)
                    .required(false),
            )
            .add_source(
                config::File::from(local_path)
                    .format(config::FileFormat::Yaml)
                    .required(false),
            )
            .build()?;
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

    // Validate STT config: if section is present, base_url must be non-empty.
    if let Some(ref stt) = config.stt {
        snafu::ensure_whatever!(
            !stt.base_url.trim().is_empty(),
            "stt.base_url is required when stt section is configured"
        );
        info!(base_url = %stt.base_url, "STT service configured");
    }

    // If managed mode, spawn and wait for whisper-server before building STT
    // client.
    let whisper_process = if let Some(ref stt) = config.stt {
        if let Some(mut wp) = rara_stt::WhisperProcess::from_config(stt) {
            wp.start().await.whatever_context(
                "failed to start managed whisper-server (check stt.server_bin and stt.model_path)",
            )?;
            info!("managed whisper-server started");
            Some(wp)
        } else {
            None
        }
    } else {
        None
    };

    let stt_service = config.stt.as_ref().map(rara_stt::SttService::from_config);

    // Build TTS service when configured — symmetric to STT.
    if let Some(ref tts) = config.tts {
        snafu::ensure_whatever!(
            !tts.base_url.trim().is_empty(),
            "tts.base_url is required when tts section is configured"
        );
        info!(base_url = %tts.base_url, model = %tts.model, "TTS service configured");
    }
    let tts_service = config.tts.as_ref().map(rara_tts::TtsService::from_config);

    let db_store = init_infra(&config)
        .await
        .whatever_context("Failed to initialize infrastructure services")?;
    let pool = db_store.pool().clone();

    let settings_svc =
        rara_backend_admin::settings::SettingsSvc::load(db_store.kv_store(), pool.clone())
            .await
            .whatever_context("Failed to initialize runtime settings")?;

    let settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider> =
        Arc::new(settings_svc.clone());
    info!("Runtime settings service loaded");

    // Resolve config file path (same logic as AppConfig::new)
    let config_path = {
        let mut path = std::env::current_dir().unwrap_or_default();
        path.push("config.yaml");
        path
    };
    let config_file_sync =
        config_sync::ConfigFileSync::new(settings_provider.clone(), config.clone(), config_path)
            .await
            .whatever_context("Failed to initialize config file sync")?;

    // -- browser subsystem (optional) -------------------------------------
    // Start Lightpanda if a `browser:` section exists in config. Failure to
    // start is non-fatal — browser tools are simply not registered.
    let browser_manager: Option<rara_browser::BrowserManagerRef> =
        if let Some(browser_cfg) = config.browser.clone() {
            match rara_browser::BrowserManager::start(browser_cfg).await {
                Ok(manager) => {
                    info!("browser subsystem initialized with Lightpanda");
                    Some(std::sync::Arc::new(manager))
                }
                Err(e) => {
                    warn!(error = %e, "browser subsystem disabled — Lightpanda failed to start");
                    None
                }
            }
        } else {
            None
        };

    let rara = crate::boot::boot(
        pool.clone(),
        settings_provider.clone(),
        &config.users,
        browser_manager,
    )
    .await
    .whatever_context("Failed to boot kernel dependencies")?;

    let backend = rara_backend_admin::state::BackendState::init(
        rara.session_index.clone(),
        rara.tape_service.clone(),
        settings_provider.clone(),
        settings_svc.clone(),
        rara.model_lister.clone(),
    )
    .await
    .whatever_context("Failed to initialize BackendState")?;

    let web_adapter = Arc::new(
        rara_channels::web::WebAdapter::new(config.owner_token.clone())
            .with_stt_service(stt_service.clone()),
    );
    let web_router = web_adapter.router();

    let telegram_adapter = match try_build_telegram(
        &backend.settings_svc,
        rara.user_question_manager.clone(),
        stt_service,
        tts_service,
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

    let wechat_adapter = match try_build_wechat(&backend.settings_svc).await {
        Ok(Some(adapter)) => {
            info!("WeChat adapter built");
            Some(adapter)
        }
        Ok(None) => {
            info!("WeChat not configured (account_id unset in settings), skipping");
            None
        }
        Err(e) => {
            warn!(error = %e, "Failed to build WeChat adapter, skipping");
            None
        }
    };

    // Build IOSubsystem with all adapters before passing to Kernel.
    let notification_channel_id = settings_provider
        .get(rara_domain_shared::settings::keys::TELEGRAM_NOTIFICATION_CHANNEL_ID)
        .await
        .and_then(|s| s.parse::<i64>().ok());
    let mut io = rara_kernel::io::IOSubsystem::new(
        rara.identity_resolver.clone(),
        rara.session_index.clone(),
        notification_channel_id,
        config.max_ingress_per_minute,
    );
    if let Some(ref tg) = telegram_adapter {
        io.register_adapter(ChannelType::Telegram, tg.clone() as Arc<dyn ChannelAdapter>);
    }
    if let Some(ref wc) = wechat_adapter {
        io.register_adapter(ChannelType::Wechat, wc.clone() as Arc<dyn ChannelAdapter>);
    }
    io.register_adapter(
        ChannelType::Web,
        web_adapter.clone() as Arc<dyn ChannelAdapter>,
    );
    if let Some(ref cli) = options.cli_adapter {
        io.register_adapter(ChannelType::Cli, cli.clone() as Arc<dyn ChannelAdapter>);
    }

    let mcp_tool_provider: Option<rara_kernel::tool::DynamicToolProviderRef> = Some(Arc::new(
        boot::McpDynamicToolProvider::new(rara.mcp_manager.clone()),
    ));

    let kernel_config = rara_kernel::kernel::KernelConfig {
        mita_heartbeat_interval: Some(config.mita.heartbeat_interval),
        context_folding: config.context_folding.clone(),
        ..Default::default()
    };

    // Build a closure that captures the skill registry and generates the
    // skills prompt block on each agent turn.
    let skill_prompt_provider: rara_kernel::handle::SkillPromptProvider = {
        let registry = rara.skill_registry.clone();
        Arc::new(move || {
            let skills = registry.list_all();
            rara_skills::prompt_gen::generate_skills_prompt(&skills)
        })
    };

    let kernel = rara_kernel::kernel::Kernel::new(
        kernel_config,
        rara.driver_registry.clone(),
        rara.tool_registry.clone(),
        rara.agent_registry.clone(),
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
        rara.knowledge_service.clone(),
        mcp_tool_provider,
        rara_kernel::trace::TraceService::new(pool.clone()),
        skill_prompt_provider,
    );

    let cancellation_token = CancellationToken::new();

    // Supervisor restarts whisper-server on crash, stops on app shutdown.
    let _whisper_supervisor =
        whisper_process.map(|wp| wp.spawn_supervisor(cancellation_token.clone()));

    // Start bidirectional config <-> settings sync
    {
        let cancel = cancellation_token.clone();
        tokio::spawn(async move {
            config_file_sync.start(cancel).await;
        });
    }

    let (_kernel_arc, kernel_handle) = kernel.start(cancellation_token.clone());

    // Wire DispatchRaraTool and ListSessionsTool with the now-available
    // KernelHandle.
    {
        let mut lock = rara.dispatch_rara_handle.write().await;
        *lock = Some(kernel_handle.clone());
    }
    {
        let mut lock = rara.list_sessions_handle.write().await;
        *lock = Some(kernel_handle.clone());
    }

    // MCP heartbeat: reconnect dead servers periodically
    {
        let mcp_mgr = rara.mcp_manager.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            ticker.tick().await; // skip immediate first tick
            loop {
                ticker.tick().await;
                mcp_mgr.reconnect_dead().await;
            }
        });
    }

    // Symphony status handle removed — symphony is now a standalone sync bridge.

    let (domain_routes, _openapi) =
        backend.routes(&kernel_handle, &rara.skill_registry, &rara.mcp_manager);

    let dock_store_path = rara_paths::data_dir().join("dock");
    let dock_state = rara_dock::DockRouterState {
        store:         std::sync::Arc::new(rara_dock::DockSessionStore::new(dock_store_path)),
        tape_service:  Some(rara.tape_service.clone()),
        kernel_handle: Some(kernel_handle.clone()),
        mutation_sink: rara.dock_mutation_sink.clone(),
        in_flight:     std::sync::Arc::new(parking_lot::Mutex::new(
            std::collections::HashSet::new(),
        )),
    };
    let dock_routes = rara_dock::dock_router(dock_state);

    let routes_fn: Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync> =
        Box::new(move |router| {
            health_routes(router)
                .merge(domain_routes.clone())
                .merge(dock_routes.clone())
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

    // Signal readiness to the gateway supervisor (if present).
    // The gateway watches our stdout for this marker.
    // tracing goes to stderr, so this does not interfere.
    println!("READY");

    // Build a shared service client used by both command and callback handlers.
    let bot_client: std::sync::Arc<dyn rara_channels::telegram::commands::BotServiceClient> = {
        use rara_channels::telegram::commands::KernelBotServiceClient;
        std::sync::Arc::new(KernelBotServiceClient::new(
            rara.session_index.clone(),
            rara.tape_service.clone(),
            kernel_handle.clone(),
            rara.mcp_manager.clone(),
        ))
    };

    // Build command handlers shared across all channels.
    let command_handlers: Vec<std::sync::Arc<dyn rara_kernel::channel::command::CommandHandler>> = {
        use rara_channels::telegram::commands::{
            BasicCommandHandler, DebugCommandHandler, McpCommandHandler, SessionCommandHandler,
            StatusCommandHandler, StopCommandHandler, TapeCommandHandler,
        };
        let session_handler = std::sync::Arc::new(SessionCommandHandler::new(bot_client.clone()));
        let stop_handler = std::sync::Arc::new(StopCommandHandler::new(
            bot_client.clone(),
            kernel_handle.clone(),
        ));
        let status_handler = std::sync::Arc::new(StatusCommandHandler::new(
            bot_client.clone(),
            kernel_handle.clone(),
        ));
        let tape_handler = std::sync::Arc::new(TapeCommandHandler::new(bot_client.clone()));
        let debug_handler =
            std::sync::Arc::new(DebugCommandHandler::new(rara.tape_service.clone()));
        // Collect all command definitions so /help can list them.
        use rara_kernel::channel::command::CommandHandler as _;
        let all_commands: Vec<rara_kernel::channel::command::CommandDefinition> = [
            session_handler.commands(),
            stop_handler.commands(),
            status_handler.commands(),
            tape_handler.commands(),
            debug_handler.commands(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let basic_handler = std::sync::Arc::new(BasicCommandHandler::new(all_commands));
        let mcp_handler = std::sync::Arc::new(McpCommandHandler::new(bot_client.clone()));
        vec![
            basic_handler,
            session_handler,
            stop_handler,
            status_handler,
            tape_handler,
            debug_handler,
            mcp_handler,
        ]
    };

    if let Some(ref tg_adapter) = telegram_adapter {
        tg_adapter.set_command_handlers(command_handlers.clone());

        // Register callback handlers for inline keyboard interactions.
        {
            use rara_channels::telegram::commands::{
                SessionDeleteCallbackHandler, SessionDeleteCancelHandler,
                SessionDeleteConfirmHandler, SessionDetailCallbackHandler,
                SessionSwitchCallbackHandler, StatusJobsCallbackHandler,
            };
            let callback_handlers: Vec<
                std::sync::Arc<dyn rara_kernel::channel::command::CallbackHandler>,
            > = vec![
                std::sync::Arc::new(SessionSwitchCallbackHandler::new(bot_client.clone())),
                std::sync::Arc::new(SessionDetailCallbackHandler::new(bot_client.clone())),
                std::sync::Arc::new(SessionDeleteCallbackHandler::new(bot_client.clone())),
                std::sync::Arc::new(SessionDeleteConfirmHandler::new(bot_client.clone())),
                std::sync::Arc::new(SessionDeleteCancelHandler::new()),
                std::sync::Arc::new(StatusJobsCallbackHandler::new(kernel_handle.clone())),
            ];
            tg_adapter.set_callback_handlers(callback_handlers);
        }

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
    if let Some(ref wc) = wechat_adapter {
        use rara_kernel::channel::adapter::ChannelAdapter as _;
        match wc.start(kernel_handle.clone()).await {
            Ok(()) => info!("WeChat adapter started"),
            Err(e) => warn!(error = %e, "Failed to start WeChat adapter"),
        }
    }
    info!("Kernel I/O subsystem running");

    // -- Symphony sync bridge -------------------------------------------------
    if let Some(ref symphony_config) = config.symphony {
        if symphony_config.enabled {
            let symphony = rara_symphony::SymphonyService::new(
                symphony_config.clone(),
                cancellation_token.clone(),
                std::env::var("GITHUB_TOKEN").ok(),
            );
            tokio::spawn(async move {
                if let Err(e) = symphony.run().await {
                    error!(error = %e, "symphony service failed");
                }
            });
            info!("Symphony service started");
        }
    }

    // Start web frontend dev server (bun run dev) if web/ exists.
    if let Some(web_port) = config.http.web_port {
        let web_dir = PathBuf::from("web");
        let web_cancel = cancellation_token.clone();
        tokio::spawn(async move {
            web_server::start_web_server(web_dir, web_port, web_cancel).await;
        });
    }

    info!("Application started successfully");

    let app_handle = AppHandle {
        shutdown_tx: Some(shutdown_tx),
        running: Arc::clone(&running),
        cancellation_token: cancellation_token.clone(),
        kernel_handle: Some(kernel_handle),
        command_handlers,
        user_question_manager: Some(rara.user_question_manager.clone()),
    };

    let running_clone = Arc::clone(&running);
    let ct_clone = cancellation_token.clone();

    tokio::spawn(async move {
        shutdown_signal(shutdown_rx).await;
        running_clone.store(false, Ordering::SeqCst);
        ct_clone.cancel();

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
    user_question_manager: rara_kernel::user_question::UserQuestionManagerRef,
    stt_service: Option<rara_stt::SttService>,
    tts_service: Option<rara_tts::TtsService>,
) -> Result<Option<Arc<rara_channels::telegram::TelegramAdapter>>, Whatever> {
    use rara_domain_shared::settings::{SettingsProvider, keys};

    fn parse_group_policy(raw: Option<String>) -> GroupPolicy {
        raw.and_then(|s| {
            s.trim()
                .parse::<GroupPolicy>()
                .map_err(|e| warn!(error = %e, "invalid telegram.group_policy, using default"))
                .ok()
        })
        .unwrap_or_default()
    }

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
    let group_policy = parse_group_policy(settings.get(keys::TELEGRAM_GROUP_POLICY).await);

    let mut tg_config = rara_channels::telegram::TelegramConfig::default();
    tg_config.primary_chat_id = chat_id;
    tg_config.allowed_group_chat_id = group_id;
    tg_config.group_policy = group_policy;

    let adapter = Arc::new(
        rara_channels::telegram::TelegramAdapter::with_proxy(&token, vec![], proxy.as_deref())
            .whatever_context("failed to build telegram adapter")?
            .with_config(tg_config)
            .with_user_question_manager(user_question_manager)
            .with_stt_service(stt_service)
            .with_tts_service(tts_service),
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
            let new_group_policy =
                parse_group_policy(settings.get(keys::TELEGRAM_GROUP_POLICY).await);
            let mut cfg = config_handle.write().unwrap_or_else(|e| e.into_inner());
            cfg.primary_chat_id = new_chat_id;
            cfg.allowed_group_chat_id = new_group_id;
            cfg.group_policy = new_group_policy;
        }
    });

    Ok(Some(adapter))
}

async fn try_build_wechat(
    settings_svc: &rara_backend_admin::settings::SettingsSvc,
) -> Result<Option<Arc<rara_channels::wechat::WechatAdapter>>, Whatever> {
    use rara_channels::wechat::storage;
    use rara_domain_shared::settings::{SettingsProvider, keys};

    let settings: Arc<dyn SettingsProvider> = Arc::new(settings_svc.clone());

    // Prefer filesystem credentials (written by login) over settings store.
    let account_id = match storage::get_account_ids() {
        Ok(ids) if !ids.is_empty() => {
            info!(
                account_id = %ids[0],
                "wechat account_id resolved from saved credentials"
            );
            ids.into_iter().next().expect("non-empty")
        }
        _ => match settings.get(keys::WECHAT_ACCOUNT_ID).await {
            Some(id) if !id.is_empty() => id,
            _ => return Ok(None),
        },
    };

    // Read base_url from the persisted AccountData first (login writes it there),
    // then fall back to settings, then to the default.
    let fs_base_url = storage::get_account_data(&account_id)
        .ok()
        .map(|d| d.base_url)
        .filter(|u| !u.is_empty());
    let base_url = match fs_base_url {
        Some(url) => url,
        None => settings
            .get(keys::WECHAT_BASE_URL)
            .await
            .unwrap_or_else(|| storage::DEFAULT_BASE_URL.to_string()),
    };

    let adapter = Arc::new(
        rara_channels::wechat::WechatAdapter::new(account_id, base_url)
            .whatever_context("failed to build wechat adapter")?,
    );

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
    shutdown_tx:               Option<oneshot::Sender<()>>,
    running:                   Arc<AtomicBool>,
    cancellation_token:        CancellationToken,
    /// Kernel handle (for injecting inbound messages, accessing stream hub,
    /// endpoint registry, etc.).
    pub kernel_handle:         Option<rara_kernel::handle::KernelHandle>,
    /// Command handlers shared across all channels (Telegram, CLI, etc.).
    pub command_handlers: Vec<std::sync::Arc<dyn rara_kernel::channel::command::CommandHandler>>,
    /// User question manager for the ask-user tool (CLI needs it to subscribe
    /// and resolve agent questions).
    pub user_question_manager: Option<rara_kernel::user_question::UserQuestionManagerRef>,
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
    use std::fs;

    use super::AppConfig;

    const BASE_YAML: &str = r#"
http:
  bind_address: "127.0.0.1:25555"
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
users:
  - name: test
    role: root
    platforms: []
mita:
  heartbeat_interval: "30m"
"#;

    #[test]
    fn app_config_loads_from_global_fallback_when_local_is_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let global = tmp.path().join("global-config.yaml");
        let local = tmp.path().join("config.yaml");
        fs::write(&global, BASE_YAML).expect("write global config");

        let config = AppConfig::load_from_paths(&global, &local).expect("load config");
        assert_eq!(config.http.bind_address, "127.0.0.1:25555");
    }

    #[test]
    fn app_config_prefers_local_override_over_global() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let global = tmp.path().join("global-config.yaml");
        let local = tmp.path().join("config.yaml");
        fs::write(&global, BASE_YAML).expect("write global config");
        fs::write(
            &local,
            BASE_YAML.replace("127.0.0.1:25555", "127.0.0.1:35555"),
        )
        .expect("write local config");

        let config = AppConfig::load_from_paths(&global, &local).expect("load config");
        assert_eq!(config.http.bind_address, "127.0.0.1:35555");
    }
}
