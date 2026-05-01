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

use clap::{Args, Parser, Subcommand};
use snafu::{ResultExt, Whatever, whatever};

mod build_info;
mod chat;
mod debug;
mod login;
mod session_index;
mod setup;
mod top;
mod wechat;

use chat::ChatArgs;
use rara_app::{AppConfig, run as run_app};

#[derive(Debug, Parser)]
#[clap(
    name = "job",
    about = "raracli",
    author = build_info::AUTHOR,
    version = build_info::FULL_VERSION
)]
struct Cli {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Server(ServerArgs),
    Chat(ChatArgs),
    Top(top::TopCmd),
    Gateway(GatewayArgs),
    Login(login::LoginCmd),
    Setup(setup::SetupCmd),
    Wechat(wechat::WechatCmd),
    Debug(debug::DebugCmd),
    /// Maintenance commands for the SQLite-backed session index
    /// (issue #2025). Today only `rebuild` is implemented; future
    /// subcommands (e.g. `dump`, `verify`) will land here.
    SessionIndex(session_index::SessionIndexCmd),
}

#[derive(Debug, Clone, Args)]
#[command(flatten_help = true)]
#[command(about = "Start the job server")]
#[command(long_about = "Start the job server with all services.\n\nExamples:\n  job server")]
struct ServerArgs {}

/// Sync prelude for the `server` command.
///
/// Why this is a separate function (and not part of an `async fn run`):
/// `init_global_logging` constructs the OTLP HTTP exporter, which uses
/// `reqwest::blocking::Client::builder().build()`. That builder spawns
/// and immediately drops a temporary tokio Runtime on the calling thread.
/// Dropping a Runtime while another Runtime is current panics in tokio
/// 1.x ("Cannot drop a runtime in a context where blocking is not
/// allowed"). So all logging / OTLP / Pyroscope construction must happen
/// **before** the outer tokio Runtime exists.
///
/// Returns the loaded config plus the guards that must outlive the
/// process (file appender flush guards + Pyroscope shutdown guard).
type ServerInitGuards = (
    AppConfig,
    Vec<common_telemetry::logging::LoggingWorkerGuard>,
    Option<common_telemetry::profiling::ProfilingGuard>,
);

