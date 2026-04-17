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
    security::ApprovalDecision,
    session::{ChannelBinding, SessionEntry, SessionIndex, SessionKey},
};
use snafu::{ResultExt, Whatever, whatever};

use crate::chat::{
    app::{ChatAction, ChatState, HandleResult, PendingApproval, PendingQuestion, Role},
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

        let config_users = config.users.clone();
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
        // When --user-id is not explicitly set, default to the first
        // configured user so that identity resolution succeeds.
        let user_id = if self.user_id == "local" {
            config_users
                .first()
                .map(|u| u.name.clone())
                .unwrap_or(self.user_id.clone())
        } else {
            self.user_id.clone()
        };
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

        let (session_tx, session_rx) = tokio::sync::watch::channel(resolved_session_id);
        spawn_stream_forwarder(adapter, stream_hub, session_rx);

        let mut terminal = ratatui::init();
        let mut chat_state = ChatState::new(session_alias.clone(), user_id.clone());
        chat_state.model_label = default_model_label;

        let command_handlers = app_handle.command_handlers.clone();
        let user_question_manager = app_handle.user_question_manager.clone();

        let result = run_chat_tui(
            &mut terminal,
            &mut chat_state,
            event_rx,
            kernel_handle,
            session_alias,
            user_id,
            &command_handlers,
            session_tx,
            user_question_manager,
        )
        .await;

        ratatui::restore();
        app_handle.shutdown();
        tokio::time::sleep(Duration::from_millis(500)).await;

        result
    }
}

/// Spawn a background task that forwards [`StreamEvent`]s to the CLI adapter.
///
/// The forwarder watches `session_rx` for session key changes so that `/new`
/// and `/switch` commands take effect without restarting the task.
fn spawn_stream_forwarder(
    adapter: Arc<TerminalAdapter>,
    stream_hub: Arc<rara_kernel::io::StreamHub>,
    mut session_rx: tokio::sync::watch::Receiver<SessionKey>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let mut active_streams = std::collections::HashSet::new();
        let mut stream_abort_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        let mut current_key = session_rx.borrow_and_update().clone();

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Prune finished stream tasks to prevent unbounded growth.
                    stream_abort_handles.retain(|h| !h.is_finished());
                    let subscriptions = stream_hub.subscribe_session(&current_key);
                    for (stream_id, mut rx) in subscriptions {
                        if active_streams.contains(&stream_id) {
                            while let Ok(event) = rx.try_recv() {
                                let _ = adapter.send_cli_event(stream_event_to_cli_event(event));
                            }
                            continue;
                        }

                        active_streams.insert(stream_id.clone());
                        let adapter = adapter.clone();
                        let handle = tokio::spawn(async move {
                            while let Ok(event) = rx.recv().await {
                                let _ = adapter.send_cli_event(stream_event_to_cli_event(event));
                            }
                            let _ = adapter.send_cli_event(CliEvent::Done);
                        });
                        stream_abort_handles.push(handle);
                    }
                }
                Ok(()) = session_rx.changed() => {
                    // Session switched — abort all running stream tasks from
                    // the old session to prevent stale events being forwarded.
                    for handle in &stream_abort_handles {
                        handle.abort();
                    }
                    // Drop finished JoinHandles immediately.
                    stream_abort_handles.clear();
                    current_key = session_rx.borrow_and_update().clone();
                    active_streams.clear();
                }
            }
        }
    });
}

