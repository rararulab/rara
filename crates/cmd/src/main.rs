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

use std::{collections::HashMap, io::Write, sync::Arc, time::Duration};

use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use crossterm::{
    cursor::MoveToColumn,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode},
};
use snafu::{ResultExt, Whatever, whatever};

mod build_info;
mod top;

use rara_app::{AppConfig, StartOptions, run as run_app, start_with_options};
use rara_channels::{
    terminal::{CliEvent, TerminalAdapter},
    tool_display::{tool_arguments_summary, tool_display_name},
};
use rara_kernel::{
    channel::types::{ChannelType, MessageContent},
    identity::UserId,
    io::{
        Endpoint, EndpointAddress, InteractionType, RawPlatformMessage,
        ReplyContext as IoReplyContext, StreamEvent,
    },
    session::{ChannelBinding, SessionEntry, SessionIndex, SessionKey},
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
    Gateway(GatewayArgs),
    Symphony(SymphonyArgs),
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
                dir: logs_dir_str,
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
                dir: logs_dir_str,
                enable_otlp_tracing: true,
                otlp_endpoint: Some("http://rara-infra-alloy:4318/v1/traces".to_string()),
                otlp_export_protocol: Some(OtlpExportProtocol::Http),
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

        // 0. Create Telegram notifier (required — fail fast if not configured).
        let Some(tg) = config.telegram.as_ref() else {
            whatever!("Gateway requires [telegram] config for notifications");
        };
        let Some(bot_token) = tg.bot_token.as_deref().filter(|s| !s.is_empty()) else {
            whatever!("Gateway requires telegram.bot_token");
        };
        let Some(raw_channel_id) = tg
            .notification_channel_id
            .as_deref()
            .filter(|s| !s.is_empty())
        else {
            whatever!("Gateway requires telegram.notification_channel_id");
        };
        let channel_id: i64 = raw_channel_id
            .parse()
            .whatever_context("telegram.notification_channel_id must be a valid i64")?;
        let notifier = std::sync::Arc::new(rara_app::gateway::UpdateNotifier::new(
            bot_token,
            channel_id,
            build_info::FULL_VERSION,
            &gateway_config.repo_url,
        ));

        // 1. Create supervisor + handle.
        let (mut supervisor, supervisor_handle) = rara_app::gateway::SupervisorService::new(
            gateway_config.clone(),
            port,
            std::sync::Arc::clone(&notifier),
        );

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
            supervisor_handle,
            update_state_rx: update_rx,
            shutdown: cancel.clone(),
        };
        let admin_bind = gateway_config.bind_address.clone();
        let _admin_handle = rara_app::gateway::server::serve(&admin_bind, admin_state)
            .await
            .whatever_context("Failed to start gateway admin HTTP server")?;

        // 5. Run supervisor (blocking).
        match supervisor.run().await {
            Ok(()) => {
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
                cancel.cancel();
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symphony command
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Args)]
#[command(about = "Start the symphony orchestrator standalone")]
struct SymphonyArgs {}

impl SymphonyArgs {
    async fn run() -> Result<(), Whatever> {
        let config = AppConfig::new().whatever_context("Failed to load config")?;

        let logs_dir = rara_paths::logs_dir();
        std::fs::create_dir_all(logs_dir).whatever_context("Failed to create logs directory")?;

        let _guards = common_telemetry::logging::init_global_logging(
            "rara-symphony",
            &common_telemetry::logging::LoggingOptions {
                dir: logs_dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            &common_telemetry::logging::TracingOptions::default(),
            None,
        );

        let Some(symphony_config) = config.symphony else {
            whatever!("Symphony requires [symphony] config section");
        };

        if !symphony_config.enabled {
            whatever!("Symphony is disabled in config (symphony.enabled = false)");
        }

        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_on_signal = cancel.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("received Ctrl+C, shutting down symphony gracefully…");
            cancel_on_signal.cancel();

            // Second Ctrl+C = force exit.
            tokio::signal::ctrl_c().await.ok();
            tracing::error!("received second Ctrl+C, force exiting");
            std::process::exit(1);
        });

        let symphony = rara_symphony::SymphonyService::new(
            symphony_config,
            cancel,
            std::env::var("GITHUB_TOKEN").ok(),
        );

        symphony
            .run()
            .await
            .whatever_context("symphony service failed")
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