fn init_server_sync() -> Result<ServerInitGuards, Whatever> {
    let config = AppConfig::new().whatever_context("Failed to load config")?;

    let logs_dir = rara_paths::logs_dir();
    std::fs::create_dir_all(logs_dir).expect("failed to create logs directory");
    let logs_dir_str = logs_dir.to_string_lossy().into_owned();

    // Pinned OpenTelemetry semantic-convention schema URL for the OTLP
    // traces exporter. Pinning the version lets backends (Langfuse,
    // Tempo, etc.) interpret span attributes against a known semconv
    // release rather than a moving target.
    const OTEL_SCHEMA_URL: &str = "https://opentelemetry.io/schemas/1.40.0";

    let langfuse_otlp = config
        .telemetry
        .otlp
        .as_ref()
        .filter(|o| o.enabled.unwrap_or(false));

    let mut logging_opts = if let Some(otlp) = langfuse_otlp {
        use common_telemetry::logging::{LoggingOptions, OtlpExportProtocol};
        let Some(endpoint) = otlp.traces_endpoint.clone() else {
            whatever!("telemetry.otlp.enabled = true requires telemetry.otlp.traces_endpoint");
        };
        LoggingOptions {
            dir: logs_dir_str,
            enable_otlp_tracing: true,
            otlp_endpoint: Some(endpoint),
            otlp_export_protocol: Some(OtlpExportProtocol::Http),
            otlp_headers: otlp.headers.clone(),
            otlp_schema_url: Some(OTEL_SCHEMA_URL.to_string()),
            otlp_deployment_environment: otlp.deployment_environment.clone(),
            ..Default::default()
        }
    } else if let Some(ref endpoint) = config
        .telemetry
        .otlp_endpoint
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        use common_telemetry::logging::{LoggingOptions, OtlpExportProtocol};
        let protocol = config.telemetry.otlp_protocol.as_deref().map(|p| match p {
            "grpc" => OtlpExportProtocol::Grpc,
            _ => OtlpExportProtocol::Http,
        });
        LoggingOptions {
            dir: logs_dir_str,
            enable_otlp_tracing: true,
            otlp_endpoint: Some(endpoint.to_string()),
            otlp_export_protocol: protocol,
            otlp_schema_url: Some(OTEL_SCHEMA_URL.to_string()),
            ..Default::default()
        }
    } else if std::env::var("KUBERNETES_SERVICE_HOST").is_ok() {
        // Running in Kubernetes — auto-connect to Alloy OTLP collector.
        use common_telemetry::logging::{LoggingOptions, OtlpExportProtocol};
        tracing::info!("Kubernetes detected — auto-enabling OTLP tracing to Alloy");
        LoggingOptions {
            dir: logs_dir_str,
            enable_otlp_tracing: true,
            otlp_endpoint: Some("http://rara-infra-alloy:4318/v1/traces".to_string()),
            otlp_export_protocol: Some(OtlpExportProtocol::Http),
            otlp_schema_url: Some(OTEL_SCHEMA_URL.to_string()),
            log_format: common_telemetry::logging::LogFormat::Json,
            ..Default::default()
        }
    } else {
        common_telemetry::logging::LoggingOptions {
            dir: logs_dir_str,
            ..Default::default()
        }
    };

    // Overlay OTLP logs config — independent of trace export so users
    // can ship logs to Loki without also wiring traces. Reads from the
    // same `telemetry.otlp` section because logs and traces share the
    // deployment-environment label and the OTLP family.
    if let Some(otlp) = config.telemetry.otlp.as_ref()
        && otlp.logs_enabled.unwrap_or(false)
    {
        let Some(logs_endpoint) = otlp.logs_endpoint.clone() else {
            whatever!("telemetry.otlp.logs_enabled = true requires telemetry.otlp.logs_endpoint");
        };
        logging_opts.enable_otlp_logs = true;
        logging_opts.otlp_logs_endpoint = Some(logs_endpoint);
        logging_opts.otlp_logs_headers = otlp.logs_headers.clone();
    }

    let guards = common_telemetry::logging::init_global_logging(
        "rara",
        &logging_opts,
        &common_telemetry::logging::TracingOptions::default(),
        None,
    );

    // Continuous profiling (Pyroscope). Held for the lifetime of the
    // server process — its `Drop` performs graceful shutdown so the
    // last batch flushes when the process receives SIGTERM/SIGINT and
    // `run_app` returns cleanly via its existing signal handler.
    let profiling_guard = init_profiling(&config)?;

    Ok((config, guards, profiling_guard))
}

/// Wire the Pyroscope profiling agent if enabled in YAML config.
///
/// `build_commit` comes from `shadow-rs` (`build::SHORT_COMMIT`), captured
/// at compile time so the running process can be cross-referenced with the
/// source tree without relying on runtime git access.
fn init_profiling(
    config: &AppConfig,
) -> Result<Option<common_telemetry::profiling::ProfilingGuard>, Whatever> {
    let Some(pyro_cfg) = config.telemetry.pyroscope.as_ref() else {
        return Ok(None);
    };
    let env = config.telemetry.env.as_deref().unwrap_or("unknown");
    let host_buf = hostname_or_unknown();
    let host = host_buf.as_str();
    let build_commit = if build_info::build::SHORT_COMMIT.is_empty() {
        "unknown"
    } else {
        build_info::build::SHORT_COMMIT
    };
    common_telemetry::profiling::init_pyroscope(pyro_cfg, env, host, build_commit)
        .whatever_context("Failed to initialise Pyroscope profiling")
}

/// Best-effort hostname lookup for low-cardinality profiling tags.
///
/// Falls back to the `HOSTNAME` env var (set by most shells) and then
/// `"unknown"` so a missing hostname never blocks profiling startup.
fn hostname_or_unknown() -> String {
    sysinfo::System::host_name()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "unknown".to_owned())
}

// ---------------------------------------------------------------------------
// Gateway command
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Args)]
#[command(about = "Start the gateway supervisor")]
#[command(
    long_about = "Start the gateway supervisor that spawns, monitors, and restarts the agent \
                  server.\n\nExamples:\n  rara gateway"
)]
struct GatewayArgs {}

