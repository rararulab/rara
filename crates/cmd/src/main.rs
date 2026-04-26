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
}

#[derive(Debug, Clone, Args)]
#[command(flatten_help = true)]
#[command(about = "Start the job server")]
#[command(long_about = "Start the job server with all services.\n\nExamples:\n  job server")]
struct ServerArgs {}

impl ServerArgs {
    async fn run() -> Result<(), Whatever> {
        // Load config first (Consul KV or env vars) so observability
        // settings are available before initialising the tracing subscriber.
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

        let logging_opts = if let Some(otlp) = langfuse_otlp {
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

        let _guards = common_telemetry::logging::init_global_logging(
            "rara",
            &logging_opts,
            &common_telemetry::logging::TracingOptions::default(),
            None,
        );

        run_app(config).await
    }
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

#[tokio::main]
async fn main() -> Result<(), Whatever> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let cli = Cli::parse();
    match cli.commands {
        Commands::Server(_) => ServerArgs::run().await,
        Commands::Chat(args) => args.run().await,
        Commands::Top(args) => args.run().await,
        Commands::Gateway(_) => GatewayArgs::run().await,
        Commands::Login(cmd) => cmd.run().await,
        Commands::Setup(cmd) => cmd.run().await,
        Commands::Wechat(cmd) => cmd.run().await,
        Commands::Debug(cmd) => cmd.run().await,
    }
}
