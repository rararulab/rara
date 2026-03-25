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

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rara_channels::terminal::CliEvent;

use crate::chat::theme;

pub const CHAT_BANNER: &str = "/help for commands • /exit to quit";

/// Result of handling a slash command in the TUI.
///
/// Returned by [`handle_slash_command`](super::handle_slash_command) to tell
/// the main loop whether to continue, exit, or rebind to a new session.
pub enum HandleResult {
    /// Keep the current session and continue the event loop.
    Continue,
    /// The user requested exit (`/exit` or `/quit`).
    Exit,
    /// The user switched to a different session (via `/new` or `/switch`).
    SessionChanged {
        /// The new session key string (CLI alias) to use.
        new_key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInfo {
    pub name:     String,
    pub input:    String,
    pub result:   String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Agent,
    System,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
    pub tool: Option<ToolInfo>,
}

pub struct ChatState {
    pub agent_name:              String,
    pub model_label:             String,
    pub mode_label:              String,
    pub session_label:           String,
    pub user_label:              String,
    pub messages:                Vec<ChatMessage>,
    pub streaming_text:          String,
    pub is_streaming:            bool,
    pub thinking:                bool,
    pub active_tool:             Option<String>,
    pub spinner_frame:           usize,
    pub input:                   String,
    pub scroll_offset:           u16,
    pub last_tokens:             Option<(u64, u64)>,
    pub last_cost_usd:           Option<f64>,
    pub streaming_chars:         usize,
    pub status_msg:              Option<String>,
    pub staged_queue:            Vec<(String, Vec<String>)>,
    pub staged_images:           Vec<String>,
    pub tool_input_buf:          String,
    /// Cached loading hint, sampled once when entering thinking state to avoid
    /// flicker on every render tick.
    pub loading_hint:            String,
    /// Cumulative input tokens for the current turn (latest iteration's context
    /// size).
    pub turn_input_tokens:       u32,
    /// Cumulative output tokens for the current turn.
    pub turn_output_tokens:      u32,
    /// Cumulative thinking time in milliseconds for the current turn.
    pub turn_thinking_ms:        u64,
    /// Guard approval request awaiting user decision (y/n).
    pub pending_approval:        Option<PendingApproval>,
    /// Agent question awaiting user answer (free-form text input).
    pub pending_question:        Option<PendingQuestion>,
    /// Tool call limit pause awaiting user decision (continue/stop).
    pub pending_tool_call_limit: Option<PendingToolCallLimit>,
}

/// A guard approval request pending user decision.
#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub id:         String,
    pub tool_name:  String,
    pub summary:    String,
    pub risk_level: String,
}

/// A question from the agent pending user answer.
#[derive(Debug, Clone)]
pub struct PendingQuestion {
    pub id:       String,
    pub question: String,
}

/// A tool call limit pause pending user decision.
#[derive(Debug, Clone)]
pub struct PendingToolCallLimit {
    pub session_key:     String,
    pub limit_id:        u64,
    pub tool_calls_made: usize,
}

pub enum ChatAction {
    Continue,
    SendMessage(String),
    Back,
    SlashCommand(String),
    /// User approved a pending guard request.
    ApproveGuard {
        id: String,
    },
    /// User denied a pending guard request.
    DenyGuard {
        id: String,
    },
    /// User answered a pending agent question.
    AnswerQuestion {
        id:     String,
        answer: String,
    },
    /// User resolved the tool call limit pause.
    ResolveToolCallLimit {
        session_key: String,
        limit_id:    u64,
        continued:   bool,
    },
}

impl ChatState {
    #[must_use]
    pub fn new(session: String, user_id: String) -> Self {
        let mut state = Self {
            agent_name:              "rara".to_owned(),
            model_label:             "default".to_owned(),
            mode_label:              "in-process".to_owned(),
            session_label:           session,
            user_label:              user_id,
            messages:                Vec::new(),
            streaming_text:          String::new(),
            is_streaming:            false,
            thinking:                false,
            active_tool:             None,
            spinner_frame:           0,
            input:                   String::new(),
            scroll_offset:           0,
            last_tokens:             None,
            last_cost_usd:           None,
            streaming_chars:         0,
            status_msg:              None,
            staged_queue:            Vec::new(),
            staged_images:           Vec::new(),
            tool_input_buf:          String::new(),
            loading_hint:            String::new(),
            turn_input_tokens:       0,
            turn_output_tokens:      0,
            turn_thinking_ms:        0,
            pending_approval:        None,
            pending_question:        None,
            pending_tool_call_limit: None,
        };
        state.push_message(Role::System, CHAT_BANNER.to_owned());
        state
    }