impl GatewayArgs {
    async fn run() -> Result<(), Whatever> {
        let config = AppConfig::new().whatever_context("Failed to load config")?;

        let logs_dir = rara_paths::logs_dir();
        std::fs::create_dir_all(logs_dir).whatever_context("Failed to create logs directory")?;

        let _guards = common_telemetry::logging::init_global_logging(
            "rara-gateway",
            &common_telemetry::logging::LoggingOptions {
                dir: logs_dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            &common_telemetry::logging::TracingOptions::default(),
            None,
        );

        let Some(gateway_config) = config.gateway.clone() else {
            whatever!("Gateway requires [gateway] config section");
        };

        // Extract port from HTTP bind_address (e.g. "127.0.0.1:25555" -> "25555").
        let bind_addr = &config.http.bind_address;
        let port = bind_addr.rsplit(':').next().unwrap_or("25555");
        tracing::info!(
            health_timeout = gateway_config.health_timeout,
            max_restart_attempts = gateway_config.max_restart_attempts,
            health_port = port,
            admin_bind = %gateway_config.bind_address,
            "Starting gateway supervisor"
        );
        tracing::info!(path = %logs_dir.display(), "Agent logs directory");
        tracing::info!("View agent logs: lnav {}", logs_dir.display());

        let cancel = tokio_util::sync::CancellationToken::new();

        // 0. Build shared Telegram bot with proxy support, then create notifier.
        let proxy = std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .or_else(|_| std::env::var("ALL_PROXY"))
            .ok()
            .filter(|v| !v.is_empty());
        if let Some(ref p) = proxy {
            tracing::info!(proxy = %p, "gateway: using proxy for Telegram");
        }
        let bot = rara_channels::telegram::build_bot(&gateway_config.bot_token, proxy.as_deref())
            .whatever_context("Failed to build gateway Telegram bot")?;

        let chat_id = gateway_config.chat_id;
        let notifier = std::sync::Arc::new(rara_app::gateway::UpdateNotifier::new(
            bot.clone(),
            chat_id,
            build_info::FULL_VERSION,
            &gateway_config.repo_url,
        ));

        // 1. Create supervisor + handle.
        let (mut supervisor, supervisor_handle) = rara_app::gateway::SupervisorService::new(
            gateway_config.clone(),
            port,
            std::sync::Arc::clone(&notifier),
        );

        // 1.5 Create process monitor.
        let process_snapshot: rara_app::gateway::SnapshotHandle = Default::default();
        let alert_thresholds: rara_app::gateway::ThresholdsHandle = Default::default();

        // Load persisted thresholds from gateway-state.yaml.
        {
            let state = rara_app::gateway::load_gateway_state();
            *alert_thresholds.write().await = state.alert_thresholds;
        }

        {
            let snapshot = process_snapshot.clone();
            let thresholds = alert_thresholds.clone();
            let sup_handle = supervisor_handle.clone();
            let notifier = std::sync::Arc::clone(&notifier);
            let poll_interval = gateway_config.health_poll_interval;
            let cancel = cancel.clone();

            tokio::spawn(async move {
                let mut monitor = rara_app::gateway::ProcessMonitor::new(snapshot, thresholds);
                let mut ticker = tokio::time::interval(poll_interval);
                loop {
                    tokio::select! {
                        _ = ticker.tick() => {
                            let pid = sup_handle.status().pid;
                            let alerts = monitor.tick(pid).await;
                            for alert in alerts {
                                notifier.resource_alert(&alert).await;
                            }
                        }
                        () = cancel.cancelled() => break,
                    }
                }
            });
        }

        // 2. Create update detector + watch receiver.
        let (detector, update_rx) =
            rara_app::gateway::UpdateDetector::new(gateway_config.clone()).await;
        let detector_cancel = cancel.clone();
        tokio::spawn(async move {
            detector.run(detector_cancel).await;
        });

        // 3. Spawn update pipeline (detector → executor → supervisor restart).
        let pipeline_rx = update_rx.clone();
        let pipeline_cancel = cancel.clone();
        let pipeline_handle = supervisor_handle.clone();
        let pipeline_config = gateway_config.clone();
        let pipeline_notifier = std::sync::Arc::clone(&notifier);
        tokio::spawn(async move {
            rara_app::gateway::run_update_pipeline(
                pipeline_config,
                pipeline_rx,
                pipeline_handle,
                pipeline_cancel,
                pipeline_notifier,
            )
            .await;
        });

        // 4. Build admin HTTP server state and spawn it.
        let admin_state = rara_app::gateway::server::GatewayAppState {
            supervisor_handle: supervisor_handle.clone(),
            update_state_rx:   update_rx.clone(),
            shutdown:          cancel.clone(),
            process_snapshot:  process_snapshot.clone(),
            alert_thresholds:  alert_thresholds.clone(),
        };
        let admin_bind = gateway_config.bind_address.clone();
        let _admin_handle = rara_app::gateway::server::serve(&admin_bind, admin_state)
            .await
            .whatever_context("Failed to start gateway admin HTTP server")?;

        // 4.5 Spawn Telegram command listener for management commands.
        let health_url = format!("http://127.0.0.1:{port}/api/health");
        let listener = rara_app::gateway::GatewayTelegramListener::new(
            bot,
            chat_id,
            supervisor_handle,
            update_rx,
            health_url,
            process_snapshot,
            alert_thresholds,
        );
        let listener_cancel = cancel.clone();
        tokio::spawn(async move {
            listener.run(listener_cancel).await;
        });

        // 5. Run supervisor (blocking).
        match supervisor.run().await {
            Ok(()) => {
                notifier.gateway_shutdown("Clean shutdown requested").await;
                cancel.cancel();
                tracing::info!("Gateway supervisor exited cleanly");
                Ok(())
            }
            Err(e) => {
                tracing::error!(error = %e, "Gateway supervisor stopped with error");
                // Gateway stays alive for manual intervention — don't propagate
                // the error as a hard failure.
                tracing::info!(
                    "Gateway will remain alive for manual intervention. Press Ctrl+C to exit."
                );
                tokio::signal::ctrl_c().await.ok();
                notifier
                    .gateway_shutdown(&format!("Supervisor error: {e}"))
                    .await;
                cancel.cancel();
                Ok(())
            }
        }
    }
}

/// Manually construct the multi-threaded tokio runtime instead of using
/// `#[tokio::main]`.
///
/// Why: the `server` command's logging init builds an OTLP HTTP exporter
/// via `reqwest::blocking::Client::builder().build()`, which internally
/// constructs and immediately drops a temporary tokio Runtime on the
/// calling thread. Dropping a Runtime while another Runtime is current
/// panics in tokio 1.x ("Cannot drop a runtime in a context where
/// blocking is not allowed"). So we must perform sync logging /
/// telemetry construction **before** entering the outer runtime, then
/// `block_on` the async work afterwards.
fn main() -> Result<(), Whatever> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let cli = Cli::parse();

