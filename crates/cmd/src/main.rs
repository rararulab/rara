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

use std::{collections::HashMap, sync::Arc, time::Duration};

use clap::{Args, Parser, Subcommand};
use snafu::{ResultExt, Whatever, whatever};

mod build_info;
mod top;

use rara_app::{AppConfig, StartOptions};
use rara_channels::terminal::{CliEvent, TerminalAdapter};
use rara_kernel::{
    channel::types::{ChannelType, MessageContent},
    io::{
        egress::{Endpoint, EndpointAddress},
        ingress::RawPlatformMessage,
        stream::StreamEvent,
        types::{InteractionType, ReplyContext as IoReplyContext},
    },
    process::{SessionId, principal::UserId},
};

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

        let logging_opts = if let Some(ref endpoint) = config
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
                enable_otlp_tracing: true,
                otlp_endpoint: Some(endpoint.to_string()),
                otlp_export_protocol: protocol,
                ..Default::default()
            }
        } else if std::env::var("KUBERNETES_SERVICE_HOST").is_ok() {
            // Running in Kubernetes — auto-connect to Alloy OTLP collector.
            use common_telemetry::logging::{LoggingOptions, OtlpExportProtocol};
            tracing::info!("Kubernetes detected — auto-enabling OTLP tracing to Alloy");
            LoggingOptions {
                enable_otlp_tracing: true,
                otlp_endpoint: Some("http://rara-infra-alloy:4318/v1/traces".to_string()),
                otlp_export_protocol: Some(OtlpExportProtocol::Http),
                log_format: common_telemetry::logging::LogFormat::Json,
                ..Default::default()
            }
        } else {
            common_telemetry::logging::LoggingOptions::default()
        };

        let _guards = common_telemetry::logging::init_global_logging(
            "rara",
            &logging_opts,
            &common_telemetry::logging::TracingOptions::default(),
            None,
        );

        config.run().await
    }
}

// ---------------------------------------------------------------------------
// Chat command
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Args)]
#[command(about = "Start interactive chat with an agent")]
struct ChatArgs {
    /// Session key for conversation continuity.
    #[arg(long, default_value = "default")]
    session: String,

    /// User identifier.
    #[arg(long, default_value = "local")]
    user_id: String,
}

impl ChatArgs {
    async fn run(self) -> Result<(), Whatever> {
        let config = AppConfig::new().whatever_context("Failed to load config")?;

        // Minimal telemetry for CLI mode (no OTLP, console only).
        let _guards = common_telemetry::logging::init_global_logging(
            "rara-cli",
            &common_telemetry::logging::LoggingOptions::default(),
            &common_telemetry::logging::TracingOptions::default(),
            None,
        );

        // Create the terminal adapter.
        let (adapter, event_rx) = TerminalAdapter::new();
        let adapter = Arc::new(adapter);

        // Start the app with the CLI adapter injected.
        let mut app_handle = config
            .start_with_options(StartOptions {
                cli_adapter: Some(adapter.clone()),
            })
            .await
            .whatever_context("Failed to start application")?;

        let kernel_handle = match app_handle.kernel_handle.take() {
            Some(h) => h,
            None => whatever!("kernel handle not available"),
        };
        let endpoint_registry = kernel_handle.endpoint_registry().clone();
        let stream_hub = kernel_handle.stream_hub().clone();

        // Compute the session ID and user ID the same way AppSessionResolver
        // and AppIdentityResolver would.
        let session_key = self.session.clone();
        let user_id_str = self.user_id.clone();
        let resolved_user_id = UserId(format!("cli:{}", user_id_str));
        let resolved_session_id = SessionId::new();

        // Register CLI endpoint in the EndpointRegistry.
        let cli_endpoint = Endpoint {
            channel_type: ChannelType::Cli,
            address:      EndpointAddress::Cli {
                session_id: session_key.clone(),
            },
        };
        endpoint_registry.register(&resolved_user_id, cli_endpoint);

        // Spawn StreamHub forwarder task.
        let forwarder_event_tx = adapter.clone();
        let forwarder_session_id = resolved_session_id.clone();
        let forwarder_hub = stream_hub.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            let mut active_streams: std::collections::HashSet<rara_kernel::io::stream::StreamId> =
                std::collections::HashSet::new();
            loop {
                interval.tick().await;
                let subs = forwarder_hub.subscribe_session(&forwarder_session_id);
                for (stream_id, mut rx) in subs {
                    if active_streams.contains(&stream_id) {
                        // Already have a forwarder for this stream, just drain.
                        while let Ok(event) = rx.try_recv() {
                            let cli_event = stream_event_to_cli_event(event);
                            let _ = forwarder_event_tx.send_cli_event(cli_event);
                        }
                    } else {
                        // New stream — spawn a dedicated forwarder task.
                        active_streams.insert(stream_id.clone());
                        let tx = forwarder_event_tx.clone();
                        tokio::spawn(async move {
                            while let Ok(event) = rx.recv().await {
                                let cli_event = stream_event_to_cli_event(event);
                                let _ = tx.send_cli_event(cli_event);
                            }
                            // Stream closed — send Done.
                            let _ = tx.send_cli_event(CliEvent::Done);
                        });
                    }
                }
            }
        });

        // Print welcome banner.
        eprintln!("=== Job Interactive Chat ===");
        eprintln!("Session: {}  User: {}", session_key, user_id_str);
        eprintln!("Type your message and press Enter. Ctrl+C to exit.\n");

        // Run the REPL loop.
        run_repl(event_rx, kernel_handle, session_key, user_id_str).await;

        // Cleanup.
        app_handle.shutdown();
        // Give a moment for graceful shutdown.
        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(())
    }
}