    pub fn reset_messages(&mut self) {
        self.messages.clear();
        self.streaming_text.clear();
        self.is_streaming = false;
        self.thinking = false;
        self.active_tool = None;
        self.spinner_frame = 0;
        self.input.clear();
        self.scroll_offset = 0;
        self.last_tokens = None;
        self.last_cost_usd = None;
        self.streaming_chars = 0;
        self.status_msg = None;
        self.staged_queue.clear();
        self.staged_images.clear();
        self.tool_input_buf.clear();
        self.loading_hint.clear();
        self.turn_input_tokens = 0;
        self.turn_output_tokens = 0;
        self.turn_thinking_ms = 0;
        self.pending_approval = None;
        self.pending_question = None;
        self.pending_tool_call_limit = None;
    }

    /// Set a pending guard approval request for the user to decide on.
    pub fn set_pending_approval(&mut self, approval: PendingApproval) {
        self.pending_approval = Some(approval);
    }

    /// Set a pending agent question for the user to answer.
    pub fn set_pending_question(&mut self, question: PendingQuestion) {
        self.pending_question = Some(question);
    }

    pub fn push_message(&mut self, role: Role, text: String) {
        self.messages.push(ChatMessage {
            role,
            text,
            tool: None,
        });
        self.scroll_offset = 0;
    }

    pub fn append_stream(&mut self, text: &str) {
        self.thinking = false;
        self.streaming_text.push_str(text);
        self.streaming_chars += text.len();
        self.scroll_offset = 0;
    }

    pub fn take_staged(&mut self) -> Option<(String, Vec<String>)> {
        if self.staged_queue.is_empty() {
            None
        } else {
            Some(self.staged_queue.remove(0))
        }
    }

    pub fn finalize_stream(&mut self) {
        if !self.streaming_text.is_empty() {
            let text = sanitize_function_tags(&std::mem::take(&mut self.streaming_text));
            self.push_message(Role::Agent, text);
        }
        self.is_streaming = false;
        self.thinking = false;
        self.active_tool = None;
        self.streaming_chars = 0;
        self.tool_input_buf.clear();
    }

    pub fn tool_start(&mut self, name: &str) {
        self.active_tool = Some(name.to_owned());
        self.tool_input_buf.clear();
        self.spinner_frame = 0;
    }

    pub fn tool_use_end(&mut self, name: &str, input: &str) {
        self.messages.push(ChatMessage {
            role: Role::Tool,
            text: name.to_owned(),
            tool: Some(ToolInfo {
                name:     name.to_owned(),
                input:    input.to_owned(),
                result:   String::new(),
                is_error: false,
            }),
        });
        self.active_tool = None;
        self.tool_input_buf.clear();
        self.scroll_offset = 0;
    }

    pub fn tool_result(&mut self, name: &str, result: &str, is_error: bool) {
        for message in self.messages.iter_mut().rev() {
            if message.role != Role::Tool {
                continue;
            }
            let Some(tool) = message.tool.as_mut() else {
                continue;
            };
            if tool.name == name && tool.result.is_empty() {
                tool.result = result.to_owned();
                tool.is_error = is_error;
                break;
            }
        }
        self.active_tool = None;
        self.tool_input_buf.clear();
        self.scroll_offset = 0;
    }

    pub fn tick(&mut self) {
        if self.active_tool.is_some() || self.thinking {
            self.spinner_frame = (self.spinner_frame + 1) % theme::SPINNER_FRAMES.len();
        }
    }

