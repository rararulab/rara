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

use base64::Engine;
use chrono::Utc;
use clap::Args;
use crossterm::event::{self, Event, KeyEventKind};
use rara_app::{AppConfig, StartOptions, start_with_options};
use rara_channels::{
    terminal::{CliEvent, TerminalAdapter},
    tool_display::{tool_arguments_summary, tool_display_name},
};
use rara_kernel::{
    channel::{
        command::{CommandContext, CommandHandler, CommandInfo, CommandResult as CmdResult},
        types::{ChannelType, ChannelUser, ContentBlock, MessageContent},
    },
    handle::KernelHandle,
    identity::UserId,
    io::{
        Endpoint, EndpointAddress, InteractionType, RawPlatformMessage,
        ReplyContext as IoReplyContext, StreamEvent,
    },
    session::{ChannelBinding, SessionEntry, SessionIndex, SessionKey},
};
use snafu::{ResultExt, Whatever, whatever};

use crate::chat::{
    app::{ChatAction, ChatState, Role},
    ui::render,
};

pub mod app;
pub mod theme;
pub mod ui;

#[derive(Debug, Clone, Args)]
#[command(about = "Start interactive chat with an agent")]
pub struct ChatArgs {
    /// Session key for conversation continuity.
    #[arg(long, default_value = "default")]
    session: String,

    /// User identifier.
    #[arg(long, default_value = "local")]
    user_id: String,
}

impl ChatArgs {
    pub async fn run(self) -> Result<(), Whatever> {
        let config = AppConfig::new().whatever_context("Failed to load config")?;
        let default_model_label = default_model_label(&config);

        let logs_dir = rara_paths::logs_dir();
        std::fs::create_dir_all(logs_dir).whatever_context("Failed to create logs directory")?;

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

        let (adapter, event_rx) = TerminalAdapter::new();
        let adapter = Arc::new(adapter);
        let mut app_handle = start_with_options(
            config,
            StartOptions {
                cli_adapter: Some(adapter.clone()),
            },
        )
        .await
        .whatever_context("Failed to start application")?;

        let kernel_handle = match app_handle.kernel_handle.take() {
            Some(handle) => handle,
            None => whatever!("kernel handle not available"),
        };
        let endpoint_registry = kernel_handle.endpoint_registry().clone();
        let stream_hub = kernel_handle.stream_hub().clone();

        let session_alias = self.session.clone();
        let user_id = self.user_id.clone();
        let resolved_user_id = cli_kernel_user_id(&user_id);
        let resolved_session_id =
            get_or_create_cli_session(kernel_handle.session_index().as_ref(), &session_alias)
                .await
                .whatever_context("Failed to resolve CLI chat session")?;

        let cli_endpoint = Endpoint {
            channel_type: ChannelType::Cli,
            address:      EndpointAddress::Cli {
                session_id: session_alias.clone(),
            },
        };
        endpoint_registry.register(&resolved_user_id, cli_endpoint);

        spawn_stream_forwarder(adapter, stream_hub, resolved_session_id);

        let mut terminal = ratatui::init();
        let mut chat_state = ChatState::new(session_alias.clone(), user_id.clone());
        chat_state.model_label = default_model_label;

        let command_handlers = app_handle.command_handlers.clone();

        let result = run_chat_tui(
            &mut terminal,
            &mut chat_state,
            event_rx,
            kernel_handle,
            session_alias,
            user_id,
            &command_handlers,
        )
        .await;

        ratatui::restore();
        app_handle.shutdown();
        tokio::time::sleep(Duration::from_millis(500)).await;

        result
    }
}