/// Convert a [`StreamEvent`] into a [`CliEvent`].
fn stream_event_to_cli_event(event: StreamEvent) -> CliEvent {
    match event {
        StreamEvent::TextDelta { text: t } => CliEvent::TextDelta { text: t },
        StreamEvent::ReasoningDelta { text: t } => CliEvent::ReasoningDelta { text: t },
        StreamEvent::ToolCallStart { name, .. } => CliEvent::ToolCallStart { name },
        StreamEvent::ToolCallEnd { error, .. } => {
            if let Some(ref err) = error {
                eprintln!("\x1b[31m[tool error] {}\x1b[0m", err);
            }
            CliEvent::ToolCallEnd
        }
        StreamEvent::Progress { stage } => CliEvent::Progress { text: stage },
        StreamEvent::TurnMetrics {
            duration_ms,
            iterations,
            tool_calls,
            model,
        } => CliEvent::Progress {
            text: format!(
                "[{model}] {iterations} iterations, {tool_calls} tool calls, {duration_ms}ms"
            ),
        },
    }
}

/// Build a [`RawPlatformMessage`] for the CLI channel.
fn build_cli_raw_message(session_key: &str, user_id: &str, content: &str) -> RawPlatformMessage {
    RawPlatformMessage {
        channel_type:        ChannelType::Cli,
        platform_message_id: Some(ulid::Ulid::new().to_string()),
        platform_user_id:    user_id.to_owned(),
        platform_chat_id:    Some(session_key.to_owned()),
        content:             MessageContent::Text(content.to_owned()),
        reply_context:       Some(IoReplyContext {
            thread_id:                None,
            reply_to_platform_msg_id: None,
            interaction_type:         InteractionType::Message,
        }),
        metadata:            HashMap::new(),
    }
}

/// Run the interactive REPL loop.
async fn run_repl(
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<CliEvent>,
    kernel_handle: rara_kernel::handle::kernel_handle::KernelHandle,
    session_key: String,
    user_id: String,
) {
    use std::io::Write;

    // Spawn a blocking stdin reader thread.
    let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<String>(16);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let trimmed = line.trim().to_owned();
                    if !trimmed.is_empty() {
                        if stdin_tx.blocking_send(trimmed).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut in_stream = false;

    loop {
        // Print prompt when not in a streaming response.
        if !in_stream {
            eprint!("> ");
            let _ = std::io::stderr().flush();
        }

        tokio::select! {
            line = stdin_rx.recv() => {
                match line {
                    Some(text) => {
                        let raw = build_cli_raw_message(&session_key, &user_id, &text);
                        if let Err(e) = kernel_handle.ingest(raw).await {
                            eprintln!("[error] Failed to send message: {}", e);
                        } else {
                            in_stream = true;
                        }
                    }
                    None => {
                        // stdin closed
                        break;
                    }
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(CliEvent::Reply { content }) => {
                        if in_stream {
                            // End the streaming line.
                            println!();
                        }
                        println!("{}", content);
                        in_stream = false;
                    }
                    Some(CliEvent::TextDelta { text }) => {
                        print!("{}", text);
                        let _ = std::io::stdout().flush();
                    }
                    Some(CliEvent::ReasoningDelta { text }) => {
                        // Show reasoning in dimmed text.
                        eprint!("\x1b[2m{}\x1b[0m", text);
                        let _ = std::io::stderr().flush();
                    }
                    Some(CliEvent::ToolCallStart { name }) => {
                        eprintln!("\x1b[33m[tool] {}\x1b[0m", name);
                    }
                    Some(CliEvent::ToolCallEnd) => {
                        eprintln!("\x1b[33m[tool] done\x1b[0m");
                    }
                    Some(CliEvent::Progress { text }) => {
                        if !text.is_empty() {
                            eprintln!("\x1b[36m[{}]\x1b[0m", text);
                        }
                    }
                    Some(CliEvent::Error { message }) => {
                        eprintln!("\x1b[31m[error] {}\x1b[0m", message);
                        in_stream = false;
                    }
                    Some(CliEvent::Done) => {
                        if in_stream {
                            println!();
                        }
                        in_stream = false;
                    }
                    None => {
                        // Event channel closed.
                        break;
                    }
                }
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
    }
}