    pub fn handle_cli_event(&mut self, event: CliEvent) {
        match event {
            CliEvent::Reply { content } => {
                let is_duplicate = self
                    .messages
                    .last()
                    .is_some_and(|message| message.role == Role::Agent && message.text == content);
                if !content.is_empty() && !is_duplicate {
                    self.push_message(Role::Agent, content);
                }
                self.is_streaming = false;
                self.thinking = false;
                self.status_msg = None;
            }
            CliEvent::TextDelta { text } => {
                self.is_streaming = true;
                self.append_stream(&text);
            }
            CliEvent::ReasoningDelta { text } => {
                self.is_streaming = true;
                if !self.thinking {
                    self.loading_hint = rara_kernel::io::loading_hints::random_hint().to_string();
                }
                self.thinking = true;
                self.append_stream(&text);
            }
            CliEvent::ToolCallStart { name, summary } => {
                if !self.streaming_text.is_empty() {
                    let text = std::mem::take(&mut self.streaming_text);
                    self.push_message(Role::Agent, text);
                }
                self.tool_start(&name);
                self.tool_use_end(&name, &summary);
            }
            CliEvent::ToolCallEnd {
                success,
                result_preview,
            } => {
                let tool_name = self.messages.iter().rev().find_map(|message| {
                    let tool = message.tool.as_ref()?;
                    if tool.result.is_empty() {
                        Some(tool.name.clone())
                    } else {
                        None
                    }
                });
                if let Some(tool_name) = tool_name {
                    self.tool_result(&tool_name, &result_preview, !success);
                }
            }
            CliEvent::Progress { text } => {
                self.status_msg = Some(text);
            }
            CliEvent::Error { message } => {
                self.is_streaming = false;
                self.thinking = false;
                self.active_tool = None;
                self.status_msg = Some(format!("Error: {message}"));
            }
            CliEvent::TurnRationale { text } => {
                self.status_msg = Some(text);
            }
            CliEvent::ApprovalRequest {
                id,
                tool_name,
                summary,
                risk_level,
            } => {
                self.set_pending_approval(PendingApproval {
                    id,
                    tool_name,
                    summary,
                    risk_level,
                });
            }
            CliEvent::UserQuestion { id, question } => {
                self.set_pending_question(PendingQuestion { id, question });
            }
            CliEvent::ToolCallLimitPaused {
                session_key,
                limit_id,
                tool_calls_made,
            } => {
                self.pending_tool_call_limit = Some(PendingToolCallLimit {
                    session_key,
                    limit_id,
                    tool_calls_made,
                });
                self.status_msg = Some(format!(
                    "Agent paused after {tool_calls_made} tool calls. [c] Continue  [s] Stop"
                ));
            }
            CliEvent::UsageUpdate {
                input_tokens,
                output_tokens,
                thinking_ms,
            } => {
                // input_tokens = latest iteration's context size (overwrite).
                // output_tokens = cumulative across iterations (overwrite).
                // thinking_ms = cumulative (overwrite).
                self.turn_input_tokens = input_tokens;
                self.turn_output_tokens = output_tokens;
                self.turn_thinking_ms = thinking_ms;
            }
            CliEvent::TurnSummary {
                duration_ms,
                iterations,
                tool_calls,
                model,
            } => {
                let duration = format_duration(duration_ms);
                let in_tok = format_tokens(u64::from(self.turn_input_tokens));
                let out_tok = format_tokens(u64::from(self.turn_output_tokens));
                let thinking = if self.turn_thinking_ms > 0 {
                    format!(" · thought {}s", self.turn_thinking_ms / 1000)
                } else {
                    String::new()
                };
                self.status_msg = Some(format!(
                    "done {duration} · {iterations} iter · {tool_calls} tools · ↑{in_tok} \
                     ↓{out_tok}{thinking} · {model}"
                ));
                // Reset for next turn.
                self.turn_input_tokens = 0;
                self.turn_output_tokens = 0;
                self.turn_thinking_ms = 0;
            }
            CliEvent::Done => self.finalize_stream(),
        }
    }