fn spawn_stream_forwarder(
    adapter: Arc<TerminalAdapter>,
    stream_hub: Arc<rara_kernel::io::StreamHub>,
    session_key: SessionKey,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let mut active_streams = std::collections::HashSet::new();

        loop {
            interval.tick().await;
            let subscriptions = stream_hub.subscribe_session(&session_key);
            for (stream_id, mut rx) in subscriptions {
                if active_streams.contains(&stream_id) {
                    while let Ok(event) = rx.try_recv() {
                        let _ = adapter.send_cli_event(stream_event_to_cli_event(event));
                    }
                    continue;
                }

                active_streams.insert(stream_id.clone());
                let adapter = adapter.clone();
                tokio::spawn(async move {
                    while let Ok(event) = rx.recv().await {
                        let _ = adapter.send_cli_event(stream_event_to_cli_event(event));
                    }
                    let _ = adapter.send_cli_event(CliEvent::Done);
                });
            }
        }
    });
}

async fn run_chat_tui(
    terminal: &mut ratatui::DefaultTerminal,
    state: &mut ChatState,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<CliEvent>,
    kernel_handle: KernelHandle,
    session_key: String,
    user_id: String,
    command_handlers: &[Arc<dyn CommandHandler>],
) -> Result<(), Whatever> {
    let mut tick = tokio::time::interval(Duration::from_millis(100));

    loop {
        terminal
            .draw(|frame| render(frame, state, frame.area()))
            .whatever_context("Failed to draw chat TUI")?;

        tokio::select! {
            _ = tick.tick() => {
                state.tick();
            }
            maybe_event = poll_crossterm_event() => {
                if let Some(Event::Key(key)) = maybe_event {
                    match state.handle_key(key) {
                        ChatAction::Continue => {}
                        ChatAction::Back => break,
                        ChatAction::SlashCommand(command) => {
                            if handle_slash_command(
                                state,
                                &command,
                                command_handlers,
                                &session_key,
                                &user_id,
                            ).await {
                                break;
                            }
                        }
                        ChatAction::SendMessage(text) => {
                            let image_paths = std::mem::take(&mut state.staged_images);
                            send_cli_message(
                                state,
                                &kernel_handle,
                                &session_key,
                                &user_id,
                                text,
                                image_paths,
                            )
                            .await;
                        }
                    }
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(CliEvent::Done) => {
                        state.handle_cli_event(CliEvent::Done);
                        if let Some((text, image_paths)) = state.take_staged() {
                            send_cli_message(
                                state,
                                &kernel_handle,
                                &session_key,
                                &user_id,
                                text,
                                image_paths,
                            )
                            .await;
                        }
                    }
                    Some(event) => state.handle_cli_event(event),
                    None => break,
                }
            }
        }
    }

    Ok(())
}