        let logs_dir = rara_paths::logs_dir();
        std::fs::create_dir_all(logs_dir).whatever_context("Failed to create logs directory")?;

        // Chat mode stays interactive: keep logs on disk, not on stdout.
        let _guards = common_telemetry::logging::init_global_logging(
            "rara-cli",
            &common_telemetry::logging::LoggingOptions {
                dir: logs_dir.to_string_lossy().into_owned(),
                append_stdout: false,
                ..Default::default()
            },
            &common_telemetry::logging::TracingOptions::default(),
            None,
        );

        // Create the terminal adapter.
        let (adapter, event_rx) = TerminalAdapter::new();
        let adapter = Arc::new(adapter);

        // Start the app with the CLI adapter injected.
        let mut app_handle = start_with_options(
            config,
            StartOptions {
                cli_adapter: Some(adapter.clone()),
            },
        )
        .await
        .whatever_context("Failed to start application")?;

        let kernel_handle = match app_handle.kernel_handle.take() {
            Some(h) => h,
            None => whatever!("kernel handle not available"),
        };
        let endpoint_registry = kernel_handle.endpoint_registry().clone();
        let stream_hub = kernel_handle.stream_hub().clone();

        // Resolve the interactive chat alias to a stable internal session up front
        // so the first reply can stream back immediately.
        let session_alias = self.session.clone();
        let user_id_str = self.user_id.clone();
        let resolved_user_id = cli_kernel_user_id(&user_id_str);
        let resolved_session_id =
            get_or_create_cli_session(kernel_handle.session_index().as_ref(), &session_alias)
                .await
                .whatever_context("Failed to resolve CLI chat session")?;

        // Register CLI endpoint in the EndpointRegistry.
        let cli_endpoint = Endpoint {
            channel_type: ChannelType::Cli,
            address:      EndpointAddress::Cli {
                session_id: session_alias.clone(),
            },
        };
        endpoint_registry.register(&resolved_user_id, cli_endpoint);

        // Spawn StreamHub forwarder task.
        let forwarder_event_tx = adapter.clone();
        let forwarder_session_id = resolved_session_id.clone();
        let forwarder_hub = stream_hub.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            let mut active_streams: std::collections::HashSet<rara_kernel::io::StreamId> =
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
        eprintln!("Session: {}  User: {}", session_alias, user_id_str);
        eprintln!("Type your message and press Enter. Ctrl+C to exit.\n");

        // Run the REPL loop.
        run_repl(event_rx, kernel_handle, session_alias, user_id_str).await;

        // Cleanup.
        app_handle.shutdown();
        // Give a moment for graceful shutdown.
        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(())
    }
}

fn cli_kernel_user_id(user_id: &str) -> UserId { UserId(user_id.to_owned()) }

async fn get_or_create_cli_session(
    session_index: &dyn SessionIndex,
    chat_id: &str,
) -> Result<SessionKey, Whatever> {
    if let Some(binding) = session_index
        .get_channel_binding("cli", chat_id)
        .await
        .whatever_context("Failed to load CLI channel binding")?
    {
        return Ok(binding.session_key);
    }

    let now = Utc::now();
    let entry = SessionEntry {
        key:           SessionKey::new(),
        title:         Some(chat_id.to_owned()),
        model:         None,
        system_prompt: None,
        message_count: 0,
        preview:       None,
        metadata:      None,
        created_at:    now,
        updated_at:    now,
    };
    let created = session_index
        .create_session(&entry)
        .await
        .whatever_context("Failed to create CLI chat session")?;
    let binding = ChannelBinding {
        channel_type: "cli".to_owned(),
        chat_id:      chat_id.to_owned(),
        session_key:  created.key.clone(),
        created_at:   now,
        updated_at:   now,
    };
    session_index
        .bind_channel(&binding)
        .await
        .whatever_context("Failed to bind CLI chat session")?;
    Ok(created.key)
}