    #[must_use]
    pub fn handle_key(&mut self, key: KeyEvent) -> ChatAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return ChatAction::Back;
        }

        // Intercept y/n when a guard approval request is pending (highest priority).
        if let Some(approval) = &self.pending_approval {
            let id = approval.id.clone();
            return match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.pending_approval = None;
                    ChatAction::ApproveGuard { id }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.pending_approval = None;
                    ChatAction::DenyGuard { id }
                }
                _ => ChatAction::Continue,
            };
        }

        // When a question is pending, user types an answer into self.input.
        if let Some(question) = &self.pending_question {
            let id = question.id.clone();
            return match key.code {
                KeyCode::Enter => {
                    let answer = self.input.trim().to_owned();
                    self.input.clear();
                    if answer.is_empty() {
                        return ChatAction::Continue;
                    }
                    self.pending_question = None;
                    ChatAction::AnswerQuestion { id, answer }
                }
                KeyCode::Esc => {
                    self.input.clear();
                    self.pending_question = None;
                    ChatAction::AnswerQuestion {
                        id,
                        answer: "(no answer)".to_owned(),
                    }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.input.clear();
                    ChatAction::Continue
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.input.push(ch);
                    ChatAction::Continue
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    ChatAction::Continue
                }
                _ => ChatAction::Continue,
            };
        }

        // Tool call limit pause: c/Enter → continue, s/Esc → stop.
        if let Some(pending) = &self.pending_tool_call_limit {
            let session_key = pending.session_key.clone();
            let limit_id = pending.limit_id;
            return match key.code {
                KeyCode::Char('c') | KeyCode::Enter => {
                    self.pending_tool_call_limit = None;
                    ChatAction::ResolveToolCallLimit {
                        session_key,
                        limit_id,
                        continued: true,
                    }
                }
                KeyCode::Char('s') | KeyCode::Esc => {
                    self.pending_tool_call_limit = None;
                    ChatAction::ResolveToolCallLimit {
                        session_key,
                        limit_id,
                        continued: false,
                    }
                }
                _ => ChatAction::Continue,
            };
        }

        if self.is_streaming {
            return self.handle_streaming_key(key);
        }

        match key.code {
            KeyCode::Esc => ChatAction::Back,
            KeyCode::Enter => {
                let msg = self.input.trim().to_owned();
                self.input.clear();
                let has_images = !self.staged_images.is_empty();
                if msg.starts_with('/') {
                    return ChatAction::SlashCommand(msg);
                }
                if msg.is_empty() && !has_images {
                    return ChatAction::Continue;
                }
                if !msg.is_empty() {
                    self.push_message(Role::User, msg.clone());
                }
                ChatAction::SendMessage(msg)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                ChatAction::Continue
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.input.push(ch);
                ChatAction::Continue
            }
            KeyCode::Backspace => {
                self.input.pop();
                ChatAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                ChatAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                ChatAction::Continue
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
                ChatAction::Continue
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                ChatAction::Continue
            }
            _ => ChatAction::Continue,
        }
    }

    fn handle_streaming_key(&mut self, key: KeyEvent) -> ChatAction {
        match key.code {
            KeyCode::Esc => ChatAction::Back,
            KeyCode::Enter => {
                let msg = self.input.trim().to_owned();
                self.input.clear();
                if (!msg.is_empty() || !self.staged_images.is_empty()) && !msg.starts_with('/') {
                    self.staged_queue
                        .push((msg.clone(), std::mem::take(&mut self.staged_images)));
                    if msg.is_empty() {
                        self.status_msg = Some("Queued staged images for next turn".to_owned());
                    } else {
                        self.push_message(Role::User, msg);
                    }
                }
                ChatAction::Continue
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                ChatAction::Continue
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.input.push(ch);
                ChatAction::Continue
            }
            KeyCode::Backspace => {
                self.input.pop();
                ChatAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                ChatAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                ChatAction::Continue
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
                ChatAction::Continue
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                ChatAction::Continue
            }
            _ => ChatAction::Continue,
        }
    }
}