async fn handle_slash_command(
    state: &mut ChatState,
    command: &str,
    handlers: &[Arc<dyn CommandHandler>],
    session_key: &str,
    user_id: &str,
) -> bool {
    let parts: Vec<&str> = command.splitn(2, ' ').collect();
    let cmd_token = parts[0];

    // TUI-local commands take priority.
    match cmd_token {
        "/help" => {
            let mut lines = vec![
                "/help         — show this help".to_owned(),
                "/exit         — end chat session".to_owned(),
                "/image <path> — stage a local image for the next turn".to_owned(),
                "/images       — list staged images".to_owned(),
                "/clear-images — clear staged images".to_owned(),
            ];
            // Append registered handler commands to help text.
            for handler in handlers {
                for def in handler.commands() {
                    let usage = def.usage.as_deref().unwrap_or("");
                    lines.push(format!("{:<14}— {}", usage, def.description));
                }
            }
            state.push_message(Role::System, lines.join("\n"));
            return false;
        }
        "/exit" | "/quit" => return true,
        "/image" => {
            let Some(raw_path) = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                state.push_message(
                    Role::System,
                    "Usage: /image /abs/path/to/file.png".to_owned(),
                );
                return false;
            };
            match tokio::fs::canonicalize(raw_path).await {
                Ok(path) => {
                    let path = path.to_string_lossy().into_owned();
                    state.staged_images.push(path.clone());
                    state.push_message(Role::System, format!("Staged image: {path}"));
                }
                Err(e) => {
                    state.push_message(Role::System, format!("Failed to stage image: {e}"));
                }
            }
            return false;
        }
        "/images" => {
            if state.staged_images.is_empty() {
                state.push_message(Role::System, "No staged images.".to_owned());
            } else {
                let lines = state
                    .staged_images
                    .iter()
                    .enumerate()
                    .map(|(index, path)| format!("{}. {}", index + 1, path))
                    .collect::<Vec<_>>()
                    .join("\n");
                state.push_message(Role::System, format!("Staged images:\n{lines}"));
            }
            return false;
        }
        "/clear-images" => {
            state.staged_images.clear();
            state.push_message(Role::System, "Cleared staged images.".to_owned());
            return false;
        }
        _ => {}
    }

    // Try kernel command handlers.
    let cmd_name = cmd_token.trim_start_matches('/');
    if !cmd_name.is_empty() {
        let matched_handler = handlers
            .iter()
            .find(|h| h.commands().iter().any(|def| def.name == cmd_name));

        if let Some(handler) = matched_handler {
            let args = if parts.len() > 1 { parts[1] } else { "" };
            let info = CommandInfo {
                name: cmd_name.to_owned(),
                args: args.to_owned(),
                raw:  command.to_owned(),
            };

            let mut metadata = HashMap::new();
            metadata.insert(
                "cli_chat_id".to_owned(),
                serde_json::Value::String(session_key.to_owned()),
            );

            let ctx = CommandContext {
                channel_type: ChannelType::Cli,
                session_key: session_key.to_owned(),
                user: ChannelUser {
                    platform_id:  format!("cli:{user_id}"),
                    display_name: Some(user_id.to_owned()),
                },
                metadata,
            };

            match handler.handle(&info, &ctx).await {
                Ok(result) => {
                    render_command_result(state, result);
                }
                Err(e) => {
                    state.push_message(Role::System, format!("Command failed: {e}"));
                }
            }
            return false;
        }
    }

    state.push_message(
        Role::System,
        format!("Unknown command: {cmd_token}. Type /help"),
    );
    false
}

/// Render a [`CmdResult`] into the TUI chat state.
fn render_command_result(state: &mut ChatState, result: CmdResult) {
    match result {
        CmdResult::Text(s) => state.push_message(Role::System, s),
        CmdResult::Html(s) => {
            // Strip basic HTML tags for terminal display.
            state.push_message(Role::System, strip_html_tags(&s));
        }
        CmdResult::HtmlWithKeyboard { html, .. } => {
            // Show text portion; inline keyboards are not supported in TUI.
            state.push_message(Role::System, strip_html_tags(&html));
        }
        CmdResult::Photo { caption, .. } => {
            let text = caption.unwrap_or_else(|| "[Photo]".to_string());
            state.push_message(Role::System, text);
        }
        CmdResult::None => {}
    }
}