    // Perform any sync, pre-runtime initialization for the chosen
    // subcommand, then build the runtime and dispatch the async body.
    match cli.commands {
        Commands::Server(_) => {
            // Sync logging + Pyroscope init MUST run before the runtime
            // exists — see module-level comment above.
            let (config, _guards, _profiling_guard) = init_server_sync()?;
            build_runtime()?.block_on(run_app(config))
        }
        Commands::Chat(args) => build_runtime()?.block_on(args.run()),
        Commands::Top(args) => build_runtime()?.block_on(args.run()),
        Commands::Gateway(_) => build_runtime()?.block_on(GatewayArgs::run()),
        Commands::Login(cmd) => build_runtime()?.block_on(cmd.run()),
        Commands::Setup(cmd) => build_runtime()?.block_on(cmd.run()),
        Commands::Wechat(cmd) => build_runtime()?.block_on(cmd.run()),
        Commands::Debug(cmd) => build_runtime()?.block_on(cmd.run()),
        Commands::SessionIndex(cmd) => build_runtime()?.block_on(cmd.run()),
    }
}

/// Build the multi-thread tokio runtime that `#[tokio::main]` would have
/// produced. Kept identical to the implicit defaults so behavior outside
/// of "what runs first" is unchanged.
fn build_runtime() -> Result<tokio::runtime::Runtime, Whatever> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .whatever_context("failed to build tokio runtime")
}