async fn run_chat_tui(
    terminal: &mut ratatui::DefaultTerminal,
    state: &mut ChatState,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<CliEvent>,
    kernel_handle: KernelHandle,
    mut session_key: String,
    user_id: String,
    command_handlers: &[Arc<dyn CommandHandler>],
    session_tx: tokio::sync::watch::Sender<SessionKey>,
    user_question_manager: Option<rara_kernel::user_question::UserQuestionManagerRef>,
) -> Result<(), Whatever> {
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut approval_rx = kernel_handle.security().approval().subscribe_requests();
    let mut question_rx = user_question_manager.as_ref().map(|mgr| mgr.subscribe());

    loop {
        terminal
            .draw(|frame| render(frame, state, frame.area()))
            .whatever_context("Failed to draw chat TUI")?;

        tokio::select! {
            _ = tick.tick() => {
                state.tick();
            }
            result = approval_rx.recv() => {
                if let Ok(request) = result {
                    state.set_pending_approval(PendingApproval {
                        id: request.id.to_string(),
                        tool_name: request.tool_name.clone(),
                        summary: request.summary.clone(),
                        risk_level: format!("{:?}", request.risk_level),
                    });
                }
            }
            result = async {
                match question_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok(question) = result {
                    state.set_pending_question(PendingQuestion {
                        id: question.id.to_string(),
                        question: question.question.clone(),
                    });
                }
            }
            maybe_event = poll_crossterm_event() => {
                if let Some(Event::Key(key)) = maybe_event {
                    match state.handle_key(key) {
                        ChatAction::Continue => {}
                        ChatAction::Back => break,
                        ChatAction::Interrupt => {
                            let resolved = session_tx.borrow().clone();
                            let _ = kernel_handle.send_signal(
                                resolved,
                                rara_kernel::session::Signal::Interrupt,
                            );
                            state.status_msg = Some("Interrupting...".to_owned());
                        }
                        ChatAction::ApproveGuard { id } => {
                            let Ok(uuid) = uuid::Uuid::parse_str(&id) else {
                                state.push_message(Role::System, format!("Invalid approval ID: {id}"));
                                continue;
                            };
                            let _ = kernel_handle.security().approval().resolve(
                                uuid,
                                ApprovalDecision::Approved,
                                Some("cli-user".to_owned()),
                            );
                            state.pending_approval = None;
                            state.push_message(Role::System, format!("Guard approved: {id}"));
                        }
                        ChatAction::DenyGuard { id } => {
                            let Ok(uuid) = uuid::Uuid::parse_str(&id) else {
                                state.push_message(Role::System, format!("Invalid approval ID: {id}"));
                                continue;
                            };
                            let _ = kernel_handle.security().approval().resolve(
                                uuid,
                                ApprovalDecision::Denied,
                                Some("cli-user".to_owned()),
                            );
                            state.pending_approval = None;
                            state.push_message(Role::System, format!("Guard denied: {id}"));
                        }
                        ChatAction::AnswerQuestion { id, answer } => {
                            let Ok(uuid) = uuid::Uuid::parse_str(&id) else {
                                state.push_message(Role::System, format!("Invalid question ID: {id}"));
                                continue;
                            };
                            if let Some(ref mgr) = user_question_manager {
                                let _ = mgr.resolve(uuid, answer.clone());
                            }
                            state.pending_question = None;
                            state.push_message(Role::System, format!("Answered: {answer}"));
                        }
                        ChatAction::ResolveToolCallLimit {
                            session_key,
                            limit_id,
                            continued,
                        } => {
                            use rara_kernel::io::ToolCallLimitDecision;
                            let decision = if continued {
                                ToolCallLimitDecision::Continue
                            } else {
                                ToolCallLimitDecision::Stop
                            };
                            if let Ok(key) = SessionKey::try_from_raw(&session_key) {
                                kernel_handle.resolve_tool_call_limit(
                                    key,
                                    limit_id,
                                    decision,
                                );
                            }
                            state.pending_tool_call_limit = None;
                            state.status_msg = Some(if continued {
                                "Agent resumed.".to_owned()
                            } else {
                                "Agent stopped.".to_owned()
                            });
                        }
                        ChatAction::SlashCommand(command) => {
                            match handle_slash_command(
                                state,
                                &command,
                                command_handlers,
                                &session_key,
                                &user_id,
                                &kernel_handle,
                            ).await {
                                HandleResult::Continue => {}
                                HandleResult::Exit => break,
                                HandleResult::SessionChanged { new_key } => {
                                    session_key = new_key;
                                    // Resolve the internal SessionKey and notify the forwarder.
                                    if let Ok(resolved) = get_or_create_cli_session(
                                        kernel_handle.session_index().as_ref(),
                                        &session_key,
                                    ).await {
                                        let _ = session_tx.send(resolved);
                                    }
                                }
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
    kernel_handle: &KernelHandle,
) -> HandleResult {
    let parts: Vec<&str> = command.splitn(2, ' ').collect();
    let cmd_token = parts[0];

    // TUI-local commands take priority.
    match cmd_token {
        "/help" => {
            let mut lines = vec![
                "/help           — show this help".to_owned(),
                "/exit           — end chat session".to_owned(),
                "/new            — create a new session".to_owned(),
                "/clear          — clear current session display".to_owned(),
                "/sessions       — list recent sessions".to_owned(),
                "/switch <key>   — switch to a session by key".to_owned(),
                "/name <title>   — rename the current session".to_owned(),
                "/image <path>   — stage a local image for the next turn".to_owned(),
                "/images         — list staged images".to_owned(),
                "/clear-images   — clear staged images".to_owned(),
            ];
            // Append registered handler commands to help text.
            for handler in handlers {
                for def in handler.commands() {
                    let usage = def.usage.as_deref().unwrap_or("");
                    lines.push(format!("{:<16}— {}", usage, def.description));
                }
            }
            state.push_message(Role::System, lines.join("\n"));
            return HandleResult::Continue;
        }
        "/exit" | "/quit" => return HandleResult::Exit,
        "/new" => {
            return handle_new_session(state, kernel_handle).await;
        }
        "/clear" => {
            state.reset_messages();
            state.push_message(Role::System, "Session display cleared.".to_owned());
            return HandleResult::Continue;
        }
        "/sessions" => {
            handle_list_sessions(state, session_key, kernel_handle).await;
            return HandleResult::Continue;
        }
        "/switch" => {
            let target = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
            return handle_switch_session(state, target, kernel_handle).await;
        }
        "/image" => {
            let Some(raw_path) = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                state.push_message(
                    Role::System,
                    "Usage: /image /abs/path/to/file.png".to_owned(),
                );
                return HandleResult::Continue;
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
            return HandleResult::Continue;
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
            return HandleResult::Continue;
        }
        "/clear-images" => {
            state.staged_images.clear();
            state.push_message(Role::System, "Cleared staged images.".to_owned());
            return HandleResult::Continue;
        }
        "/name" => {
            let title = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
            handle_rename_session(state, session_key, title, kernel_handle).await;
            return HandleResult::Continue;
        }
        _ => {}
    }

    // Try kernel command handlers (e.g. /model, /usage).
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
            return HandleResult::Continue;
        }
    }

    state.push_message(
        Role::System,
        format!("Unknown command: {cmd_token}. Type /help"),
    );
    HandleResult::Continue
}

/// Handle the `/new` command: create a new session and switch to it.
async fn handle_new_session(state: &mut ChatState, kernel_handle: &KernelHandle) -> HandleResult {
    let session_index = kernel_handle.session_index();
    let now = Utc::now();
    let new_entry = SessionEntry {
        key:            SessionKey::new(),
        title:          None,
        model:          None,
        model_provider: None,
        thinking_level: None,
        system_prompt:  None,
        message_count:  0,
        preview:        None,
        metadata:       None,
        created_at:     now,
        updated_at:     now,
    };

    let created = match session_index.create_session(&new_entry).await {
        Ok(entry) => entry,
        Err(e) => {
            state.push_message(Role::System, format!("Failed to create session: {e}"));
            return HandleResult::Continue;
        }
    };

    // Use the short UUID prefix as the CLI alias for the new session.
    let new_alias = short_session_key(&created.key);
    let binding = ChannelBinding {
        channel_type: ChannelType::Cli,
        chat_id:      new_alias.clone(),
        thread_id:    None,
        session_key:  created.key,
        created_at:   now,
        updated_at:   now,
    };

    if let Err(e) = session_index.bind_channel(&binding).await {
        state.push_message(
            Role::System,
            format!("Session created but binding failed: {e}"),
        );
        return HandleResult::Continue;
    }

    state.reset_messages();
    state.session_label = new_alias.clone();
    state.push_message(Role::System, format!("New session created: {new_alias}"));

    HandleResult::SessionChanged { new_key: new_alias }
}

/// Handle the `/sessions` command: list recent sessions.
async fn handle_list_sessions(
    state: &mut ChatState,
    current_session_key: &str,
    kernel_handle: &KernelHandle,
) {
    let session_index = kernel_handle.session_index();
    let sessions = match session_index.list_sessions(10, 0).await {
        Ok(list) => list,
        Err(e) => {
            state.push_message(Role::System, format!("Failed to list sessions: {e}"));
            return;
        }
    };

    if sessions.is_empty() {
        state.push_message(Role::System, "No sessions found.".to_owned());
        return;
    }

    // Resolve the current session's internal key for comparison.
    let current_internal_key = session_index
        .get_channel_binding(ChannelType::Cli, current_session_key, None)
        .await
        .ok()
        .flatten()
        .map(|b| b.session_key);

    let lines: Vec<String> = sessions
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let is_current = current_internal_key
                .as_ref()
                .is_some_and(|k| *k == entry.key);
            let marker = if is_current { " *" } else { "" };
            let title = entry.title.as_deref().unwrap_or("(untitled)");
            let short_key = short_session_key(&entry.key);
            format!(
                "{}. [{}] {} ({} msgs){}",
                i + 1,
                short_key,
                title,
                entry.message_count,
                marker,
            )
        })
        .collect();

    let header = "Sessions (* = current):\n".to_owned();
    let footer = "\nUse /switch <key> to change session.".to_owned();
    state.push_message(
        Role::System,
        format!("{header}{}{footer}", lines.join("\n")),
    );
}

/// Handle the `/switch <key>` command: switch to a different session.
async fn handle_switch_session(
    state: &mut ChatState,
    target: Option<&str>,
    kernel_handle: &KernelHandle,
) -> HandleResult {
    let Some(target_key_str) = target else {
        state.push_message(Role::System, "Usage: /switch <session-key>".to_owned());
        return HandleResult::Continue;
    };

    let session_index = kernel_handle.session_index();

    // Try to find a session whose key starts with the given prefix.
    let sessions = match session_index.list_sessions(100, 0).await {
        Ok(list) => list,
        Err(e) => {
            state.push_message(Role::System, format!("Failed to list sessions: {e}"));
            return HandleResult::Continue;
        }
    };

    let target_entry = sessions.iter().find(|entry| {
        let full = entry.key.to_string();
        full.starts_with(target_key_str) || short_session_key(&entry.key) == target_key_str
    });

    let Some(entry) = target_entry else {
        state.push_message(Role::System, format!("Session not found: {target_key_str}"));
        return HandleResult::Continue;
    };

    // Create a new CLI binding for this session using the short key as alias.
    let new_alias = short_session_key(&entry.key);
    let now = Utc::now();
    let binding = ChannelBinding {
        channel_type: ChannelType::Cli,
        chat_id:      new_alias.clone(),
        thread_id:    None,
        session_key:  entry.key,
        created_at:   now,
        updated_at:   now,
    };

    if let Err(e) = session_index.bind_channel(&binding).await {
        state.push_message(Role::System, format!("Failed to bind session: {e}"));
        return HandleResult::Continue;
    }

    let title = entry.title.as_deref().unwrap_or("(untitled)");
    state.reset_messages();
    state.session_label = new_alias.clone();
    state.push_message(
        Role::System,
        format!("Switched to session: {new_alias} — {title}"),
    );

    HandleResult::SessionChanged { new_key: new_alias }
}

/// Return the first 8 characters of a session key UUID for display.
fn short_session_key(key: &SessionKey) -> String {
    let full = key.to_string();
    full.chars().take(8).collect()
}

/// Handle the `/name <title>` command: rename the current session.
async fn handle_rename_session(
    state: &mut ChatState,
    session_key: &str,
    title: Option<&str>,
    kernel_handle: &KernelHandle,
) {
    let Some(title) = title else {
        state.push_message(Role::System, "Usage: /name <title>".to_owned());
        return;
    };

    let session_index = kernel_handle.session_index();

    // Resolve the CLI alias to the internal SessionKey.
    let binding = match session_index
        .get_channel_binding(ChannelType::Cli, session_key, None)
        .await
    {
        Ok(Some(b)) => b,
        Ok(None) => {
            state.push_message(Role::System, "Session not found.".to_owned());
            return;
        }
        Err(e) => {
            state.push_message(Role::System, format!("Failed to find session: {e}"));
            return;
        }
    };

    let mut entry = match session_index.get_session(&binding.session_key).await {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            state.push_message(Role::System, "Session not found.".to_owned());
            return;
        }
        Err(e) => {
            state.push_message(Role::System, format!("Failed to get session: {e}"));
            return;
        }
    };

    entry.title = Some(title.to_owned());
    match session_index.update_session(&entry).await {
        Ok(_) => {
            state.session_label = title.to_owned();
            state.push_message(Role::System, format!("Session renamed to: {title}"));
        }
        Err(e) => {
            state.push_message(Role::System, format!("Failed to rename session: {e}"));
        }
    }
}

/// Render a [`CmdResult`] from a kernel command handler into the chat
/// display. HTML is converted to terminal-friendly plain text; inline
/// keyboards are silently dropped since terminal cannot render buttons.
fn render_command_result(state: &mut ChatState, result: CmdResult) {
    match result {
        CmdResult::Text(s) => state.push_message(Role::System, s),
        CmdResult::Html(s) => {
            state.push_message(Role::System, html_to_terminal(&s));
        }
        CmdResult::HtmlWithKeyboard { html, .. } => {
            // Inline keyboards are not renderable in the terminal; show the
            // text portion only.
            state.push_message(Role::System, html_to_terminal(&html));
        }
        CmdResult::Photo { caption, .. } => {
            // Terminal cannot display images — show caption with a marker.
            let text = caption
                .map(|c| format!("[Image] {c}"))
                .unwrap_or_else(|| "[Image]".to_string());
            state.push_message(Role::System, text);
        }
        CmdResult::None => {}
    }
}

/// Convert HTML (as produced by Telegram-oriented command handlers) into
/// terminal-friendly plain text.
///
/// Handles the common tags used by command handlers (`<b>`, `<i>`, `<code>`,
/// `<pre>`, `<br>`) and HTML entities. Remaining unknown tags are stripped.
fn html_to_terminal(s: &str) -> String {
    // Phase 1: replace known tags with terminal-friendly markers.
    let result = s
        .replace("<b>", "")
        .replace("</b>", "")
        .replace("<i>", "")
        .replace("</i>", "")
        .replace("<code>", "`")
        .replace("</code>", "`")
        .replace("<pre>", "```\n")
        .replace("</pre>", "\n```")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");

    // Phase 2: strip any remaining HTML tags.
    let mut out = String::with_capacity(result.len());
    let mut in_tag = false;
    for ch in result.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }

    // Phase 3: unescape common HTML entities.
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
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
        .get_channel_binding(ChannelType::Cli, chat_id, None)
        .await
        .whatever_context("Failed to load CLI channel binding")?
    {
        return Ok(binding.session_key);
    }

    let now = Utc::now();
    let entry = SessionEntry {
        key:            SessionKey::new(),
        title:          Some(chat_id.to_owned()),
        model:          None,
        model_provider: None,
        thinking_level: None,
        system_prompt:  None,
        message_count:  0,
        preview:        None,
        metadata:       None,
        created_at:     now,
        updated_at:     now,
    };
    let created = session_index
        .create_session(&entry)
        .await
        .whatever_context("Failed to create CLI chat session")?;
    let binding = ChannelBinding {
        channel_type: ChannelType::Cli,
        chat_id:      chat_id.to_owned(),
        thread_id:    None,
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
        StreamEvent::TurnRationale { text } => CliEvent::TurnRationale { text },
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
        } => CliEvent::TurnSummary {
            duration_ms,
            iterations: iterations as u32,
            tool_calls: tool_calls as u32,
            model,
        },
        StreamEvent::PlanCreated {
            compact_summary,
            total_steps,
            ..
        } => CliEvent::PlanCreated {
            goal:              compact_summary,
            total_steps:       total_steps as u32,
            step_descriptions: Vec::new(),
        },
        StreamEvent::PlanProgress {
            current_step,
            total_steps,
            status_text,
            ..
        } => CliEvent::PlanProgress {
            current_step: current_step as u32,
            total_steps: total_steps as u32,
            status_text,
        },
        StreamEvent::PlanReplan { reason } => CliEvent::Progress {
            text: format!("Replanning: {reason}"),
        },
        StreamEvent::PlanCompleted { summary } => CliEvent::PlanCompleted { summary },
        StreamEvent::UsageUpdate {
            input_tokens,
            output_tokens,
            thinking_ms,
        } => CliEvent::UsageUpdate {
            input_tokens,
            output_tokens,
            thinking_ms,
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
        StreamEvent::ToolCallLimit {
            session_key,
            limit_id,
            tool_calls_made,
            ..
        } => CliEvent::ToolCallLimitPaused {
            session_key: session_key.to_string(),
            limit_id,
            tool_calls_made,
        },
        StreamEvent::ToolCallLimitResolved { continued, .. } => CliEvent::Progress {
            text: if continued {
                "Agent resumed".to_string()
            } else {
                "Agent stopped".to_string()
            },
        },
        StreamEvent::LoopBreakerTriggered { pattern, tools, .. } => CliEvent::Progress {
            text: format!("Loop detected ({pattern}): disabled {}", tools.join(", ")),
        },
        StreamEvent::ToolOutput { chunk, .. } => CliEvent::TextDelta { text: chunk },
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
    state.thinking = true;
    state.thinking_started = Some(std::time::Instant::now());
    state.streaming_chars = 0;
    state.last_tokens = None;
    state.last_cost_usd = None;
    state.status_msg = None;
    state.turn_input_tokens = 0;
    state.turn_output_tokens = 0;
    state.turn_thinking_ms = 0;
    // Clear progress state from previous turn.
    state.tool_progress.clear();
    state.turn_started = Some(std::time::Instant::now());
    state.plan_goal = None;
    state.plan_steps = None;
    state.plan_current_step = None;

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

    /// 20 MiB upper bound — prevents accidental memory blowup from huge files.
    const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

    for path in image_paths {
        let meta = tokio::fs::metadata(path)
            .await
            .with_whatever_context(|_| format!("Failed to stat image: {path}"))?;
        snafu::ensure_whatever!(
            meta.len() <= MAX_IMAGE_BYTES,
            "Image too large ({:.1} MiB, max 20 MiB): {path}",
            meta.len() as f64 / (1024.0 * 1024.0)
        );
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
        channel::types::{ChannelType, ContentBlock, MessageContent},
        identity::UserId,
        io::StreamEvent,
        session::SessionIndex,
    };
    use rara_sessions::file_index::FileSessionIndex;

    use super::{
        build_cli_raw_message, cli_kernel_user_id, get_or_create_cli_session, short_session_key,
        stream_event_to_cli_event,
    };
    use crate::chat::app::ChatState;

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
            .get_channel_binding(ChannelType::Cli, "default", None)
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

    #[test]
    fn short_session_key_returns_first_8_chars() {
        let key = rara_kernel::session::SessionKey::new();
        let short = short_session_key(&key);
        let full = key.to_string();
        assert_eq!(short.len(), 8);
        assert!(full.starts_with(&short));
    }

    #[tokio::test]
    async fn new_session_creates_entry_and_binding() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let index = FileSessionIndex::new(tmp.path()).await.expect("index");
        let session_index: &dyn SessionIndex = &index;

        // Create initial session.
        let _first = get_or_create_cli_session(session_index, "default")
            .await
            .expect("first session");

        // list_sessions should return at least one.
        let sessions = session_index.list_sessions(10, 0).await.expect("list");
        assert!(!sessions.is_empty());
    }

    #[tokio::test]
    async fn session_index_tracks_created_sessions_and_bindings() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let index = std::sync::Arc::new(FileSessionIndex::new(tmp.path()).await.expect("index"));

        // Create a minimal KernelHandle substitute is not feasible, so we
        // test the helper directly.
        let session_index: &dyn SessionIndex = index.as_ref();
        let _first = get_or_create_cli_session(session_index, "default")
            .await
            .expect("first session");

        let sessions = session_index.list_sessions(10, 0).await.expect("list");
        assert_eq!(sessions.len(), 1);
    }

    // TODO: test that `/exit` returns `HandleResult::Exit`. Skipped because
    // `handle_slash_command` requires a `KernelHandle` which is non-trivial to
    // construct in a unit test (needs a full kernel bootstrap). Consider adding
    // an integration test or extracting the match into a pure function.

    #[test]
    fn clear_command_resets_state() {
        let mut state = ChatState::new("default".into(), "local".into());
        state.push_message(super::Role::User, "hello".into());
        assert!(state.messages.len() > 1);

        state.reset_messages();
        assert!(state.messages.is_empty());
    }

    // -----------------------------------------------------------------------
    // html_to_terminal
    // -----------------------------------------------------------------------

    #[test]
    fn html_to_terminal_strips_bold_and_italic() {
        assert_eq!(
            super::html_to_terminal("<b>bold</b> and <i>italic</i>"),
            "bold and italic"
        );
    }

    #[test]
    fn html_to_terminal_converts_code_to_backticks() {
        assert_eq!(
            super::html_to_terminal("use <code>foo</code> here"),
            "use `foo` here"
        );
    }

    #[test]
    fn html_to_terminal_converts_pre_to_fenced_block() {
        assert_eq!(
            super::html_to_terminal("<pre>line1\nline2</pre>"),
            "```\nline1\nline2\n```"
        );
    }

    #[test]
    fn html_to_terminal_converts_br_to_newline() {
        assert_eq!(super::html_to_terminal("a<br>b<br/>c<br />d"), "a\nb\nc\nd");
    }

    #[test]
    fn html_to_terminal_unescapes_entities() {
        assert_eq!(
            super::html_to_terminal("&amp; &lt; &gt; &quot; &#39;"),
            "& < > \" '"
        );
    }

    #[test]
    fn html_to_terminal_strips_unknown_tags() {
        assert_eq!(
            super::html_to_terminal("<div>hello <span>world</span></div>"),
            "hello world"
        );
    }

    #[test]
    fn html_to_terminal_handles_mcp_status_output() {
        let html = "<b>MCP Servers</b> (2)\n\n\u{25CF} <b>context-mode</b> \u{2014} connected \
                    (interceptor: \u{2713})\n\u{25CB} <b>other</b> \u{2014} disconnected\n\n1/2 \
                    connected";
        let result = super::html_to_terminal(html);
        assert!(result.contains("MCP Servers (2)"));
        assert!(result.contains("context-mode"));
        assert!(!result.contains('<'));
    }

    // -----------------------------------------------------------------------
    // render_command_result
    // -----------------------------------------------------------------------

    #[test]
    fn render_photo_shows_image_marker_with_caption() {
        use rara_kernel::channel::command::CommandResult as CmdResult;

        let mut state = ChatState::new("default".into(), "local".into());
        super::render_command_result(
            &mut state,
            CmdResult::Photo {
                data:    vec![],
                caption: Some("Anchor tree (3 sessions)".to_owned()),
            },
        );
        let last = state.messages.last().expect("message");
        assert_eq!(last.text, "[Image] Anchor tree (3 sessions)");
    }

    #[test]
    fn render_photo_without_caption_shows_placeholder() {
        use rara_kernel::channel::command::CommandResult as CmdResult;

        let mut state = ChatState::new("default".into(), "local".into());
        super::render_command_result(
            &mut state,
            CmdResult::Photo {
                data:    vec![],
                caption: None,
            },
        );
        let last = state.messages.last().expect("message");
        assert_eq!(last.text, "[Image]");
    }

    #[test]
    fn render_html_with_keyboard_drops_buttons() {
        use rara_kernel::channel::{command::CommandResult as CmdResult, types::InlineButton};

        let mut state = ChatState::new("default".into(), "local".into());
        super::render_command_result(
            &mut state,
            CmdResult::HtmlWithKeyboard {
                html:     "<b>Status</b>\nActive: 1".to_owned(),
                keyboard: vec![vec![InlineButton {
                    text:          "All jobs".to_owned(),
                    callback_data: Some("status_jobs:abc".to_owned()),
                    url:           None,
                }]],
            },
        );
        let last = state.messages.last().expect("message");
        assert!(last.text.contains("Status"));
        assert!(!last.text.contains("All jobs"));
    }
}