/// Format a duration in milliseconds into a human-readable string.
fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{:.1}m", ms as f64 / 60_000.0)
    }
}

/// Format a token count into a compact human-readable string (e.g. "1.2k").
fn format_tokens(tokens: u64) -> String {
    if tokens < 1000 {
        format!("{tokens}")
    } else if tokens < 1_000_000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

fn sanitize_function_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<function>") {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find("</function>") {
            rest = &rest[start + end + "</function>".len()..];
        } else {
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use rara_channels::terminal::CliEvent;

    use super::{
        ChatAction, ChatState, PendingApproval, PendingQuestion, PendingToolCallLimit, Role,
    };

    #[test]
    fn new_chat_state_starts_with_openfang_banner() {
        let chat = ChatState::new("default".into(), "local".into());

        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, Role::System);
        assert_eq!(chat.messages[0].text, "/help for commands • /exit to quit");
    }

    #[test]
    fn text_delta_promotes_agent_message_on_done() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.handle_cli_event(CliEvent::TextDelta { text: "hi".into() });
        chat.handle_cli_event(CliEvent::Done);

        assert_eq!(
            chat.messages.last().map(|message| message.role),
            Some(Role::Agent)
        );
        assert_eq!(
            chat.messages.last().map(|message| message.text.as_str()),
            Some("hi")
        );
    }

    #[test]
    fn tool_events_create_embedded_tool_message() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.handle_cli_event(CliEvent::ToolCallStart {
            name:    "read_file".into(),
            summary: "README.md".into(),
        });

        let tool = chat
            .messages
            .last()
            .and_then(|message| message.tool.as_ref());
        assert!(tool.is_some());
        let tool = tool.expect("tool info");
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.input, "README.md");
        assert_eq!(tool.result, "");
        assert!(!tool.is_error);

        chat.handle_cli_event(CliEvent::ToolCallEnd {
            success:        true,
            result_preview: "contents".into(),
        });

        let tool = chat
            .messages
            .last()
            .and_then(|message| message.tool.as_ref());
        assert!(tool.is_some());
        let tool = tool.expect("tool info");
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.input, "README.md");
        assert_eq!(tool.result, "contents");
        assert!(!tool.is_error);
    }

    #[test]
    fn finalize_stream_strips_function_tags() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.handle_cli_event(CliEvent::TextDelta {
            text: "hello<function>hidden</function>world".into(),
        });
        chat.handle_cli_event(CliEvent::Done);

        assert_eq!(
            chat.messages.last().map(|message| message.text.as_str()),
            Some("helloworld")
        );
    }

    #[test]
    fn reply_after_stream_does_not_duplicate_last_agent_message() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.handle_cli_event(CliEvent::TextDelta {
            text: "same reply".into(),
        });
        chat.handle_cli_event(CliEvent::Done);
        chat.handle_cli_event(CliEvent::Reply {
            content: "same reply".into(),
        });

        let agent_messages: Vec<_> = chat
            .messages
            .iter()
            .filter(|message| message.role == Role::Agent)
            .collect();
        assert_eq!(agent_messages.len(), 1);
    }

    #[test]
    fn streaming_input_is_staged_not_sent_immediately() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.is_streaming = true;
        chat.input = "next".into();

        let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::Continue));
        assert_eq!(chat.staged_queue.len(), 1);
        assert_eq!(chat.staged_queue[0].0, "next");
        assert_eq!(
            chat.messages.last().map(|message| message.role),
            Some(Role::User)
        );
    }

    #[test]
    fn slash_command_is_returned_when_idle() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.input = "/clear".into();

        let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::SlashCommand(cmd) if cmd == "/clear"));
    }

    #[test]
    fn enter_without_text_sends_when_images_are_staged() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.staged_images.push("/tmp/cat.png".into());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::SendMessage(text) if text.is_empty()));
    }

    #[test]
    fn slash_command_still_wins_when_images_are_staged() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.staged_images.push("/tmp/cat.png".into());
        chat.input = "/images".into();

        let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::SlashCommand(cmd) if cmd == "/images"));
    }

    fn make_pending_approval() -> PendingApproval {
        PendingApproval {
            id:         "550e8400-e29b-41d4-a716-446655440000".into(),
            tool_name:  "bash".into(),
            summary:    "rm -rf /tmp/old".into(),
            risk_level: "Critical".into(),
        }
    }

    #[test]
    fn y_key_approves_pending_guard_request() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_approval(make_pending_approval());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

        assert!(
            matches!(action, ChatAction::ApproveGuard { id } if id == "550e8400-e29b-41d4-a716-446655440000")
        );
        assert!(chat.pending_approval.is_none());
    }

    #[test]
    fn enter_approves_pending_guard_request() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_approval(make_pending_approval());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(
            matches!(action, ChatAction::ApproveGuard { id } if id == "550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn n_key_denies_pending_guard_request() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_approval(make_pending_approval());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));

        assert!(
            matches!(action, ChatAction::DenyGuard { id } if id == "550e8400-e29b-41d4-a716-446655440000")
        );
        assert!(chat.pending_approval.is_none());
    }

    #[test]
    fn esc_denies_pending_guard_request() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_approval(make_pending_approval());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(
            matches!(action, ChatAction::DenyGuard { id } if id == "550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn other_keys_ignored_when_approval_pending() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_approval(make_pending_approval());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::Continue));
        assert!(chat.pending_approval.is_some());
    }

    #[test]
    fn scrolling_uses_bottom_relative_offset() {
        let mut chat = ChatState::new("default".into(), "local".into());

        let _ = chat.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(chat.scroll_offset, 1);

        let _ = chat.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(chat.scroll_offset, 11);

        let _ = chat.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(chat.scroll_offset, 10);
    }

    fn make_pending_question() -> PendingQuestion {
        PendingQuestion {
            id:       "660e8400-e29b-41d4-a716-446655440000".into(),
            question: "What is the API key?".into(),
        }
    }

    #[test]
    fn user_question_event_sets_pending_question() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.handle_cli_event(CliEvent::UserQuestion {
            id:       "660e8400-e29b-41d4-a716-446655440000".into(),
            question: "What is the API key?".into(),
        });

        assert!(chat.pending_question.is_some());
        let q = chat.pending_question.as_ref().expect("pending question");
        assert_eq!(q.id, "660e8400-e29b-41d4-a716-446655440000");
        assert_eq!(q.question, "What is the API key?");
    }

    #[test]
    fn enter_submits_answer_when_question_pending() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_question(make_pending_question());
        chat.input = "sk-12345".into();

        let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::AnswerQuestion { id, answer }
                if id == "660e8400-e29b-41d4-a716-446655440000" && answer == "sk-12345"));
        assert!(chat.pending_question.is_none());
        assert!(chat.input.is_empty());
    }

    #[test]
    fn empty_enter_does_not_submit_question() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_question(make_pending_question());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::Continue));
        assert!(chat.pending_question.is_some());
    }

    #[test]
    fn esc_skips_question_with_no_answer() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_question(make_pending_question());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::AnswerQuestion { id, answer }
                if id == "660e8400-e29b-41d4-a716-446655440000" && answer == "(no answer)"));
        assert!(chat.pending_question.is_none());
    }

    #[test]
    fn typing_in_question_mode_appends_to_input() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_question(make_pending_question());

        let _ = chat.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let _ = chat.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));

        assert_eq!(chat.input, "ab");
        assert!(chat.pending_question.is_some());
    }

    #[test]
    fn backspace_in_question_mode_removes_char() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_question(make_pending_question());
        chat.input = "abc".into();

        let _ = chat.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        assert_eq!(chat.input, "ab");
    }

    #[test]
    fn approval_takes_priority_over_question() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_approval(make_pending_approval());
        chat.set_pending_question(make_pending_question());

        // 'y' should trigger approval, not question input.
        let action = chat.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::ApproveGuard { .. }));
        assert!(chat.pending_approval.is_none());
        // Question should still be pending.
        assert!(chat.pending_question.is_some());
    }

    #[test]
    fn reset_messages_clears_pending_question() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_question(make_pending_question());

        chat.reset_messages();

        assert!(chat.pending_question.is_none());
    }

    fn make_pending_tool_call_limit() -> PendingToolCallLimit {
        PendingToolCallLimit {
            session_key:     "550e8400-e29b-41d4-a716-446655440000".into(),
            limit_id:        1,
            tool_calls_made: 25,
        }
    }

    #[test]
    fn tool_call_limit_event_sets_pending_state() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.handle_cli_event(CliEvent::ToolCallLimitPaused {
            session_key:     "550e8400-e29b-41d4-a716-446655440000".into(),
            limit_id:        1,
            tool_calls_made: 25,
        });

        assert!(chat.pending_tool_call_limit.is_some());
        let p = chat.pending_tool_call_limit.as_ref().expect("pending");
        assert_eq!(p.limit_id, 1);
        assert_eq!(p.tool_calls_made, 25);
        assert!(chat.status_msg.as_ref().expect("status").contains("25"));
    }

    #[test]
    fn c_key_continues_tool_call_limit() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.pending_tool_call_limit = Some(make_pending_tool_call_limit());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));

        assert!(matches!(
            action,
            ChatAction::ResolveToolCallLimit {
                continued: true,
                limit_id: 1,
                ..
            }
        ));
        assert!(chat.pending_tool_call_limit.is_none());
    }

    #[test]
    fn enter_continues_tool_call_limit() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.pending_tool_call_limit = Some(make_pending_tool_call_limit());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(
            action,
            ChatAction::ResolveToolCallLimit {
                continued: true,
                limit_id: 1,
                ..
            }
        ));
    }

    #[test]
    fn s_key_stops_tool_call_limit() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.pending_tool_call_limit = Some(make_pending_tool_call_limit());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));

        assert!(matches!(
            action,
            ChatAction::ResolveToolCallLimit {
                continued: false,
                limit_id: 1,
                ..
            }
        ));
        assert!(chat.pending_tool_call_limit.is_none());
    }

    #[test]
    fn esc_stops_tool_call_limit() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.pending_tool_call_limit = Some(make_pending_tool_call_limit());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(matches!(
            action,
            ChatAction::ResolveToolCallLimit {
                continued: false,
                limit_id: 1,
                ..
            }
        ));
    }

    #[test]
    fn other_keys_ignored_when_tool_call_limit_pending() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.pending_tool_call_limit = Some(make_pending_tool_call_limit());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::Continue));
        assert!(chat.pending_tool_call_limit.is_some());
    }

    #[test]
    fn approval_takes_priority_over_tool_call_limit() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_approval(make_pending_approval());
        chat.pending_tool_call_limit = Some(make_pending_tool_call_limit());

        let action = chat.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::ApproveGuard { .. }));
        // Tool call limit should still be pending.
        assert!(chat.pending_tool_call_limit.is_some());
    }

    #[test]
    fn question_takes_priority_over_tool_call_limit() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.set_pending_question(make_pending_question());
        chat.pending_tool_call_limit = Some(make_pending_tool_call_limit());
        chat.input = "answer".into();

        let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(action, ChatAction::AnswerQuestion { .. }));
        // Tool call limit should still be pending.
        assert!(chat.pending_tool_call_limit.is_some());
    }

    #[test]
    fn reset_messages_clears_pending_tool_call_limit() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.pending_tool_call_limit = Some(make_pending_tool_call_limit());

        chat.reset_messages();

        assert!(chat.pending_tool_call_limit.is_none());
    }

    // -----------------------------------------------------------------------
    // Execution trace summary tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_duration_milliseconds() {
        assert_eq!(super::format_duration(0), "0ms");
        assert_eq!(super::format_duration(500), "500ms");
        assert_eq!(super::format_duration(999), "999ms");
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(super::format_duration(1000), "1.0s");
        assert_eq!(super::format_duration(1500), "1.5s");
        assert_eq!(super::format_duration(59_999), "60.0s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(super::format_duration(60_000), "1.0m");
        assert_eq!(super::format_duration(90_000), "1.5m");
    }

    #[test]
    fn format_tokens_plain() {
        assert_eq!(super::format_tokens(0), "0");
        assert_eq!(super::format_tokens(999), "999");
    }

    #[test]
    fn format_tokens_kilo() {
        assert_eq!(super::format_tokens(1000), "1.0k");
        assert_eq!(super::format_tokens(1500), "1.5k");
        assert_eq!(super::format_tokens(999_999), "1000.0k");
    }

    #[test]
    fn format_tokens_mega() {
        assert_eq!(super::format_tokens(1_000_000), "1.0M");
        assert_eq!(super::format_tokens(2_500_000), "2.5M");
    }

    #[test]
    fn usage_update_accumulates_in_state() {
        let mut chat = ChatState::new("default".into(), "local".into());

        // First iteration.
        chat.handle_cli_event(CliEvent::UsageUpdate {
            input_tokens:  1000,
            output_tokens: 200,
            thinking_ms:   500,
        });
        assert_eq!(chat.turn_input_tokens, 1000);
        assert_eq!(chat.turn_output_tokens, 200);
        assert_eq!(chat.turn_thinking_ms, 500);

        // Second iteration — values are cumulative, so overwrite.
        chat.handle_cli_event(CliEvent::UsageUpdate {
            input_tokens:  2000,
            output_tokens: 450,
            thinking_ms:   800,
        });
        assert_eq!(chat.turn_input_tokens, 2000);
        assert_eq!(chat.turn_output_tokens, 450);
        assert_eq!(chat.turn_thinking_ms, 800);
    }

    #[test]
    fn turn_summary_sets_status_and_resets_counters() {
        let mut chat = ChatState::new("default".into(), "local".into());

        // Simulate usage accumulation.
        chat.handle_cli_event(CliEvent::UsageUpdate {
            input_tokens:  5000,
            output_tokens: 1200,
            thinking_ms:   3000,
        });

        chat.handle_cli_event(CliEvent::TurnSummary {
            duration_ms: 4500,
            iterations:  3,
            tool_calls:  2,
            model:       "gpt-4".into(),
        });

        let status = chat.status_msg.as_ref().expect("status should be set");
        assert!(status.contains("4.5s"), "should contain formatted duration");
        assert!(status.contains("3 iter"), "should contain iteration count");
        assert!(status.contains("2 tools"), "should contain tool count");
        assert!(status.contains("5.0k"), "should contain input tokens");
        assert!(status.contains("1.2k"), "should contain output tokens");
        assert!(
            status.contains("thought 3s"),
            "should contain thinking time"
        );
        assert!(status.contains("gpt-4"), "should contain model name");

        // Counters should be reset.
        assert_eq!(chat.turn_input_tokens, 0);
        assert_eq!(chat.turn_output_tokens, 0);
        assert_eq!(chat.turn_thinking_ms, 0);
    }

    #[test]
    fn turn_summary_omits_thinking_when_zero() {
        let mut chat = ChatState::new("default".into(), "local".into());

        chat.handle_cli_event(CliEvent::TurnSummary {
            duration_ms: 1000,
            iterations:  1,
            tool_calls:  0,
            model:       "claude".into(),
        });

        let status = chat.status_msg.as_ref().expect("status should be set");
        assert!(!status.contains("thought"), "should not contain thinking");
    }

    #[test]
    fn reset_messages_clears_turn_metrics() {
        let mut chat = ChatState::new("default".into(), "local".into());
        chat.turn_input_tokens = 100;
        chat.turn_output_tokens = 200;
        chat.turn_thinking_ms = 300;

        chat.reset_messages();

        assert_eq!(chat.turn_input_tokens, 0);
        assert_eq!(chat.turn_output_tokens, 0);
        assert_eq!(chat.turn_thinking_ms, 0);
    }
}