/// Minimal HTML tag stripper for terminal display.
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    // Unescape common HTML entities.
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn default_model_label(config: &AppConfig) -> String {
    let Some(llm) = config.llm.as_ref() else {
        return "default".to_owned();
    };
    let Some(provider) = llm.default_provider.as_deref() else {
        return "default".to_owned();
    };
    let Some(provider_config) = llm.providers.get(provider) else {
        return provider.to_owned();
    };
    match provider_config.default_model.as_deref() {
        Some(model) if !model.is_empty() => format!("{provider}/{model}"),
        _ => provider.to_owned(),
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

fn stream_event_to_cli_event(event: StreamEvent) -> CliEvent {
    match event {
        StreamEvent::TextDelta { text } => CliEvent::TextDelta { text },
        StreamEvent::ReasoningDelta { text } => CliEvent::ReasoningDelta { text },
        StreamEvent::ToolCallStart {
            name, arguments, ..
        } => {
            let summary = tool_arguments_summary(&name, &arguments);
            let name = tool_display_name(&name).to_owned();
            CliEvent::ToolCallStart { name, summary }
        }
        StreamEvent::ToolCallEnd {
            success,
            result_preview,
            ..
        } => CliEvent::ToolCallEnd {
            success,
            result_preview,
        },
        StreamEvent::Progress { stage } => CliEvent::Progress { text: stage },
        StreamEvent::TextClear => CliEvent::Progress {
            text: String::new(),
        },
        StreamEvent::TurnMetrics {
            duration_ms,
            iterations,
            tool_calls,
            model,
            rara_message_id: _,
        } => CliEvent::Progress {
            text: format!(
                "[{model}] {iterations} iterations, {tool_calls} tool calls, {duration_ms}ms"
            ),
        },
        StreamEvent::PlanCreated {
            compact_summary,
            total_steps,
            ..
        } => CliEvent::Progress {
            text: format!("Plan ({total_steps} steps): {compact_summary}"),
        },
        StreamEvent::PlanProgress { status_text, .. } => CliEvent::Progress { text: status_text },
        StreamEvent::PlanReplan { reason } => CliEvent::Progress {
            text: format!("Replanning: {reason}"),
        },
        StreamEvent::PlanCompleted { summary } => CliEvent::Progress {
            text: format!("Plan completed: {summary}"),
        },
        StreamEvent::UsageUpdate { .. } => CliEvent::Progress {
            text: String::new(),
        },
        StreamEvent::BackgroundTaskStarted {
            agent_name,
            description,
            ..
        } => CliEvent::Progress {
            text: format!("Background: {agent_name} — {description}"),
        },
        StreamEvent::BackgroundTaskDone { task_id, status } => CliEvent::Progress {
            text: format!("Background task {task_id} {status:?}"),
        },
        StreamEvent::DockTurnComplete { session_id, .. } => CliEvent::Progress {
            text: format!("Dock turn complete: {session_id}"),
        },
    }
}

async fn send_cli_message(
    state: &mut ChatState,
    kernel_handle: &KernelHandle,
    session_key: &str,
    user_id: &str,
    text: String,
    image_paths: Vec<String>,
) {
    state.is_streaming = true;
    state.loading_hint = rara_kernel::io::loading_hints::random_hint().to_string();
    state.thinking = true;
    state.streaming_chars = 0;
    state.last_tokens = None;
    state.last_cost_usd = None;
    state.status_msg = None;

    let attachments = match load_image_blocks(&image_paths).await {
        Ok(attachments) => attachments,
        Err(error) => {
            state.staged_images = image_paths;
            state.handle_cli_event(CliEvent::Error {
                message: error.to_string(),
            });
            return;
        }
    };

    let raw = build_cli_raw_message(session_key, user_id, &text, attachments);
    if let Err(error) = kernel_handle.ingest(raw).await {
        state.handle_cli_event(CliEvent::Error {
            message: error.to_string(),
        });
    }
}

async fn load_image_blocks(image_paths: &[String]) -> Result<Vec<ContentBlock>, Whatever> {
    let mut blocks = Vec::with_capacity(image_paths.len());

    for path in image_paths {
        let bytes = tokio::fs::read(path)
            .await
            .with_whatever_context(|_| format!("Failed to read image: {path}"))?;
        let (compressed, media_type) = rara_kernel::llm::image::compress_image(
            &bytes,
            rara_kernel::llm::image::DEFAULT_MAX_EDGE,
            rara_kernel::llm::image::DEFAULT_QUALITY,
        )
        .with_whatever_context(|_| format!("Failed to compress image: {path}"))?;
        blocks.push(ContentBlock::ImageBase64 {
            media_type,
            data: base64::engine::general_purpose::STANDARD.encode(&compressed),
        });
    }

    Ok(blocks)
}

fn build_cli_raw_message(
    session_key: &str,
    user_id: &str,
    content: &str,
    attachments: Vec<ContentBlock>,
) -> RawPlatformMessage {
    let content = if attachments.is_empty() {
        MessageContent::Text(content.to_owned())
    } else {
        let mut blocks = Vec::with_capacity(attachments.len() + usize::from(!content.is_empty()));
        if !content.is_empty() {
            blocks.push(ContentBlock::Text {
                text: content.to_owned(),
            });
        }
        blocks.extend(attachments);
        MessageContent::Multimodal(blocks)
    };

    RawPlatformMessage {
        channel_type: ChannelType::Cli,
        platform_message_id: Some(ulid::Ulid::new().to_string()),
        platform_user_id: format!("cli:{user_id}"),
        platform_chat_id: Some(session_key.to_owned()),
        content,
        reply_context: Some(IoReplyContext {
            thread_id:                None,
            reply_to_platform_msg_id: None,
            interaction_type:         InteractionType::Message,
        }),
        metadata: HashMap::new(),
    }
}

async fn poll_crossterm_event() -> Option<Event> {
    tokio::task::spawn_blocking(|| {
        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            event::read().ok().and_then(|event| match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => Some(Event::Key(key)),
                _ => None,
            })
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
    use rara_channels::terminal::CliEvent;
    use rara_kernel::{
        channel::types::{ContentBlock, MessageContent},
        identity::UserId,
        io::StreamEvent,
        session::SessionIndex,
    };
    use rara_sessions::file_index::FileSessionIndex;

    use super::{
        build_cli_raw_message, cli_kernel_user_id, get_or_create_cli_session, handle_slash_command,
        stream_event_to_cli_event,
    };
    use crate::chat::app::{CHAT_BANNER, ChatState};

    #[tokio::test]
    async fn cli_session_binding_is_created_once_and_reused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let index = FileSessionIndex::new(tmp.path()).await.expect("index");
        let session_index: &dyn SessionIndex = &index;

        let first = get_or_create_cli_session(session_index, "default")
            .await
            .expect("first session");
        let second = get_or_create_cli_session(session_index, "default")
            .await
            .expect("second session");
        let binding = session_index
            .get_channel_binding("cli", "default")
            .await
            .expect("binding query")
            .expect("binding");

        assert_eq!(first, second);
        assert_eq!(binding.session_key, first);
    }

    #[test]
    fn cli_endpoint_registration_uses_kernel_user_id() {
        assert_eq!(cli_kernel_user_id("ryan"), UserId("ryan".to_owned()));
    }

    #[test]
    fn reasoning_deltas_are_forwarded_to_cli() {
        let event = StreamEvent::ReasoningDelta {
            text: "internal".to_owned(),
        };

        assert!(matches!(
            stream_event_to_cli_event(event),
            CliEvent::ReasoningDelta { text } if text == "internal"
        ));
    }

    #[test]
    fn text_deltas_still_stream_to_cli() {
        let event = StreamEvent::TextDelta {
            text: "hello".to_owned(),
        };

        assert!(matches!(
            stream_event_to_cli_event(event),
            CliEvent::TextDelta { text } if text == "hello"
        ));
    }

    #[tokio::test]
    async fn exit_slash_command_requests_shutdown_without_message() {
        let mut state = ChatState::new("default".into(), "local".into());
        let handlers: Vec<std::sync::Arc<dyn rara_kernel::channel::command::CommandHandler>> =
            vec![];

        assert!(handle_slash_command(&mut state, "/exit", &handlers, "default", "local").await);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].text, CHAT_BANNER);
    }

    #[test]
    fn cli_raw_message_is_multimodal_when_image_paths_are_present() {
        let raw = build_cli_raw_message(
            "default",
            "local",
            "describe",
            vec![ContentBlock::ImageBase64 {
                media_type: "image/png".to_owned(),
                data:       "AAAA".to_owned(),
            }],
        );

        assert!(matches!(
            raw.content,
            MessageContent::Multimodal(blocks) if blocks.len() == 2
        ));
    }
}