/// Convert a [`StreamEvent`] into a [`CliEvent`].
fn stream_event_to_cli_event(event: StreamEvent) -> CliEvent {
    match event {
        StreamEvent::TextDelta { text: t } => CliEvent::TextDelta { text: t },
        StreamEvent::ReasoningDelta { text: t } => CliEvent::ReasoningDelta { text: t },
        StreamEvent::ToolCallStart {
            name, arguments, ..
        } => {
            let summary = tool_arguments_summary(&name, &arguments);
            let display = tool_display_name(&name).to_owned();
            CliEvent::ToolCallStart {
                name: display,
                summary,
            }
        }
        StreamEvent::ToolCallEnd {
            error,
            success,
            result_preview,
            ..
        } => {
            if let Some(ref err) = error {
                eprintln!("\x1b[31m[tool error] {}\x1b[0m", err);
            }
            CliEvent::ToolCallEnd {
                success,
                result_preview,
            }
        }
        StreamEvent::BackgroundToolStarted { name, summary, .. } => CliEvent::Progress {
            text: format!("[background tool] {name}: {summary}"),
        },
        StreamEvent::BackgroundToolFinished {
            success, summary, ..
        } => CliEvent::Progress {
            text: format!(
                "[background tool {}] {}",
                if success { "done" } else { "failed" },
                summary
            ),
        },
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
        platform_user_id:    format!("cli:{user_id}"),
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
    kernel_handle: rara_kernel::handle::KernelHandle,
    session_key: String,
    user_id: String,
) {
    use std::io::Write;

    let Ok(_raw_mode_guard) = RawModeGuard::new() else {
        eprintln!("[error] Failed to enable interactive terminal mode");
        return;
    };

    let mut input = ChatInputBuffer::default();
    let mut in_stream = false;
    redraw_prompt(&input);

    loop {
        tokio::select! {
            maybe_event = poll_chat_event(), if !in_stream => {
                if let Some(Event::Key(key)) = maybe_event {
                    match input.handle_key(key) {
                        ChatInputAction::None => {
                            redraw_prompt(&input);
                        }
                        ChatInputAction::Submit(text) => {
                            clear_prompt_line();
                            println!("{}", submitted_prompt_line(&text));
                            let raw = build_cli_raw_message(&session_key, &user_id, &text);
                            if let Err(e) = kernel_handle.ingest(raw).await {
                                eprintln!("[error] Failed to send message: {}", e);
                                redraw_prompt(&input);
                            } else {
                                in_stream = true;
                            }
                        }
                        ChatInputAction::Quit => {
                            clear_prompt_line();
                            break;
                        }
                    }
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(CliEvent::Reply { content }) => {
                        clear_prompt_line();
                        if in_stream {
                            println!();
                        }
                        println!("{}", content);
                        in_stream = false;
                        redraw_prompt(&input);
                    }
                    Some(CliEvent::TextDelta { text }) => {
                        print!("{}", text);
                        let _ = std::io::stdout().flush();
                    }
                    Some(CliEvent::ReasoningDelta { text }) => {
                        eprint!("\x1b[2m{}\x1b[0m", text);
                        let _ = std::io::stderr().flush();
                    }
                    Some(CliEvent::ToolCallStart { name, summary }) => {
                        clear_prompt_line();
                        if summary.is_empty() {
                            eprintln!("\x1b[33m[tool] {}...\x1b[0m", name);
                        } else {
                            eprintln!("\x1b[33m[tool] {}: {}\x1b[0m", name, summary);
                        }
                        if !in_stream {
                            redraw_prompt(&input);
                        }
                    }
                    Some(CliEvent::ToolCallEnd { success, result_preview }) => {
                        clear_prompt_line();
                        if !success {
                            eprintln!("\x1b[31m[tool] failed\x1b[0m");
                        } else if !result_preview.is_empty() {
                            let first_line = result_preview.lines().next().unwrap_or(&result_preview);
                            let display: String = first_line.chars().take(100).collect();
                            if first_line.chars().count() > 100 {
                                eprintln!("\x1b[33m[tool] done: {}\u{2026}\x1b[0m", display);
                            } else {
                                eprintln!("\x1b[33m[tool] done: {}\x1b[0m", display);
                            }
                        } else {
                            eprintln!("\x1b[33m[tool] done\x1b[0m");
                        }
                        if !in_stream {
                            redraw_prompt(&input);
                        }
                    }
                    Some(CliEvent::Progress { text }) => {
                        if !text.is_empty() {
                            clear_prompt_line();
                            eprintln!("\x1b[36m[{}]\x1b[0m", text);
                            if !in_stream {
                                redraw_prompt(&input);
                            }
                        }
                    }
                    Some(CliEvent::Error { message }) => {
                        clear_prompt_line();
                        eprintln!("\x1b[31m[error] {}\x1b[0m", message);
                        in_stream = false;
                        redraw_prompt(&input);
                    }
                    Some(CliEvent::Done) => {
                        clear_prompt_line();
                        if in_stream {
                            println!();
                        }
                        in_stream = false;
                        redraw_prompt(&input);
                    }
                    None => {
                        clear_prompt_line();
                        break;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Default)]
struct ChatInputBuffer {
    text: String,
}

enum ChatInputAction {
    None,
    Submit(String),
    Quit,
}

impl ChatInputBuffer {
    fn handle_key(&mut self, key: KeyEvent) -> ChatInputAction {
        if key.kind != KeyEventKind::Press {
            return ChatInputAction::None;
        }
        match key.code {
            KeyCode::Enter => {
                let text = self.text.trim().to_owned();
                self.text.clear();
                if matches!(text.as_str(), "/quit" | "/exit") {
                    ChatInputAction::Quit
                } else if text.is_empty() {
                    ChatInputAction::None
                } else {
                    ChatInputAction::Submit(text)
                }
            }
            KeyCode::Backspace => {
                self.text.pop();
                ChatInputAction::None
            }
            KeyCode::Esc => {
                self.text.clear();
                ChatInputAction::None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear();
                ChatInputAction::None
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.text.push(ch);
                ChatInputAction::None
            }
            _ => ChatInputAction::None,
        }
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn new() -> std::io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) { let _ = disable_raw_mode(); }
}

fn clear_prompt_line() {
    let _ = execute!(
        std::io::stderr(),
        MoveToColumn(0),
        Clear(ClearType::CurrentLine)
    );
}

fn redraw_prompt(input: &ChatInputBuffer) {
    clear_prompt_line();
    eprint!("> {}", input.text);
    let _ = std::io::stderr().flush();
}

fn submitted_prompt_line(text: &str) -> String { format!("> {text}") }

async fn poll_chat_event() -> Option<Event> {
    tokio::task::spawn_blocking(|| {
        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            event::read().ok()
        } else {
            None
        }
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use rara_kernel::{identity::UserId, io::StreamEvent, session::SessionIndex};
    use rara_sessions::file_index::FileSessionIndex;

    use super::{
        ChatInputAction, ChatInputBuffer, cli_kernel_user_id, get_or_create_cli_session,
        stream_event_to_cli_event, submitted_prompt_line,
    };

    #[tokio::test]
    async fn cli_session_binding_is_created_once_and_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let index = FileSessionIndex::new(tmp.path()).await.unwrap();
        let session_index: &dyn SessionIndex = &index;

        let first = get_or_create_cli_session(session_index, "default")
            .await
            .unwrap();
        let second = get_or_create_cli_session(session_index, "default")
            .await
            .unwrap();
        let binding = session_index
            .get_channel_binding("cli", "default")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(binding.session_key, first);
    }

    #[test]
    fn cli_endpoint_registration_uses_kernel_user_id() {
        assert_eq!(cli_kernel_user_id("ryan"), UserId("ryan".to_owned()));
    }

    #[test]
    fn chat_input_backspace_and_ctrl_u_clear_current_line() {
        let mut input = ChatInputBuffer::default();

        for ch in ['h', 'e', 'l', 'l', 'o'] {
            assert!(matches!(
                input.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)),
                ChatInputAction::None
            ));
        }
        assert_eq!(input.text, "hello");

        input.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(input.text, "hell");

        input.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(input.text.is_empty());
    }

    #[test]
    fn submitted_message_keeps_user_text_visible() {
        assert_eq!(submitted_prompt_line("hello"), "> hello");
    }

    #[test]
    fn reasoning_deltas_are_forwarded_to_cli() {
        let event = StreamEvent::ReasoningDelta {
            text: "internal".to_owned(),
        };

        assert!(matches!(
            stream_event_to_cli_event(event),
            super::CliEvent::ReasoningDelta { text } if text == "internal"
        ));
    }

    #[test]
    fn text_deltas_still_stream_to_cli() {
        let event = StreamEvent::TextDelta {
            text: "hello".to_owned(),
        };

        assert!(matches!(
            stream_event_to_cli_event(event),
            super::CliEvent::TextDelta { text } if text == "hello"
        ));
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
        Commands::Symphony(_) => SymphonyArgs::run().await,
    }
}
