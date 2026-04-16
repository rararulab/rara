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

//! Telegram channel adapter.
//!
//! Implements [`ChannelAdapter`] using the Telegram Bot API via `getUpdates`
//! long polling. Inbound messages are converted to [`RawPlatformMessage`] and
//! handed to the [`KernelHandle`] in a fire-and-forget fashion. Outbound
//! delivery is handled by the [`ChannelAdapter`] implementation.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │         TelegramAdapter                 │
//! │                                         │
//! │  start() ─► spawn polling task          │
//! │              │                          │
//! │              ├── getUpdates (long poll)  │
//! │              │     │                    │
//! │              │     ├── Update → RawPlatformMessage
//! │              │     │     │              │
//! │              │     │     ▼              │
//! │              │     │  handle.ingest()    │
//! │              │     │                    │
//! │              │     └── loop             │
//! │              │                          │
//! │  stop()  ─► shutdown signal             │
//! └─────────────────────────────────────────┘
//! ```

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, RwLock as StdRwLock},
    time::Instant,
};

use tokio::task::AbortHandle;
use uuid::Uuid;

/// Tracks abort handles for guard approval expiry tasks so they can be
/// cancelled when the user resolves an approval before the timeout fires.
static GUARD_EXPIRY_HANDLES: LazyLock<DashMap<Uuid, AbortHandle>> = LazyLock::new(DashMap::new);

/// Pending user-question entry stored in [`PENDING_USER_QUESTIONS`].
struct PendingUserQuestion {
    question_id:               Uuid,
    question_text:             String,
    manager:                   UserQuestionManagerRef,
    /// Platform-native id of the user who asked — incoming answers are
    /// rejected unless `msg.from.id`/`callback.from.id` matches this value.
    /// `None` when the origin had no platform identity (web/cli).
    expected_platform_user_id: Option<String>,
    /// Pre-defined answer options. When `Some`, the prompt was rendered as
    /// an inline keyboard; callback resolution maps the pressed index into
    /// this list.
    options:                   Option<Vec<String>>,
    /// The `(chat_id, prompt_message_id)` where the prompt was rendered.
    /// Kept so the timeout cleanup task can remove both index entries.
    prompt_location:           (i64, i32),
}

/// Primary pending-question index keyed by the kernel-generated question
/// UUID, so both reply-to and inline-keyboard callbacks can find the entry
/// by a single stable identifier.
///
/// Entries are inserted by [`question_listener`] and removed by
/// `handle_update` (on resolve) or by the per-question timeout cleanup task.
static PENDING_USER_QUESTIONS: LazyLock<DashMap<Uuid, PendingUserQuestion>> =
    LazyLock::new(DashMap::new);

/// Secondary index: `(chat_id, prompt_message_id) → question_id`, used to
/// locate a pending question from a user's reply-to-message. Kept in sync
/// with [`PENDING_USER_QUESTIONS`].
static PROMPT_MSG_TO_QID: LazyLock<DashMap<(i64, i32), Uuid>> = LazyLock::new(DashMap::new);

/// Matches complete tool-call XML blocks (open + close, possibly mismatched
/// names).
static TOOL_CALL_BLOCK_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?si)<(?:toolcall|tool_call|tool_use|function=[^>]*)(?:\s[^>]*)?>.*?</(?:toolcall|tool_call|tool_use|function)>|<(?:toolcall|tool_call|tool_use|function=[^>]*)(?:\s[^>]*)?/>"
    )
    .expect("tool call block regex must compile")
});

/// Matches orphaned individual opening or closing tool-call tags.
///
/// Why orphaned tags happen during streaming:
/// The stream forwarder flushes accumulated text when it exceeds
/// `STREAM_SPLIT_THRESHOLD`. If `<toolcall>` arrives before the threshold
/// flush but `</tool_call>` arrives after, the opening tag is flushed and
/// `accumulated.clear()` discards it. The closing tag then arrives as an
/// orphan in the next batch. Neither tag matches `TOOL_CALL_BLOCK_RE`
/// because they're never in the same buffer together.
///
/// Additionally, when the LLM "degrades" and emits tool-call XML as plain
/// text (instead of using the structured tool-call API), the agent loop
/// sees `stop_reason != ToolCalls`, terminates immediately, and returns the
/// XML as a normal text response. The block regex may still miss fragments
/// if the XML is malformed (e.g., `<toolcall>...</tool_call>`-style
/// mismatches that span flush boundaries).
static TOOL_CALL_TAG_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)</?(?:toolcall|tool_call|tool_use|function=[^>]*)(?:\s[^>]*)?>")
        .expect("tool call tag regex must compile")
});

use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use rara_domain_shared::settings::SettingsProvider;
use rara_kernel::{
    channel::{
        adapter::ChannelAdapter,
        command::{
            CallbackHandler, CallbackResult, CommandContext, CommandHandler, CommandInfo,
            CommandResult,
        },
        types::{ChannelType, ChannelUser, GroupPolicy, InlineButton, MessageContent, ReplyMarkup},
    },
    error::KernelError,
    handle::KernelHandle,
    io::{
        EgressError, Endpoint, EndpointAddress, IOError, InteractionType, PlatformOutbound,
        RawPlatformMessage, ReplyContext, StreamHubRef,
    },
    security::{ApprovalDecision, ApprovalRequest, ResolveError},
    session::SessionIndexRef,
    user_question::{UserQuestion, UserQuestionManagerRef},
};
use teloxide::{
    payloads::{
        AnswerCallbackQuerySetters, EditForumTopicSetters, EditMessageTextSetters,
        GetUpdatesSetters, SendChatActionSetters, SendDocumentSetters, SendMessageSetters,
        SendPhotoSetters, SendVoiceSetters,
    },
    requests::{Request, Requester},
    types::{
        AllowedUpdate, ChatAction, ChatId, ChatKind, ChatPublic, InlineKeyboardButton,
        InlineKeyboardMarkup, MessageId, ParseMode, PublicChatKind, PublicChatSupergroup,
        ReplyParameters, Update, UpdateKind,
    },
};
use tokio::sync::{RwLock, watch};
use tracing::{debug, error, info, warn};

/// Convert an `Option<i64>` forum thread ID to teloxide's `ThreadId`.
///
/// Telegram forum topics use the root message ID of the topic as the thread
/// identifier. teloxide wraps this in `ThreadId(MessageId(i32))`.
#[allow(clippy::single_option_map)]
fn to_thread_id(thread_id: Option<i64>) -> Option<teloxide::types::ThreadId> {
    thread_id.map(|tid| {
        debug_assert!(i32::try_from(tid).is_ok(), "thread_id overflows i32: {tid}");
        #[allow(clippy::cast_possible_truncation)]
        teloxide::types::ThreadId(MessageId(tid as i32))
    })
}

/// Build a public `t.me/c/…` deep-link for a forum topic.
///
/// Private supergroups use chat IDs below `-1_000_000_000_000`; Telegram's
/// web client links use the "short" chat ID form which strips the `-100`
/// prefix. The returned URL opens the client directly in the given topic.
fn forum_topic_link(chat_id: i64, thread_id: i64) -> String {
    // Telegram supergroup IDs are always negative; make them positive and
    // strip the `100` prefix (1_000_000_000_000) to get the short form.
    let short = (-chat_id) - 1_000_000_000_000;
    format!("https://t.me/c/{short}/{thread_id}")
}

/// Derive a forum topic name from the user's first message text.
///
/// Strips noise that would make a bad topic label: the bot `@mention` and a
/// leading `/command` token. Truncates the result to 30 characters. Falls
/// back to `"New chat"` when the message has no usable text.
fn derive_initial_topic_name(text: Option<&str>, bot_username: Option<&str>) -> String {
    let Some(raw) = text.map(str::trim).filter(|s| !s.is_empty()) else {
        return "New chat".to_owned();
    };

    let without_mention = strip_group_mention(raw, bot_username);

    // Drop the leading `/command` token if present (e.g. `/new hello`
    // → `hello`). Only strip when the whole first whitespace-delimited
    // token starts with `/` so plain messages containing a mid-sentence
    // slash are preserved.
    let stripped = match without_mention.split_once(char::is_whitespace) {
        Some((head, rest)) if head.starts_with('/') => rest.trim_start().to_owned(),
        _ if without_mention.starts_with('/') => String::new(),
        _ => without_mention,
    };

    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return "New chat".to_owned();
    }

    trimmed.chars().take(30).collect()
}

/// Apply forum topic `thread_id` to a teloxide request builder if present.
///
/// Eliminates the repeated 3-line `if let Some(tid) = ...` pattern across
/// all send_message / send_chat_action / send_photo / send_document /
/// send_voice call sites (~18 occurrences).
macro_rules! with_thread_id {
    ($req:expr, $thread_id:expr) => {{
        let req = $req;
        if let Some(tid) = to_thread_id($thread_id) {
            req.message_thread_id(tid)
        } else {
            req
        }
    }};
}

/// Long-polling timeout in seconds (Telegram server-side wait).
const POLL_TIMEOUT_SECS: u32 = 30;

/// Groups with this many members or fewer are treated like private chats --
/// the bot responds to every message without requiring an @mention or keyword.
const SMALL_GROUP_THRESHOLD: u32 = 3;

/// Initial error retry delay.
const INITIAL_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Maximum retry delay for exponential backoff.
const MAX_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

/// Minimum interval between Telegram `edit_message_text` calls (1.5 seconds)
/// to avoid hitting Telegram API rate limits.
const MIN_EDIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

/// Maximum characters per Telegram message before splitting to a new message.
/// Set below 4096 to leave buffer for HTML tag expansion from markdown→html.
const STREAM_SPLIT_THRESHOLD: usize = 3800;

/// Build a [`teloxide::Bot`] with an optional proxy and a timeout suitable
/// for long polling.
///
/// This is a standalone helper so both the Telegram adapter and the gateway
/// can construct a properly configured `Bot` without duplicating the proxy /
/// timeout logic.
///
/// The proxy URL is passed to [`reqwest012::Proxy::all`] (supports `http://`,
/// `https://`, `socks5://`).
///
/// Note: `reqwest012` is reqwest 0.12, kept because teloxide 0.17 is pinned
/// to that major version and its `ClientBuilder` / `Proxy` types must match.
pub fn build_bot(token: &str, proxy: Option<&str>) -> Result<teloxide::Bot, anyhow::Error> {
    match proxy {
        Some(url) => {
            let client = teloxide::net::default_reqwest_settings()
                .proxy(reqwest012::Proxy::all(url)?)
                .timeout(std::time::Duration::from_secs(
                    POLL_TIMEOUT_SECS as u64 + 30,
                ))
                .build()?;
            Ok(teloxide::Bot::with_client(token, client))
        }
        None => Ok(teloxide::Bot::new(token)),
    }
}

/// Single tool's progress state within a streaming turn.
struct ToolProgress {
    id:          String,
    /// Raw tool name from the LLM (e.g. "shell_execute", "read-file").
    raw_name:    String,
    name:        String,
    activity:    String,
    summary:     String,
    started_at:  Instant,
    finished:    bool,
    success:     bool,
    duration:    Option<std::time::Duration>,
    error:       Option<String>,
    /// Compact result hint extracted from `ToolCallEnd::result_preview`.
    result_hint: Option<String>,
}

/// Status of a single plan step.
#[derive(Debug, Clone)]
enum StepStatus {
    Pending,
    Running,
    Done,
    Failed(String),
}

/// State of a single plan step for display.
#[derive(Debug, Clone)]
struct PlanStepState {
    task:   String,
    status: StepStatus,
}

/// Progress message state for tool execution feedback.
///
/// During streaming: renders live progress with tool activity + token footer.
/// On stream close: converted into an [`ExecutionTrace`], and the Telegram
/// message is edited to a compact summary with an inline "📊 详情" button
/// that toggles the full trace view.
///
/// ## Token semantics (from kernel `UsageUpdate`)
/// - `input_tokens` = latest iteration's prompt tokens (current context size)
/// - `output_tokens` = cumulative completion tokens across all iterations
/// - `thinking_ms` = cumulative extended-thinking duration
struct ProgressMessage {
    message_id:        Option<MessageId>,
    tools:             Vec<ToolProgress>,
    last_edit:         Instant,
    turn_started:      Instant,
    input_tokens:      u32,
    output_tokens:     u32,
    thinking_ms:       u64,
    /// Accumulated reasoning text for trace (truncated to ~500 chars).
    /// Collected from `StreamEvent::ReasoningDelta`; shown in expanded trace.
    reasoning_preview: String,
    /// Model name, populated from `StreamEvent::TurnMetrics` (arrives before
    /// stream close).
    model:             String,
    /// Iteration count, populated from `StreamEvent::TurnMetrics`.
    iterations:        usize,
    /// Rara internal message ID — the `InboundMessage.id` that triggered this
    /// turn.
    rara_message_id:   String,
    /// Plan steps saved as display strings for the post-completion trace
    /// detail view.
    saved_plan_steps:  Vec<String>,
    /// Cached loading hint, sampled once per turn to avoid flicker on
    /// re-render.
    loading_hint:      String,
    /// Plan steps — `Some` when in plan mode, `None` for reactive.
    plan_steps:        Option<Vec<PlanStepState>>,
    /// Goal description for the plan header.
    plan_goal:         Option<String>,
    /// Index of the currently executing step (0-based), `None` before first
    /// step starts.
    plan_current_step: Option<usize>,
    /// High-level rationale for the current turn, shown above tool lines.
    turn_rationale:    Option<String>,
    /// Whether the LLM is currently in extended thinking (reasoning) phase.
    /// Set on first `ReasoningDelta`, cleared on first `ToolCallStart`.
    thinking:          bool,
    /// Active background tasks (subagents) spawned during this turn.
    background_tasks:  Vec<BackgroundTaskState>,
}

/// Tracks a spawned background task for progress display.
struct BackgroundTaskState {
    task_id:     String,
    agent_name:  String,
    description: String,
    started_at:  Instant,
    finished:    bool,
    status:      Option<rara_kernel::io::BackgroundTaskStatus>,
}

impl ProgressMessage {
    fn new(rara_message_id: String) -> Self {
        Self {
            message_id: None,
            tools: Vec::new(),
            last_edit: Instant::now()
                .checked_sub(MIN_EDIT_INTERVAL)
                .unwrap_or_else(Instant::now),
            turn_started: Instant::now(),
            input_tokens: 0,
            output_tokens: 0,
            thinking_ms: 0,
            reasoning_preview: String::new(),
            model: String::new(),
            iterations: 0,
            rara_message_id,
            saved_plan_steps: Vec::new(),
            loading_hint: super::loading_hints::random_hint().to_string(),
            plan_steps: None,
            plan_goal: None,
            plan_current_step: None,
            turn_rationale: None,
            thinking: false,
            background_tasks: Vec::new(),
        }
    }

    /// Render the current progress text, dispatching to plan or reactive
    /// format.
    fn render_text(&self) -> String {
        if self.plan_steps.is_some() {
            render_plan_progress(self)
        } else {
            render_progress(&self.tools, self.turn_started.elapsed(), self)
        }
    }
}

use rara_kernel::trace::{ExecutionTrace, ToolTraceEntry};

/// Format a duration as a compact human-readable string.
pub(super) fn format_duration_compact(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 1 {
        format!("{}ms", d.as_millis())
    } else if secs < 60 {
        format!("{}.{}s", secs, d.subsec_millis() / 100)
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

/// Format a single tool line for per-tool progress display.
///
/// Format: `{status} {emoji} {verb} {summary} {elapsed}`
/// Three states: ⏳ in-progress, ✅ success, ❌ failure.
fn format_tool_line(t: &ToolProgress) -> String {
    use crate::tool_display::{tool_emoji, truncate_summary};

    let emoji = tool_emoji(&t.raw_name);
    let is_shell = matches!(t.raw_name.as_str(), "shell_execute" | "bash");
    let verb = if is_shell { "$" } else { &t.name };
    let summary = truncate_summary(&t.summary, 50);
    let summary_part = if summary.is_empty() {
        String::new()
    } else {
        format!(" {summary}")
    };

    if t.finished {
        let dur = t
            .duration
            .map(|d| format!(" {}", format_duration_compact(d)))
            .unwrap_or_default();
        let hint = t
            .result_hint
            .as_ref()
            .map(|h| format!(" {h}"))
            .unwrap_or_default();
        if t.success {
            format!("\u{2705} {emoji} {verb}{summary_part}{hint}{dur}")
        } else {
            let err_suffix = t
                .error
                .as_ref()
                .map(|e| {
                    let short: String = e.chars().take(40).collect();
                    format!(": {short}")
                })
                .unwrap_or_default();
            format!("\u{274c} {emoji} {verb}{summary_part}{dur}{err_suffix}")
        }
    } else if t.activity == rara_kernel::io::stages::THINKING {
        String::new()
    } else {
        let elapsed = format_duration_compact(t.started_at.elapsed());
        format!("\u{23f3} {emoji} {verb}{summary_part} {elapsed}")
    }
}

/// Build a thinking hint: show the first line of reasoning preview if
/// available, otherwise fall back to the poetic loading hint.
fn thinking_hint(progress: &ProgressMessage) -> String {
    let preview = progress.reasoning_preview.trim();
    if preview.is_empty() {
        return format!("\u{1f9e0} {}", progress.loading_hint);
    }
    let first_line = preview
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(preview);
    let truncated: String = first_line.chars().take(60).collect();
    let ellipsis = if first_line.chars().count() > 60 {
        "\u{2026}"
    } else {
        ""
    };
    format!("\u{1f9e0} {truncated}{ellipsis}")
}

/// Render per-tool progress lines for display in Telegram.
///
/// Each tool gets its own line with emoji + verb + summary + elapsed.
/// When there are more than 5 tools, older finished ones are collapsed
/// into a single summary line.
fn render_progress(
    tools: &[ToolProgress],
    turn_elapsed: std::time::Duration,
    progress: &ProgressMessage,
) -> String {
    if tools.is_empty() {
        if !progress.thinking {
            return String::new();
        }
        let verb = super::spinner_verbs::random_verb().to_lowercase();
        let mut lines = vec![thinking_hint(progress)];
        lines.push(format!(
            "\u{2733}\u{fe0f} {verb}... {}",
            format_duration_compact(turn_elapsed)
        ));
        return lines.join("\n");
    }

    let mut lines = Vec::new();
    let finished_count = tools.iter().filter(|t| t.finished).count();
    let total = tools.len();

    if total <= 5 {
        for tool in tools {
            let line = format_tool_line(tool);
            if !line.is_empty() {
                lines.push(line);
            }
        }
    } else {
        // Collapse older finished tools; keep last 2 finished + all in-progress.
        let show_last_finished = 2usize;
        let collapse_count = finished_count.saturating_sub(show_last_finished);

        if collapse_count > 0 {
            let collapsed_dur: std::time::Duration = tools
                .iter()
                .filter(|t| t.finished)
                .take(collapse_count)
                .filter_map(|t| t.duration)
                .sum();
            let dur_str = if collapsed_dur.is_zero() {
                String::new()
            } else {
                format!(" ({})", format_duration_compact(collapsed_dur))
            };
            lines.push(format!("\u{22ef} {collapse_count} tools done{dur_str}"));
        }

        let mut finished_seen = 0usize;
        for tool in tools {
            if tool.finished {
                finished_seen += 1;
                if finished_seen <= collapse_count {
                    continue;
                }
            }
            let line = format_tool_line(tool);
            if !line.is_empty() {
                lines.push(line);
            }
        }
    }

    // If all tools are done and LLM is thinking again, show hint.
    let all_done = tools.iter().all(|t| t.finished);
    if all_done && progress.thinking {
        lines.push(thinking_hint(progress));
    }

    // Background tasks (subagents).
    render_background_tasks(&progress.background_tasks, &mut lines);

    // Footer: spinner verb + elapsed + tokens + thinking.
    {
        let verb = super::spinner_verbs::random_verb().to_lowercase();
        let mut parts = vec![format_duration_compact(turn_elapsed)];

        if progress.input_tokens > 0 || progress.output_tokens > 0 {
            let in_str = format_token_count(progress.input_tokens);
            let out_str = format_token_count(progress.output_tokens);
            parts.push(format!("\u{2191}{in_str} \u{2193}{out_str}"));
        }

        if progress.thinking_ms > 0 {
            let secs = progress.thinking_ms / 1000;
            if secs > 0 {
                parts.push(format!("thought {secs}s"));
            }
        }

        lines.push(format!(
            "\u{2733}\u{fe0f} {verb}... {}",
            parts.join(" \u{00b7} ")
        ));
    }

    lines.join("\n")
}

/// Render plan-mode progress: steps as primary structure, tool calls
/// nested under the current running step.
///
/// TODO: UI strings are hardcoded in Chinese (e.g. "第N步", "（N步）").
/// Extract to a locale/template system when i18n is needed.
fn render_plan_progress(progress: &ProgressMessage) -> String {
    let plan_goal = progress.plan_goal.as_deref().unwrap_or("Plan");
    let steps = progress.plan_steps.as_deref().unwrap_or_default();
    let tools = &progress.tools;
    let turn_elapsed = progress.turn_started.elapsed();

    let total = steps.len();
    let mut lines = vec![format!(
        "\u{1f4cb} {plan_goal}\u{ff08}{total}\u{6b65}\u{ff09}"
    )];
    lines.push(String::new());

    for (i, step) in steps.iter().enumerate() {
        let (icon, suffix) = match &step.status {
            StepStatus::Done => ("\u{2705}", String::new()),
            StepStatus::Running => ("\u{25b6}\u{fe0f}", String::new()),
            StepStatus::Failed(reason) => {
                let short: String = reason.chars().take(40).collect();
                ("\u{274c}", format!(": {short}"))
            }
            StepStatus::Pending => ("\u{2b1c}", String::new()),
        };
        lines.push(format!(
            "{icon} \u{7b2c}{}\u{6b65}\u{ff1a}{}{suffix}",
            i + 1,
            step.task
        ));

        // Nest tool calls under the running step.
        if matches!(step.status, StepStatus::Running) && !tools.is_empty() {
            for tool in tools {
                let tool_line = format_tool_line(tool);
                if !tool_line.is_empty() {
                    lines.push(format!("  {tool_line}"));
                }
            }
        }
    }

    // Background tasks (subagents).
    render_background_tasks(&progress.background_tasks, &mut lines);

    // Footer: elapsed + tokens
    let mut parts = vec![format_duration_compact(turn_elapsed)];
    if progress.input_tokens > 0 || progress.output_tokens > 0 {
        let in_str = format_token_count(progress.input_tokens);
        let out_str = format_token_count(progress.output_tokens);
        parts.push(format!("\u{2191}{in_str} \u{2193}{out_str}"));
    }
    if progress.thinking_ms > 0 {
        let secs = progress.thinking_ms / 1000;
        if secs > 0 {
            parts.push(format!("thought {secs}s"));
        }
    }
    lines.push(String::new());
    lines.push(format!("\u{2733} {}", parts.join(" \u{00b7} ")));

    lines.join("\n")
}

/// Render background task (subagent) status lines.
///
/// Each task gets one line: status emoji + agent name + description + elapsed.
fn render_background_tasks(tasks: &[BackgroundTaskState], lines: &mut Vec<String>) {
    if tasks.is_empty() {
        return;
    }
    lines.push(String::new());
    for task in tasks {
        let elapsed = format_duration_compact(task.started_at.elapsed());
        if task.finished {
            let icon = match task.status {
                Some(rara_kernel::io::BackgroundTaskStatus::Completed) => "\u{2705}",
                Some(rara_kernel::io::BackgroundTaskStatus::Failed) => "\u{274c}",
                Some(rara_kernel::io::BackgroundTaskStatus::Cancelled) => "\u{23f9}\u{fe0f}",
                None => "\u{2705}",
            };
            lines.push(format!(
                "{icon} \u{1f916} {} \u{2014} {} {elapsed}",
                task.agent_name, task.description,
            ));
        } else {
            lines.push(format!(
                "\u{23f3} \u{1f916} {} \u{2014} {} {elapsed}",
                task.agent_name, task.description,
            ));
        }
    }
}

pub(super) fn format_token_count(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{tokens}")
    }
}

/// Render a compact one-line summary for a completed execution trace.
/// Example: `✅ 45s · ↑12.5k ↓1.2k · thought 9s`
/// This is the collapsed state shown with the inline "详情" button.
fn render_compact_summary(trace: &ExecutionTrace) -> String {
    let mut parts = Vec::new();
    parts.push(format_duration_compact(std::time::Duration::from_secs(
        trace.duration_secs,
    )));

    if trace.input_tokens > 0 || trace.output_tokens > 0 {
        parts.push(format!(
            "\u{2191}{} \u{2193}{}",
            format_token_count(trace.input_tokens),
            format_token_count(trace.output_tokens),
        ));
    }

    if trace.thinking_ms > 0 {
        let secs = trace.thinking_ms / 1000;
        if secs > 0 {
            parts.push(format!("thought {secs}s"));
        }
    }

    format!("\u{2705} {}", parts.join(" \u{00b7} "))
}

/// Render full execution trace detail for the expanded view.
/// Sections: 🧠 Thinking → 📋 Plan → 🔧 Tools → 📊 Usage
/// Hard-truncated to 4000 chars (Telegram limit is 4096).
/// Uses HTML formatting: `<b>`, `<blockquote>`, entity escaping.
fn render_trace_detail(trace: &ExecutionTrace) -> String {
    let mut text = render_compact_summary(trace);
    text.push_str("\n\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\n");

    // Turn rationale (moved from live progress to trace detail)
    if let Some(ref rationale) = trace.turn_rationale {
        text.push_str(&format!(
            "\n\u{1f4ad} <b>Rationale</b>\n<blockquote>{}</blockquote>\n",
            trace_html_escape(rationale),
        ));
    }

    // Thinking section
    if !trace.thinking_preview.is_empty() {
        text.push_str(&format!(
            "\n\u{1f9e0} <b>Thinking</b> ({}s)\n<blockquote>{}</blockquote>\n",
            trace.thinking_ms / 1000,
            trace_html_escape(&trace.thinking_preview),
        ));
    }

    // Plan section
    if !trace.plan_steps.is_empty() {
        text.push_str("\n\u{1f4cb} <b>Plan</b>\n");
        for step in &trace.plan_steps {
            text.push_str(&format!("  {}\n", trace_html_escape(step)));
        }
    }

    // Tools section
    if !trace.tools.is_empty() {
        text.push_str("\n\u{1f527} <b>Tools</b>\n");
        for (i, tool) in trace.tools.iter().enumerate() {
            let connector = if i == trace.tools.len() - 1 {
                "\u{2514}"
            } else {
                "\u{251c}"
            };
            let icon = if tool.success { "\u{2713}" } else { "\u{2717}" };
            let dur = tool
                .duration_ms
                .map(|ms| {
                    format!(
                        " ({})",
                        format_duration_compact(std::time::Duration::from_millis(ms))
                    )
                })
                .unwrap_or_default();
            let summary = if tool.summary.is_empty() {
                String::new()
            } else {
                format!(" \u{2014} {}", trace_html_escape(&tool.summary))
            };
            let err = tool
                .error
                .as_ref()
                .map(|e| format!("\n    \u{26a0} {}", trace_html_escape(e)))
                .unwrap_or_default();
            text.push_str(&format!(
                "  {connector} {}{dur} {icon}{summary}{err}\n",
                trace_html_escape(&tool.name)
            ));
        }
    }

    // Usage section
    text.push_str(&format!(
        "\n\u{1f4ca} <b>Usage</b>\n  {} iterations \u{00b7} \u{2191}{} \u{2193}{} tokens",
        trace.iterations,
        format_token_count(trace.input_tokens),
        format_token_count(trace.output_tokens),
    ));

    if !trace.model.is_empty() {
        text.push_str(&format!(" \u{00b7} {}", trace_html_escape(&trace.model)));
    }

    text.push_str(&format!(
        "\n\n\u{1f194} <b>Message ID</b>\n  <code>{}</code>",
        trace_html_escape(&trace.rara_message_id),
    ));

    // Telegram message limit is 4096 chars.
    // Must truncate on a char boundary to avoid panic on multi-byte UTF-8.
    if text.len() > 4000 {
        let truncate_at = text
            .char_indices()
            .take_while(|(i, _)| *i <= 3990)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(3990.min(text.len()));
        text.truncate(truncate_at);
        text.push_str("\n\u{2026}(truncated)");
    }

    text
}

/// Render a cascade trace as Telegram HTML for sending as a new message.
///
/// Each tick is rendered with its entries (UserInput, Thought, Action,
/// Observation) using emoji prefixes and `<blockquote>` blocks. Content is
/// truncated per-entry and the total output is capped at 4000 chars.
fn render_cascade_html(cascade: &rara_kernel::cascade::CascadeTrace) -> String {
    use rara_kernel::cascade::CascadeEntryKind;

    const MAX_ENTRY_CHARS: usize = 300;
    // 96 bytes of headroom below Telegram's 4096-char limit for the
    // truncation marker and any trailing whitespace.
    const MAX_TOTAL_CHARS: usize = 4000;

    let mut text = String::from("\u{1f50d} <b>Cascade Trace</b>\n");
    text.push_str(&format!(
        "<i>{} ticks \u{00b7} {} tool calls \u{00b7} {} entries</i>\n",
        cascade.summary.tick_count, cascade.summary.tool_call_count, cascade.summary.total_entries,
    ));

    let mut truncated = false;

    'outer: for tick in &cascade.ticks {
        let checkpoint = text.len();
        text.push_str(&format!("\n\u{25b6} <b>TICK {}</b>\n", tick.index + 1));
        if text.len() > MAX_TOTAL_CHARS {
            text.truncate(checkpoint);
            truncated = true;
            break;
        }

        for entry in &tick.entries {
            let checkpoint = text.len();

            let (emoji, label) = match entry.kind {
                CascadeEntryKind::UserInput => ("\u{1f4ac}", "User Input"),
                CascadeEntryKind::Thought => ("\u{1f9e0}", "Thought"),
                CascadeEntryKind::Action => ("\u{26a1}", "Action"),
                CascadeEntryKind::Observation => ("\u{1f441}", "Observation"),
            };

            let content = if entry.content.len() > MAX_ENTRY_CHARS {
                let truncate_at = entry
                    .content
                    .char_indices()
                    .take_while(|(i, _)| *i <= MAX_ENTRY_CHARS)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(MAX_ENTRY_CHARS.min(entry.content.len()));
                format!("{}\u{2026}", &entry.content[..truncate_at])
            } else {
                entry.content.clone()
            };

            // Use a placeholder for empty content to avoid empty blockquote tags.
            let display_content = if content.is_empty() {
                "(empty)"
            } else {
                &content
            };

            text.push_str(&format!(
                "  {emoji} <b>{label}</b> \u{00b7} <code>{}</code>\n<blockquote>{}</blockquote>\n",
                trace_html_escape(&entry.id),
                trace_html_escape(display_content),
            ));

            // If this entry pushed us over budget, roll back to avoid
            // truncating inside HTML tags (which produces malformed HTML
            // that Telegram rejects).
            if text.len() > MAX_TOTAL_CHARS {
                text.truncate(checkpoint);
                truncated = true;
                break 'outer;
            }
        }
    }

    if truncated {
        text.push_str("\n\u{2026}(truncated)");
    }

    text
}

/// Minimal HTML escaping for text embedded in trace display.
fn trace_html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

use crate::tool_display::{tool_activity_label, tool_display_info};

/// Monotonic counter used to tag each [`StreamingMessage`] with a unique epoch.
///
/// The stale-state cleanup task compares its captured epoch against the current
/// entry's epoch to avoid evicting a successor turn that reused the same
/// `chat_id` within the 120s cleanup window. Starts at 1 so 0 can act as a
/// sentinel if ever needed.
static STREAM_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Per-chat streaming state for progressive `editMessageText` updates.
struct StreamingMessage {
    /// Monotonic tag identifying which turn owns this entry. Used by the
    /// delayed cleanup task to distinguish its own state from a successor
    /// turn's fresh state.
    epoch:                 u64,
    /// All message IDs sent for this stream (multiple when splitting long
    /// content).
    message_ids:           Vec<MessageId>,
    /// Accumulated raw text for the current (latest) message.
    accumulated:           String,
    /// Number of raw characters already finalized into earlier split messages.
    streamed_prefix_chars: usize,
    /// Last successful `editMessageText` timestamp for throttling.
    last_edit:             Instant,
    /// Whether new text has been appended since the last edit.
    dirty:                 bool,
}

impl StreamingMessage {
    fn new() -> Self {
        Self {
            epoch:                 STREAM_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            message_ids:           Vec::new(),
            accumulated:           String::new(),
            streamed_prefix_chars: 0,
            last_edit:             Instant::now(),
            dirty:                 false,
        }
    }
}

/// Runtime configuration for the Telegram adapter.
///
/// Can be updated at runtime via [`TelegramAdapter::config_handle`] to change
/// authorization settings without restarting the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramConfig {
    /// Primary chat ID for privileged commands (e.g. /search, /jd).
    pub primary_chat_id:       Option<i64>,
    /// Allowed group chat ID. Only this group is authorized for bot
    /// interaction.
    pub allowed_group_chat_id: Option<i64>,
    /// How the bot handles group chat messages.
    pub group_policy:          GroupPolicy,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            primary_chat_id:       None,
            allowed_group_chat_id: None,
            group_policy:          GroupPolicy::MentionOrSmallGroup,
        }
    }
}

/// Telegram channel adapter using `getUpdates` long polling.
///
/// # Configuration
///
/// - `allowed_chat_ids` — when non-empty, only messages from these chat IDs are
///   processed. Messages from other chats are silently dropped. When empty, all
///   messages are accepted.
///
/// - `polling_timeout` — long-poll timeout in seconds (default: 30). The HTTP
///   client timeout is set 15 seconds higher to avoid premature disconnects.
///
/// - `config` — runtime-updatable settings (primary chat ID, allowed group chat
///   ID). Obtain a shared handle via [`config_handle`](Self::config_handle) and
///   mutate through `std::sync::RwLock::write()`.
///
/// # Lifecycle
///
/// 1. Call [`start`](ChannelAdapter::start) with a [`KernelHandle`]. This
///    spawns a background tokio task that polls for updates.
/// 2. For each inbound message, the adapter converts the Telegram [`Update`] to
///    a [`RawPlatformMessage`] and hands it to the sink. Outbound delivery is
///    handled separately via [`ChannelAdapter::send`].
/// 3. Call [`stop`](ChannelAdapter::stop) to signal the polling loop to exit
///    gracefully.
pub struct TelegramAdapter {
    bot:                   teloxide::Bot,
    allowed_chat_ids:      Vec<i64>,
    polling_timeout:       u32,
    shutdown_tx:           watch::Sender<bool>,
    shutdown_rx:           watch::Receiver<bool>,
    /// Bot username from getMe (set during start).
    bot_username:          Arc<RwLock<Option<String>>>,
    /// Registered command handlers for slash commands.
    command_handlers:      StdRwLock<Vec<Arc<dyn CommandHandler>>>,
    /// Registered callback handlers for interactive elements.
    callback_handlers:     StdRwLock<Vec<Arc<dyn CallbackHandler>>>,
    /// Runtime-updatable configuration (primary chat ID, allowed group chat
    /// ID).
    config:                Arc<StdRwLock<TelegramConfig>>,
    /// StreamHub for subscribing to real-time token deltas.
    stream_hub:            Arc<RwLock<Option<StreamHubRef>>>,
    /// Per-chat active streaming state, keyed by `chat_id`.
    active_streams:        Arc<DashMap<i64, StreamingMessage>>,
    /// User question manager for the ask-user tool — when set, the adapter
    /// subscribes to new questions and resolves them via reply-to messages.
    user_question_manager: Option<UserQuestionManagerRef>,
    /// Optional STT service for transcribing voice messages to text.
    stt_service:           Option<rara_stt::SttService>,
    /// Optional TTS service for synthesizing voice replies.
    tts_service:           Option<rara_tts::TtsService>,
    /// Chat IDs whose most recent inbound message was a voice note.
    /// Checked at egress to decide whether to reply with a voice note.
    voice_chat_ids:        Arc<DashSet<i64>>,
    /// Settings provider for persisting pinned message IDs across restarts.
    settings:              Arc<dyn SettingsProvider>,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter.
    ///
    /// # Arguments
    ///
    /// - `bot` — a configured [`teloxide::Bot`] instance
    /// - `allowed_chat_ids` — list of Telegram chat IDs that are permitted to
    ///   interact with the adapter. Pass an empty vec to allow all chats.
    pub fn new(
        bot: teloxide::Bot,
        allowed_chat_ids: Vec<i64>,
        settings: Arc<dyn SettingsProvider>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            bot,
            allowed_chat_ids,
            polling_timeout: POLL_TIMEOUT_SECS,
            shutdown_tx,
            shutdown_rx,
            bot_username: Arc::new(RwLock::new(None)),
            command_handlers: StdRwLock::new(Vec::new()),
            callback_handlers: StdRwLock::new(Vec::new()),
            config: Arc::new(StdRwLock::new(TelegramConfig::default())),
            stream_hub: Arc::new(RwLock::new(None)),
            active_streams: Arc::new(DashMap::new()),
            user_question_manager: None,
            stt_service: None,
            tts_service: None,
            voice_chat_ids: Arc::new(DashSet::new()),
            settings,
        }
    }

    /// Build a [`teloxide::Bot`] with an optional proxy, then wrap it in an
    /// adapter.
    ///
    /// The proxy URL is passed to [`reqwest012::Proxy::all`] (supports
    /// `http://`, `https://`, `socks5://`).
    pub fn with_proxy(
        token: &str,
        allowed_chat_ids: Vec<i64>,
        proxy: Option<&str>,
        settings: Arc<dyn SettingsProvider>,
    ) -> Result<Self, anyhow::Error> {
        let bot = build_bot(token, proxy)?;
        Ok(Self::new(bot, allowed_chat_ids, settings))
    }

    /// Create a new Telegram adapter with a custom polling timeout.
    #[must_use]
    pub fn with_polling_timeout(mut self, timeout_secs: u32) -> Self {
        self.polling_timeout = timeout_secs;
        self
    }

    /// Register command handlers (builder pattern — must be called before `Arc`
    /// wrapping).
    #[must_use]
    pub fn with_command_handlers(self, handlers: Vec<Arc<dyn CommandHandler>>) -> Self {
        *self
            .command_handlers
            .write()
            .unwrap_or_else(|e| e.into_inner()) = handlers;
        self
    }

    /// Replace command handlers at runtime (works through `&self` /
    /// `Arc<Self>`).
    pub fn set_command_handlers(&self, handlers: Vec<Arc<dyn CommandHandler>>) {
        *self
            .command_handlers
            .write()
            .unwrap_or_else(|e| e.into_inner()) = handlers;
    }

    /// Register callback handlers (builder pattern — must be called before
    /// `Arc` wrapping).
    #[must_use]
    pub fn with_callback_handlers(self, handlers: Vec<Arc<dyn CallbackHandler>>) -> Self {
        *self
            .callback_handlers
            .write()
            .unwrap_or_else(|e| e.into_inner()) = handlers;
        self
    }

    /// Replace callback handlers at runtime (works through `&self` /
    /// `Arc<Self>`).
    pub fn set_callback_handlers(&self, handlers: Vec<Arc<dyn CallbackHandler>>) {
        *self
            .callback_handlers
            .write()
            .unwrap_or_else(|e| e.into_inner()) = handlers;
    }

    /// Return a clone of the underlying [`teloxide::Bot`] handle.
    ///
    /// Used by command handlers that need to call Telegram APIs directly
    /// (e.g. deleting forum topics on `/clear`).
    pub fn bot(&self) -> teloxide::Bot { self.bot.clone() }

    /// Set the primary chat ID for privileged commands.
    ///
    /// Commands like `/search` and `/jd` are restricted to this chat only.
    /// This is a convenience builder that mutates the internal config.
    #[must_use]
    pub fn with_primary_chat_id(self, id: i64) -> Self {
        {
            let mut cfg = self.config.write().unwrap_or_else(|e| e.into_inner());
            cfg.primary_chat_id = Some(id);
        }
        self
    }

    /// Set the allowed group chat ID.
    ///
    /// When set, only the specified group is authorized for group-chat
    /// interactions. Messages from other groups receive an "unauthorized"
    /// response and are not dispatched further.
    /// This is a convenience builder that mutates the internal config.
    #[must_use]
    pub fn with_allowed_group_chat_id(self, id: i64) -> Self {
        {
            let mut cfg = self.config.write().unwrap_or_else(|e| e.into_inner());
            cfg.allowed_group_chat_id = Some(id);
        }
        self
    }

    /// Set the full runtime config.
    ///
    /// Replaces the current config with the provided one.
    #[must_use]
    pub fn with_config(self, config: TelegramConfig) -> Self {
        {
            let mut cfg = self.config.write().unwrap_or_else(|e| e.into_inner());
            *cfg = config;
        }
        self
    }

    /// Attach a [`UserQuestionManager`](rara_kernel::user_question::UserQuestionManager)
    /// so the adapter can render agent questions and resolve them via
    /// reply-to messages.
    #[must_use]
    pub fn with_user_question_manager(mut self, mgr: UserQuestionManagerRef) -> Self {
        self.user_question_manager = Some(mgr);
        self
    }

    /// Attach an STT service for voice message transcription.
    #[must_use]
    pub fn with_stt_service(mut self, stt: Option<rara_stt::SttService>) -> Self {
        self.stt_service = stt;
        self
    }

    /// Attach a TTS service for synthesizing voice replies.
    #[must_use]
    pub fn with_tts_service(mut self, tts: Option<rara_tts::TtsService>) -> Self {
        self.tts_service = tts;
        self
    }

    /// Return a shared handle to the runtime config.
    ///
    /// Callers can use this to update configuration at runtime (e.g. change the
    /// primary chat ID) without restarting the adapter. The polling loop reads
    /// the config on every update, so changes take effect immediately.
    pub fn config_handle(&self) -> Arc<StdRwLock<TelegramConfig>> { Arc::clone(&self.config) }

    /// Read a snapshot of the current config.
    ///
    /// If the lock is poisoned, recovers and returns the inner value.
    pub fn current_config(&self) -> TelegramConfig {
        match self.config.read() {
            Ok(g) => g.clone(),
            Err(e) => e.into_inner().clone(),
        }
    }

    /// Check whether a chat ID is allowed.
    ///
    /// Returns `true` if the allowed list is empty (all chats permitted) or
    /// if the chat ID is explicitly listed.
    fn is_allowed(&self, chat_id: i64) -> bool {
        self.allowed_chat_ids.is_empty() || self.allowed_chat_ids.contains(&chat_id)
    }

    /// Send binary attachments (images or documents) to a Telegram chat.
    async fn send_attachments(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        attachments: &[rara_kernel::io::Attachment],
    ) {
        use teloxide::types::InputFile;

        for attachment in attachments {
            let input_file = InputFile::memory(attachment.data.clone());
            let input_file = if let Some(ref name) = attachment.filename {
                input_file.file_name(name.clone())
            } else {
                input_file
            };

            if attachment.mime_type.starts_with("image/") {
                let req =
                    with_thread_id!(self.bot.send_photo(ChatId(chat_id), input_file), thread_id);
                let _ = req.await.map_err(|e| warn!("failed to send photo: {e}"));
            } else {
                let req = with_thread_id!(
                    self.bot.send_document(ChatId(chat_id), input_file),
                    thread_id
                );
                let _ = req.await.map_err(|e| warn!("failed to send document: {e}"));
            }
        }
    }

    /// Synthesize speech from `text` and send it as a Telegram voice note.
    ///
    /// Returns `true` if the voice note was sent successfully. On any failure
    /// the caller should fall back to a plain text message (graceful
    /// degradation).
    async fn try_send_voice_reply(&self, chat_id: i64, thread_id: Option<i64>, text: &str) -> bool {
        let Some(ref tts) = self.tts_service else {
            return false;
        };

        let audio = match tts.synthesize(text).await {
            Ok(a) => a,
            Err(e) => {
                warn!(error = %e, "TTS synthesis failed, falling back to text");
                return false;
            }
        };

        let input_file = teloxide::types::InputFile::memory(audio.data).file_name("reply.ogg");
        let req = with_thread_id!(self.bot.send_voice(ChatId(chat_id), input_file), thread_id);
        if let Err(e) = req.await {
            warn!(error = %e, "failed to send voice note, falling back to text");
            return false;
        }

        true
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn channel_type(&self) -> ChannelType { ChannelType::Telegram }

    async fn send(&self, endpoint: &Endpoint, msg: PlatformOutbound) -> Result<(), EgressError> {
        let (chat_id, thread_id) = match &endpoint.address {
            EndpointAddress::Telegram { chat_id, thread_id } => (*chat_id, *thread_id),
            _ => {
                return Err(EgressError::DeliveryFailed {
                    message: "not a telegram endpoint".to_string(),
                });
            }
        };

        match msg {
            PlatformOutbound::Reply {
                content,
                reply_context,
                attachments,
            } => {
                let original_len = content.len();
                let original_chars = content.chars().count();
                let mut content = if let Some(state) = self.active_streams.get(&chat_id) {
                    info!(
                        chat_id,
                        original_len,
                        original_chars,
                        streamed_prefix_chars = state.streamed_prefix_chars,
                        accumulated_len = state.accumulated.len(),
                        message_ids = ?state.message_ids,
                        dirty = state.dirty,
                        "tg egress: Reply arrived with active stream state"
                    );
                    slice_after_char_prefix(&content, state.streamed_prefix_chars)
                } else {
                    info!(
                        chat_id,
                        original_len, "tg egress: Reply arrived, no active stream state"
                    );
                    content
                };
                if content.is_empty() && attachments.is_empty() {
                    warn!(
                        chat_id,
                        original_len,
                        original_chars,
                        "tg egress: Reply content empty after prefix slice, skipping send"
                    );
                    self.active_streams.remove(&chat_id);
                    return Ok(());
                }

                if self.active_streams.contains_key(&chat_id) {
                    let mut streamed_visible_prefix: Option<String> = None;
                    {
                        let has_msg_id = self
                            .active_streams
                            .get(&chat_id)
                            .map(|s| s.message_ids.last().map_or(false, |id| *id != MessageId(0)))
                            .unwrap_or(false);

                        if !has_msg_id {
                            for _ in 0..30 {
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                let ready = self
                                    .active_streams
                                    .get(&chat_id)
                                    .map(|s| {
                                        s.message_ids.last().map_or(false, |id| *id != MessageId(0))
                                    })
                                    .unwrap_or(true);
                                if ready {
                                    break;
                                }
                            }
                        }
                    }

                    if let Some((_, stream_state)) = self.active_streams.remove(&chat_id) {
                        // Only treat accumulated text as "already displayed" when a
                        // Telegram message was actually sent. Fast text-only turns
                        // may close before the throttle tick fires, leaving
                        // accumulated content that was never shown to the user.
                        let msg_was_sent = stream_state
                            .message_ids
                            .last()
                            .map_or(false, |id| *id != MessageId(0));
                        info!(
                            chat_id,
                            msg_was_sent,
                            message_ids = ?stream_state.message_ids,
                            accumulated_len = stream_state.accumulated.len(),
                            streamed_prefix_chars = stream_state.streamed_prefix_chars,
                            content_len = content.len(),
                            "tg egress: removed stream state for final Reply"
                        );
                        if msg_was_sent {
                            streamed_visible_prefix =
                                Some(strip_tool_call_xml(&stream_state.accumulated));
                        }
                        if let Some(&last_msg_id) = stream_state.message_ids.last() {
                            if last_msg_id != MessageId(0) {
                                let html =
                                    crate::telegram::markdown::markdown_to_telegram_html(&content);
                                let chunks = crate::telegram::markdown::chunk_message(&html, 4096);
                                let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("");
                                let edit_result = self
                                    .bot
                                    .edit_message_text(ChatId(chat_id), last_msg_id, first_chunk)
                                    .parse_mode(ParseMode::Html)
                                    .await;

                                let edit_ok = match &edit_result {
                                    Ok(_) => true,
                                    Err(teloxide::RequestError::Api(api_err))
                                        if format!("{api_err}")
                                            .contains("message is not modified") =>
                                    {
                                        true
                                    }
                                    Err(_) => false,
                                };

                                if edit_ok {
                                    for chunk in chunks.iter().skip(1) {
                                        let req = with_thread_id!(
                                            self.bot
                                                .send_message(ChatId(chat_id), chunk)
                                                .parse_mode(ParseMode::Html),
                                            thread_id
                                        );
                                        let _ = req.await;
                                    }
                                    self.send_attachments(chat_id, thread_id, &attachments)
                                        .await;
                                    return Ok(());
                                }
                                if let Err(e) = edit_result {
                                    warn!(
                                        chat_id,
                                        error = %e,
                                        "telegram: edit streaming message failed, falling back to suffix send"
                                    );
                                }
                            }
                        }
                    }

                    // If editing the streamed message failed, only send the
                    // unsent suffix to avoid duplicated prefix messages.
                    if let Some(prefix) = streamed_visible_prefix.as_deref() {
                        content = slice_after_prefix_if_matches(&content, prefix);
                    }
                }

                // If the inbound was a voice message and TTS is configured,
                // reply with a voice note (graceful degradation on failure).
                if self.voice_chat_ids.remove(&chat_id).is_some() && !content.is_empty() {
                    self.try_send_voice_reply(chat_id, thread_id, &content)
                        .await;
                }

                if !content.is_empty() {
                    let html = crate::telegram::markdown::markdown_to_telegram_html(&content);
                    let chunks = crate::telegram::markdown::chunk_message(&html, 4096);
                    for (i, chunk) in chunks.iter().enumerate() {
                        let mut req = with_thread_id!(
                            self.bot
                                .send_message(ChatId(chat_id), chunk)
                                .parse_mode(ParseMode::Html),
                            thread_id
                        );

                        if i == 0 {
                            if let Some(ref ctx) = reply_context {
                                if let Some(ref reply_id) = ctx.reply_to_platform_msg_id {
                                    if let Ok(msg_id) = parse_message_id(reply_id) {
                                        req = req.reply_parameters(ReplyParameters::new(msg_id));
                                    }
                                }
                            }
                        }

                        req.await.map_err(|e| EgressError::DeliveryFailed {
                            message: format!("failed to send telegram message: {e}"),
                        })?;
                    }
                }

                // Send attachments (images/documents).
                self.send_attachments(chat_id, thread_id, &attachments)
                    .await;
            }
            PlatformOutbound::StreamChunk {
                delta, edit_target, ..
            } => {
                if let Some(ref target_id) = edit_target {
                    if let Ok(msg_id) = parse_message_id(target_id) {
                        let html = crate::telegram::markdown::markdown_to_telegram_html(&delta);
                        let _ = self
                            .bot
                            .edit_message_text(ChatId(chat_id), msg_id, &html)
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                } else {
                    let req =
                        with_thread_id!(self.bot.send_message(ChatId(chat_id), &delta), thread_id);
                    let _ = req.await;
                }
            }
            PlatformOutbound::Progress { .. } => {
                let req = with_thread_id!(
                    self.bot
                        .send_chat_action(ChatId(chat_id), ChatAction::Typing),
                    thread_id
                );
                let _ = req.await;
            }
        }

        Ok(())
    }

    async fn start(&self, handle: KernelHandle) -> Result<(), KernelError> {
        *self.stream_hub.write().await = Some(handle.stream_hub().clone());

        // Delete any existing webhook so getUpdates works.
        self.bot
            .delete_webhook()
            .await
            .map_err(|e| KernelError::Other {
                message: format!("failed to delete webhook: {e}").into(),
            })?;

        // Verify the bot token via getMe.
        let me = self.bot.get_me().await.map_err(|e| KernelError::Other {
            message: format!("failed to verify bot token via getMe: {e}").into(),
        })?;
        info!(
            bot_id = me.id.0,
            bot_username = ?me.username,
            "telegram adapter: bot identity verified"
        );

        // Store bot username for metadata enrichment.
        if let Some(ref username) = me.username {
            *self.bot_username.write().await = Some(username.clone());
        }

        let bot = self.bot.clone();
        let allowed_chat_ids = self.allowed_chat_ids.clone();
        let polling_timeout = self.polling_timeout;
        let mut shutdown_rx = self.shutdown_rx.clone();
        let bot_username = Arc::clone(&self.bot_username);
        let config = Arc::clone(&self.config);
        let stream_hub = Arc::clone(&self.stream_hub);
        let active_streams = Arc::clone(&self.active_streams);
        let command_handlers: Arc<[Arc<dyn CommandHandler>]> = self
            .command_handlers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .into();
        let callback_handlers: Arc<[Arc<dyn CallbackHandler>]> = self
            .callback_handlers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .into();
        let stt_service = self.stt_service.clone();
        let voice_chat_ids = Arc::clone(&self.voice_chat_ids);
        let settings = Arc::clone(&self.settings);

        // Register slash-menu with Telegram so '/' shows available commands.
        {
            let all_cmds: Vec<teloxide::types::BotCommand> = command_handlers
                .iter()
                .flat_map(|h| h.commands())
                .map(|def| teloxide::types::BotCommand::new(&def.name, &def.description))
                .collect();
            if !all_cmds.is_empty() {
                if let Err(e) = bot.set_my_commands(all_cmds).await {
                    warn!(error = %e, "telegram: failed to register bot commands");
                } else {
                    info!(
                        "telegram: registered {} bot command(s)",
                        command_handlers.iter().flat_map(|h| h.commands()).count()
                    );
                }
            }
        }

        // Spawn approval request listener — sends inline keyboard to the
        // originating chat (resolved via session binding).
        {
            let approval_rx = handle.security().approval().subscribe_requests();
            let approval_bot = self.bot.clone();
            let approval_config = Arc::clone(&self.config);
            let approval_session_index = Arc::clone(handle.session_index());
            let mut approval_shutdown = self.shutdown_rx.clone();
            tokio::spawn(async move {
                approval_listener(
                    approval_bot,
                    approval_rx,
                    approval_config,
                    approval_session_index,
                    &mut approval_shutdown,
                )
                .await;
            });
        }

        // Spawn user-question listener — sends question messages to primary chat.
        // User replies to these messages are intercepted in `handle_update` and
        // routed to `UserQuestionManager::resolve()`.
        if let Some(ref mgr) = self.user_question_manager {
            let question_rx = mgr.subscribe();
            let question_bot = self.bot.clone();
            let question_config = Arc::clone(&self.config);
            let question_mgr = Arc::clone(mgr);
            let mut question_shutdown = self.shutdown_rx.clone();
            tokio::spawn(async move {
                question_listener(
                    question_bot,
                    question_rx,
                    question_config,
                    question_mgr,
                    &mut question_shutdown,
                )
                .await;
            });
        }

        tokio::spawn(async move {
            polling_loop(
                bot,
                handle,
                allowed_chat_ids,
                polling_timeout,
                &mut shutdown_rx,
                bot_username,
                config,
                stream_hub,
                active_streams,
                command_handlers,
                callback_handlers,
                stt_service,
                voice_chat_ids,
                settings,
            )
            .await;
        });

        info!("telegram adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<(), KernelError> {
        let _ = self.shutdown_tx.send(true);
        info!("telegram adapter: shutdown signal sent");
        Ok(())
    }

    async fn typing_indicator(&self, session_key: &str) -> Result<(), KernelError> {
        let chat_id = parse_chat_id(session_key)?;
        self.bot
            .send_chat_action(ChatId(chat_id), ChatAction::Typing)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("failed to send typing indicator: {e}").into(),
            })?;
        Ok(())
    }

    async fn rename_session_label(
        &self,
        binding: &rara_kernel::session::ChannelBinding,
        title: &str,
    ) -> Result<(), KernelError> {
        let Some(thread_id_str) = binding.thread_id.as_deref() else {
            // No thread — nothing to rename at the Telegram topic level.
            return Ok(());
        };
        let Ok(chat_id) = binding.chat_id.parse::<i64>() else {
            tracing::warn!(chat_id = %binding.chat_id, "rename: invalid chat_id");
            return Ok(());
        };
        let Ok(tid) = thread_id_str.parse::<i32>() else {
            tracing::warn!(thread_id = %thread_id_str, "rename: invalid thread_id");
            return Ok(());
        };
        let thread_id = teloxide::types::ThreadId(teloxide::types::MessageId(tid));
        // Telegram caps topic name at 128 chars.
        let name: String = title.chars().take(128).collect();
        if let Err(e) = self
            .bot
            .edit_forum_topic(teloxide::types::ChatId(chat_id), thread_id)
            .name(name)
            .await
        {
            tracing::warn!(error = %e, chat_id, tid, "edit_forum_topic failed");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Polling loop (I/O Bus model via KernelHandle)
// ---------------------------------------------------------------------------

/// The getUpdates long-polling loop.
///
/// Converts each update to a [`RawPlatformMessage`] and hands it to the
/// [`KernelHandle`] in a fire-and-forget fashion. The adapter does **not**
/// wait for a response -- egress delivers replies through
/// [`ChannelAdapter::send`].
///
/// Commands and callbacks are routed through the kernel like regular
/// messages via [`InteractionType`]. The adapter only performs
/// authorization checks and group-chat filtering.
async fn polling_loop(
    bot: teloxide::Bot,
    handle: KernelHandle,
    allowed_chat_ids: Vec<i64>,
    polling_timeout: u32,
    shutdown_rx: &mut watch::Receiver<bool>,
    bot_username: Arc<RwLock<Option<String>>>,
    config: Arc<StdRwLock<TelegramConfig>>,
    stream_hub: Arc<RwLock<Option<StreamHubRef>>>,
    active_streams: Arc<DashMap<i64, StreamingMessage>>,
    command_handlers: Arc<[Arc<dyn CommandHandler>]>,
    callback_handlers: Arc<[Arc<dyn CallbackHandler>]>,
    stt_service: Option<rara_stt::SttService>,
    voice_chat_ids: Arc<DashSet<i64>>,
    settings: Arc<dyn SettingsProvider>,
) {
    let mut offset: Option<i32> = None;
    let mut retry_delay = INITIAL_RETRY_DELAY;

    info!("telegram adapter: starting getUpdates polling loop");

    loop {
        // Check for shutdown before each poll.
        if *shutdown_rx.borrow() {
            info!("telegram adapter: shutdown received");
            break;
        }

        let mut request = bot
            .get_updates()
            .timeout(polling_timeout)
            .allowed_updates(vec![
                AllowedUpdate::Message,
                AllowedUpdate::EditedMessage,
                AllowedUpdate::CallbackQuery,
            ]);

        if let Some(off) = offset {
            request = request.offset(off);
        }

        // Use select to allow shutdown during the long poll.
        let result = tokio::select! {
            _ = shutdown_rx.changed() => {
                info!("telegram adapter: shutdown during getUpdates wait");
                break;
            }
            result = request.send() => result,
        };

        match result {
            Ok(updates) => {
                // Reset retry delay on success.
                retry_delay = INITIAL_RETRY_DELAY;

                for update in updates {
                    // Advance offset past this update.
                    #[allow(clippy::cast_possible_wrap)]
                    let next_offset = update.id.0 as i32 + 1;
                    offset = Some(next_offset);

                    // Spawn handler as a separate task.
                    let handle = handle.clone();
                    let bot = bot.clone();
                    let allowed = allowed_chat_ids.clone();
                    let bot_username = Arc::clone(&bot_username);
                    let config = Arc::clone(&config);
                    let stream_hub = Arc::clone(&stream_hub);
                    let active_streams = Arc::clone(&active_streams);
                    let command_handlers = Arc::clone(&command_handlers);
                    let callback_handlers = Arc::clone(&callback_handlers);
                    let stt = stt_service.clone();
                    let voice_ids = Arc::clone(&voice_chat_ids);
                    let stg = Arc::clone(&settings);
                    tokio::spawn(async move {
                        handle_update(
                            update,
                            &handle,
                            &bot,
                            &allowed,
                            &bot_username,
                            &config,
                            &stream_hub,
                            &active_streams,
                            &command_handlers,
                            &callback_handlers,
                            &stt,
                            &voice_ids,
                            &stg,
                        )
                        .await;
                    });
                }
            }
            Err(teloxide::RequestError::Api(ref api_err)) => {
                let err_str = format!("{api_err}");
                if err_str.contains("terminated by other getUpdates request") {
                    warn!("telegram adapter: another bot instance detected — exiting");
                    break;
                }
                error!(error = ?api_err, "telegram adapter: API error in getUpdates");
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
            }
            Err(e) => {
                error!(error = ?e, "telegram adapter: getUpdates request failed");
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
            }
        }
    }

    info!("telegram adapter: polling loop stopped");
}

// ---------------------------------------------------------------------------
// Approval request listener
// ---------------------------------------------------------------------------

/// Minimal HTML escaping for text embedded in Telegram HTML messages.
fn guard_html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Handle a guard approval/deny callback query from an inline keyboard button.
async fn handle_guard_callback(
    handle: &KernelHandle,
    bot: &teloxide::Bot,
    callback: &teloxide::types::CallbackQuery,
    data: &str,
    allowed_user_ids: &[i64],
) {
    // Verify the user clicking the button is authorized.
    let from_id = callback.from.id.0 as i64;
    if !allowed_user_ids.is_empty() && !allowed_user_ids.contains(&from_id) {
        warn!(
            user_id = from_id,
            "guard callback: unauthorized user attempted approval"
        );
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("⚠️ Unauthorized")
            .await;
        return;
    }
    // Parse "guard:approve:{uuid}" or "guard:deny:{uuid}"
    let parts: Vec<&str> = data.splitn(3, ':').collect();
    if parts.len() != 3 {
        warn!(data, "guard callback: malformed data");
        return;
    }

    let (action, id_str) = (parts[1], parts[2]);
    let request_id = match uuid::Uuid::parse_str(id_str) {
        Ok(id) => id,
        Err(e) => {
            warn!(id_str, error = %e, "guard callback: invalid UUID");
            return;
        }
    };

    let decision = match action {
        "approve" => ApprovalDecision::Approved,
        "deny" => ApprovalDecision::Denied,
        _ => {
            warn!(action, "guard callback: unknown action");
            return;
        }
    };

    let decided_by = callback
        .from
        .username
        .as_deref()
        .unwrap_or("unknown")
        .to_string();

    // Cancel the auto-expiry task before resolving so it cannot race with the
    // message edit below.
    if let Some((_, abort_handle)) = GUARD_EXPIRY_HANDLES.remove(&request_id) {
        abort_handle.abort();
    }

    let result =
        handle
            .security()
            .approval()
            .resolve(request_id, decision, Some(decided_by.clone()));

    // Answer the callback query (removes the loading spinner on the button).
    let answer_text = match decision {
        ApprovalDecision::Approved => "✅ Approved",
        ApprovalDecision::Denied => "❌ Denied",
        _ => "Done",
    };
    let _ = bot
        .answer_callback_query(callback.id.clone())
        .text(answer_text)
        .await;

    // Collapse the full guard message into a compact one-liner after
    // the decision so it no longer dominates the chat view.
    if let Some(msg) = &callback.message {
        let (msg_id, chat_id) = match msg {
            teloxide::types::MaybeInaccessibleMessage::Regular(m) => (m.id, m.chat.id),
            teloxide::types::MaybeInaccessibleMessage::Inaccessible(m) => (m.message_id, m.chat.id),
        };

        let compact = match (&decision, &result) {
            (ApprovalDecision::Approved, Ok(_)) => {
                format!("🛡 <b>Guard</b> ✅ by @{decided_by}")
            }
            (ApprovalDecision::Denied, Ok(_)) => {
                format!("🛡 <b>Guard</b> ❌ by @{decided_by}")
            }
            (_, Err(ResolveError::Expired)) => "🛡 <b>Guard</b> ⏰ timed out".to_string(),
            (_, Err(ResolveError::NotFound(_))) => "🛡 <b>Guard</b> ⏰ already resolved".to_string(),
            #[allow(unreachable_patterns)]
            (_, Err(e)) => {
                format!("🛡 <b>Guard</b> ⚠️ {}", guard_html_escape(&e.to_string()))
            }
            (_, Ok(_)) => "🛡 <b>Guard</b> done".to_string(),
        };

        let _ = bot
            .edit_message_text(chat_id, msg_id, compact)
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(
                Vec::<Vec<InlineKeyboardButton>>::new(),
            ))
            .await;
    }

    match result {
        Ok(resp) => info!(
            request_id = %request_id,
            decision = ?resp.decision,
            decided_by = ?resp.decided_by,
            "guard approval resolved via Telegram"
        ),
        Err(e) => warn!(
            request_id = %request_id,
            error = %e,
            "guard approval resolution failed"
        ),
    }
}

/// Handle a tool-call-limit callback query (continue/stop) from a Telegram
/// inline keyboard button.
///
/// ## Callback data protocol
///
/// Format: `"limit:{action}:{session_key}:{limit_id}"`
///
/// - `action`      — `"continue"` (resume loop) or `"stop"` (graceful stop)
/// - `session_key`  — identifies the session whose agent loop is paused
/// - `limit_id` — monotonic counter binding this button to a specific limit
///   instance. Stale IDs are rejected by
///   `KernelHandle::resolve_tool_call_limit`.
///
/// ## Authorization
///
/// Uses **chat-based auth** (`callback.message.chat().id`) checked against
/// `allowed_chat_ids`, matching the same authorization used for inbound
/// messages. This is intentional: in group chats any member of the allowed
/// chat can resolve the limit, not just the user who triggered it.
///
/// ## UI feedback
///
/// After resolving, the original inline keyboard message is edited to show
/// who made the decision and what action was taken.
async fn handle_tool_call_limit_callback(
    handle: &KernelHandle,
    bot: &teloxide::Bot,
    callback: &teloxide::types::CallbackQuery,
    data: &str,
    allowed_chat_ids: &[i64],
) {
    // Verify authorization: check that the callback originates from an
    // allowed chat (same list used for message authorization).
    let cb_chat_id = callback
        .message
        .as_ref()
        .map(|m| m.chat().id.0)
        .unwrap_or(0);
    if !allowed_chat_ids.is_empty() && !allowed_chat_ids.contains(&cb_chat_id) {
        warn!(
            chat_id = cb_chat_id,
            "tool call limit callback: unauthorized chat"
        );
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("⚠️ Unauthorized")
            .await;
        return;
    }

    // Parse "limit:continue:{session_key}:{limit_id}" or
    // "limit:stop:{session_key}:{limit_id}"
    let parts: Vec<&str> = data.splitn(4, ':').collect();
    if parts.len() != 4 {
        warn!(data, "tool call limit callback: malformed data");
        return;
    }

    let (action, session_key_str, limit_id_str) = (parts[1], parts[2], parts[3]);
    let session_key = match rara_kernel::session::SessionKey::try_from_raw(session_key_str) {
        Ok(k) => k,
        Err(e) => {
            warn!(error = %e, "tool call limit callback: invalid session key");
            return;
        }
    };
    let limit_id: u64 = match limit_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            warn!(limit_id_str, "tool call limit callback: invalid limit_id");
            return;
        }
    };

    let decision = match action {
        "continue" => rara_kernel::io::ToolCallLimitDecision::Continue,
        "stop" => rara_kernel::io::ToolCallLimitDecision::Stop,
        _ => {
            warn!(action, "tool call limit callback: unknown action");
            return;
        }
    };

    let resolved = handle.resolve_tool_call_limit(session_key, limit_id, decision);

    let answer_text = match decision {
        rara_kernel::io::ToolCallLimitDecision::Continue => "▶️ Continuing",
        rara_kernel::io::ToolCallLimitDecision::Stop => "⏹ Stopped",
    };
    let _ = bot
        .answer_callback_query(callback.id.clone())
        .text(answer_text)
        .await;

    // Edit message to show decision, remove buttons.
    if let Some(msg) = &callback.message {
        let decided_by = callback.from.username.as_deref().unwrap_or("unknown");
        let (msg_id, chat_id, original_text) = match msg {
            teloxide::types::MaybeInaccessibleMessage::Regular(m) => (
                m.id,
                m.chat.id,
                m.text().unwrap_or("Pause decision").to_owned(),
            ),
            teloxide::types::MaybeInaccessibleMessage::Inaccessible(m) => {
                (m.message_id, m.chat.id, "Pause decision".to_owned())
            }
        };

        let status = if resolved {
            match decision {
                rara_kernel::io::ToolCallLimitDecision::Continue => {
                    format!("▶️ <b>Continued</b> by @{decided_by}")
                }
                rara_kernel::io::ToolCallLimitDecision::Stop => {
                    format!("⏹ <b>Stopped</b> by @{decided_by}")
                }
            }
        } else {
            "⚠️ Decision expired (agent already finished or timed out)".to_string()
        };

        let new_text = format!("{}\n\n{}", guard_html_escape(&original_text), status);
        let _ = bot
            .edit_message_text(chat_id, msg_id, new_text)
            .parse_mode(ParseMode::Html)
            .await;
    }
}

/// Handle a trace show/hide callback query from an inline keyboard button.
///
/// Callback data format: `"trace:{action}:{chat_id}:{msg_id}"`
/// - `action` = "show" → expand to full trace, button becomes "收起"
/// - `action` = "hide" → collapse back to compact summary, button becomes
///   "详情"
///
/// Trace data is fetched from [`rara_kernel::trace::TraceService`] (SQLite)
/// and rendered into Telegram HTML on demand. The callback is always answered
/// immediately to eliminate the Telegram spinner.
///
/// Callback data format: `"trace:{action}:{chat_id}:{msg_id}:{trace_id}"`
///
/// Legacy format `"trace:{action}:{chat_id}:{msg_id}"` (pre-v0.0.18) is also
/// handled gracefully: the spinner is dismissed with a hint toast.
async fn handle_trace_callback(
    bot: &teloxide::Bot,
    callback: &teloxide::types::CallbackQuery,
    data: &str,
    trace_service: &rara_kernel::trace::TraceService,
) {
    // Parse callback data. Always answer first to dismiss spinner.
    let parts: Vec<&str> = data.splitn(5, ':').collect();

    // Legacy 3-segment format (no trace_id) — answer with hint and bail.
    if parts.len() != 5 {
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("Trace expired after update, please trigger a new one")
            .await;
        return;
    }

    let action = parts[1];
    let chat_id_str = parts[2];
    let msg_id_str = parts[3];
    let trace_id = parts[4];

    // Answer callback immediately — removes Telegram spinner.
    let _ = bot.answer_callback_query(callback.id.clone()).await;

    let trace = match trace_service.get(trace_id).await {
        Ok(Some(t)) => t,
        _ => return,
    };

    let callback_prefix = format!("trace:{{}}:{chat_id_str}:{msg_id_str}:{trace_id}");
    let (text, button_text, next_action) = match action {
        "show" => (
            render_trace_detail(&trace),
            "\u{1f4ca} \u{6536}\u{8d77}",
            callback_prefix.replace("{}", "hide"),
        ),
        _ => (
            render_compact_summary(&trace),
            "\u{1f4ca} \u{8be6}\u{60c5}",
            callback_prefix.replace("{}", "show"),
        ),
    };

    if let (Ok(cid), Ok(mid)) = (chat_id_str.parse::<i64>(), msg_id_str.parse::<i32>()) {
        let cascade_callback = format!("cas:show:{chat_id_str}:{msg_id_str}:{trace_id}");
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(button_text, next_action),
            InlineKeyboardButton::callback("\u{1f50d} Cascade", cascade_callback),
        ]]);
        let _ = bot
            .edit_message_text(ChatId(cid), MessageId(mid), &text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await;
    }
}

/// Handle a cascade callback: toggle cascade trace view in-place on the
/// existing message (edit instead of sending a new message).
///
/// Callback data format: `"cas:{show|hide}:{chat_id}:{msg_id}:{trace_id}"`
///
/// NOTE: The full callback_data string approaches the Telegram 64-byte limit
/// (worst case ~60 bytes with supergroup chat_id + ULID). Do not add more
/// segments without shortening existing ones.
async fn handle_cascade_callback(
    bot: &teloxide::Bot,
    callback: &teloxide::types::CallbackQuery,
    data: &str,
    handle: &KernelHandle,
) {
    let parts: Vec<&str> = data.splitn(5, ':').collect();
    if parts.len() != 5 {
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("Invalid cascade callback")
            .await;
        return;
    }

    let action = parts[1];
    let chat_id_str = parts[2];
    let msg_id_str = parts[3];
    let trace_id = parts[4];

    let (Ok(cid), Ok(mid)) = (chat_id_str.parse::<i64>(), msg_id_str.parse::<i32>()) else {
        let _ = bot.answer_callback_query(callback.id.clone()).await;
        return;
    };

    match action {
        // "hide" → restore compact summary with the original buttons.
        "hide" => {
            let _ = bot.answer_callback_query(callback.id.clone()).await;
            let trace = match handle.trace_service().get(trace_id).await {
                Ok(Some(t)) => t,
                _ => return,
            };
            let compact = render_compact_summary(&trace);
            let trace_cb = format!("trace:show:{chat_id_str}:{msg_id_str}:{trace_id}");
            let cascade_cb = format!("cas:show:{chat_id_str}:{msg_id_str}:{trace_id}");
            let keyboard = InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("\u{1f4ca} \u{8be6}\u{60c5}", trace_cb),
                InlineKeyboardButton::callback("\u{1f50d} Cascade", cascade_cb),
            ]]);
            if let Err(e) = bot
                .edit_message_text(ChatId(cid), MessageId(mid), &compact)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await
            {
                warn!(
                    error = %e,
                    chat_id = cid,
                    msg_id = mid,
                    "cascade: failed to restore compact summary"
                );
            }
        }

        // "show" → build cascade trace and display in-place.
        "show" => {
            // Answer callback immediately to dismiss Telegram's loading spinner,
            // before any async data loading that could cause a timeout.
            let _ = bot.answer_callback_query(callback.id.clone()).await;

            let session_id = match handle.trace_service().get_session_id(trace_id).await {
                Ok(Some(s)) => s,
                _ => {
                    warn!("cascade: trace not found for trace_id={trace_id}");
                    let _ = bot
                        .edit_message_text(
                            ChatId(cid),
                            MessageId(mid),
                            "⚠️ Cascade not available: trace not found",
                        )
                        .await;
                    return;
                }
            };

            let entries = match handle.tape().entries(&session_id).await {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "cascade: failed to read tape entries");
                    let _ = bot
                        .edit_message_text(
                            ChatId(cid),
                            MessageId(mid),
                            "⚠️ Cascade not available: tape read error",
                        )
                        .await;
                    return;
                }
            };

            tracing::debug!(
                session_id = %session_id,
                entry_count = entries.len(),
                "cascade: loaded tape entries"
            );

            let rara_message_id = match handle.trace_service().get(trace_id).await {
                Ok(Some(t)) => t.rara_message_id,
                _ => String::new(),
            };

            // Locate the turn slice for this trace.
            let boundaries = rara_kernel::cascade::find_turn_boundaries(&entries);
            let turn_entries = if let Ok(ulid) = ulid::Ulid::from_string(trace_id) {
                let ts_ms = ulid.timestamp_ms() as i64;
                let target = jiff::Timestamp::from_millisecond(ts_ms)
                    .unwrap_or_else(|_| jiff::Timestamp::now());
                let turn =
                    rara_kernel::cascade::find_turn_by_timestamp(&entries, &boundaries, target);
                rara_kernel::cascade::turn_slice(&entries, &boundaries, turn)
            } else {
                &entries
            };

            // Try pre-built trace first; fall back to post-hoc build for
            // legacy sessions.
            let cascade = rara_kernel::cascade::load_persisted_cascade(turn_entries)
                .unwrap_or_else(|| {
                    rara_kernel::cascade::build_cascade(turn_entries, &rara_message_id)
                });

            tracing::debug!(
                ticks = cascade.ticks.len(),
                total_entries = cascade.summary.total_entries,
                "cascade: built trace"
            );

            if cascade.ticks.is_empty() {
                warn!("cascade: trace is empty for trace_id={trace_id}");
                let _ = bot
                    .edit_message_text(ChatId(cid), MessageId(mid), "⚠️ Cascade trace is empty")
                    .await;
                return;
            }

            let html = render_cascade_html(&cascade);
            let hide_cb = format!("cas:hide:{chat_id_str}:{msg_id_str}:{trace_id}");
            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                "\u{1f50d} \u{6536}\u{8d77}",
                hide_cb,
            )]]);
            if let Err(e) = bot
                .edit_message_text(ChatId(cid), MessageId(mid), &html)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard.clone())
                .await
            {
                warn!(
                    error = %e,
                    chat_id = cid,
                    msg_id = mid,
                    html_len = html.len(),
                    "cascade: failed to edit message with cascade view, retrying as plain text"
                );
                // Fallback: show a plain-text error so the user gets feedback
                // instead of silent failure ("no response").
                let fallback = format!(
                    "\u{26a0}\u{fe0f} Cascade rendering failed (HTML too complex, {} \
                     bytes).\nTicks: {}, entries: {}",
                    html.len(),
                    cascade.summary.tick_count,
                    cascade.summary.total_entries,
                );
                if let Err(e2) = bot
                    .edit_message_text(ChatId(cid), MessageId(mid), &fallback)
                    .reply_markup(keyboard)
                    .await
                {
                    warn!(error = %e2, "cascade: plain-text fallback also failed");
                }
            }
        }

        unknown => {
            warn!(action = unknown, "cascade: unknown action");
            let _ = bot.answer_callback_query(callback.id.clone()).await;
        }
    }
}

/// Handle a dashboard callback: render the requested tab and edit the
/// message in-place.
///
/// Callback data format: `"dash:{tab}:{chat_id}:{msg_id}:{trace_id}"`
///
/// Sessions are scoped to the chat's bound session + children so that
/// one chat cannot inspect another chat's sessions.
async fn handle_dashboard_callback(
    bot: &teloxide::Bot,
    callback: &teloxide::types::CallbackQuery,
    data: &str,
    handle: &KernelHandle,
) {
    let _ = bot.answer_callback_query(callback.id.clone()).await;

    let parts: Vec<&str> = data.splitn(5, ':').collect();
    if parts.len() < 4 {
        return;
    }

    let tab = super::dashboard::DashTab::from_str_prefix(parts[1]);
    let (Ok(cid), Ok(mid)) = (parts[2].parse::<i64>(), parts[3].parse::<i32>()) else {
        return;
    };
    let trace_id = parts.get(4).filter(|t| **t != "-").copied();

    // Scope sessions to the originating session (from trace_id) + its
    // children.  This anchors the dashboard to the session that produced
    // the message, not the chat's *current* binding — so `/new` or
    // `/checkout` won't make old Dashboard buttons show the wrong session.
    let all_sessions = handle.list_processes();
    let root_key = match trace_id {
        Some(tid) => handle
            .trace_service()
            .get_session_id(tid)
            .await
            .ok()
            .flatten()
            .and_then(|s| rara_kernel::session::SessionKey::try_from_raw(&s).ok()),
        None => {
            // Fallback for dashboards opened without a trace_id (e.g.
            // future `/dashboard` command): use the chat's current binding.
            let chat_id_str = cid.to_string();
            handle
                .session_index()
                .get_channel_binding(
                    rara_kernel::channel::types::ChannelType::Telegram,
                    &chat_id_str,
                    None,
                )
                .await
                .ok()
                .flatten()
                .map(|b| b.session_key)
        }
    };
    let scoped = match root_key {
        Some(key) => super::dashboard::scoped_sessions(&all_sessions, key),
        None => vec![],
    };

    let text = super::dashboard::render_dashboard(tab, &scoped);
    let keyboard = super::dashboard::dashboard_keyboard(tab, cid, mid, trace_id);

    let _ = bot
        .edit_message_text(ChatId(cid), MessageId(mid), &text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await;
}

/// Listens for new approval requests and sends inline keyboard messages
/// to the originating Telegram chat so the user can approve or deny.
///
/// The chat is resolved from the session's channel binding; falls back to
/// `primary_chat_id` when no binding exists.
async fn approval_listener(
    bot: teloxide::Bot,
    mut rx: tokio::sync::broadcast::Receiver<ApprovalRequest>,
    config: Arc<StdRwLock<TelegramConfig>>,
    session_index: SessionIndexRef,
    shutdown_rx: &mut watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                info!("telegram approval listener: shutting down");
                return;
            }
            result = rx.recv() => {
                let req = match result {
                    Ok(r) => r,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        error!(skipped = n, "telegram approval listener: {n} approval requests lost due to lag — affected tool calls will time out after 120s");
                        continue;
                    }
                };

                // Resolve the originating chat from the session's channel
                // binding; fall back to primary_chat_id when unavailable.
                let binding_chat_id = session_index
                    .get_channel_binding_by_session(&req.session_key)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|b| b.chat_id.parse::<i64>().ok());

                let chat_id = binding_chat_id.or_else(|| {
                    let cfg = config.read().unwrap_or_else(|e| e.into_inner());
                    cfg.primary_chat_id
                });
                let Some(chat_id) = chat_id else {
                    warn!(
                        session_key = %req.session_key,
                        "telegram approval listener: no channel binding and no primary_chat_id configured"
                    );
                    continue;
                };

                let (_display, args_summary_raw) = tool_display_info(&req.tool_name, &req.tool_args);
                let args_summary = crate::tool_display::truncate_summary(&args_summary_raw, 80);
                let mut text = format!(
                    "<b>🛡 Guard Blocked Tool Call</b>\n\n\
                     <b>Tool:</b> <code>{tool}</code>\n",
                    tool = guard_html_escape(&req.tool_name),
                );
                if !args_summary.is_empty() {
                    text.push_str(&format!(
                        "<b>Action:</b> <code>{}</code>\n",
                        guard_html_escape(&args_summary),
                    ));
                }
                if let Some(ctx) = &req.context {
                    text.push_str(&format!(
                        "<b>Context:</b> {}\n",
                        guard_html_escape(ctx),
                    ));
                }
                // Compute expiration time for display
                let expires_at = req
                    .requested_at
                    .checked_add(jiff::SignedDuration::from_secs(req.timeout_secs as i64))
                    .unwrap_or(req.requested_at);
                let requested_str = req.requested_at.strftime("%H:%M:%S");
                let expires_str = expires_at.strftime("%H:%M:%S");

                text.push_str(&format!(
                    "<b>Reason:</b> {summary}\n\
                     <b>Risk:</b> {risk:?}\n\n\
                     ⏱ <b>Requested:</b> {requested}\n\
                     ⏳ <b>Expires:</b> {timeout}s (at {expires})",
                    summary = guard_html_escape(&req.summary),
                    risk = req.risk_level,
                    requested = requested_str,
                    timeout = req.timeout_secs,
                    expires = expires_str,
                ));

                // Keep the info block separate from the action prompt so the
                // expiry task can reuse `text` without the prompt line.
                let display_text = format!("{text}\n\nApprove or deny this action:");

                let keyboard = InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback("✅ Approve", format!("guard:approve:{}", req.id)),
                    InlineKeyboardButton::callback("❌ Deny", format!("guard:deny:{}", req.id)),
                ]]);

                let result = bot
                    .send_message(ChatId(chat_id), &display_text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await;

                match result {
                    Ok(sent_msg) => {
                        // Spawn a delayed task to auto-expire the message when
                        // the approval timeout elapses. The abort handle is
                        // stored so `handle_guard_callback` can cancel it if
                        // the user responds before the timeout.
                        let expiry_bot = bot.clone();
                        let expiry_chat_id = ChatId(chat_id);
                        let expiry_msg_id = sent_msg.id;
                        let timeout_secs = req.timeout_secs;
                        let request_id = req.id;
                        let handle = tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;

                            // Remove our own entry from the map.
                            GUARD_EXPIRY_HANDLES.remove(&request_id);

                            // Collapse to compact one-liner on expiry.
                            let expired_text = "🛡 <b>Guard</b> ⏰ timed out".to_string();
                            let _ = expiry_bot
                                .edit_message_text(expiry_chat_id, expiry_msg_id, expired_text)
                                .parse_mode(ParseMode::Html)
                                .reply_markup(InlineKeyboardMarkup::new(
                                    Vec::<Vec<InlineKeyboardButton>>::new(),
                                ))
                                .await;
                        });

                        GUARD_EXPIRY_HANDLES.insert(request_id, handle.abort_handle());
                    }
                    Err(e) => {
                        warn!(error = %e, "telegram approval listener: failed to send approval prompt");
                    }
                }
            }
        }
    }
}

/// Listens for new user questions from the ask-user tool and sends them as
/// messages to the primary Telegram chat. The sent message ID is tracked in
/// [`PENDING_USER_QUESTIONS`] so that reply-to messages can resolve them.
async fn question_listener(
    bot: teloxide::Bot,
    mut rx: tokio::sync::broadcast::Receiver<UserQuestion>,
    config: Arc<StdRwLock<TelegramConfig>>,
    mgr: UserQuestionManagerRef,
    shutdown_rx: &mut watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                info!("telegram question listener: shutting down");
                return;
            }
            result = rx.recv() => {
                let question = match result {
                    Ok(q) => q,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "telegram question listener: {n} questions lost due to lag");
                        continue;
                    }
                };

                if let Err(e) = dispatch_user_question(&bot, &config, &mgr, question).await {
                    warn!(error = %e, "telegram question listener: dispatch failed");
                }
            }
        }
    }
}

/// Render and route a single [`UserQuestion`] to Telegram, applying
/// sensitivity-based routing, optional inline-keyboard rendering, and
/// installing the pending-question bookkeeping that later resolution paths
/// (reply-to or callback press) depend on.
///
/// Returns `Err` only for unrecoverable dispatch failures (e.g. no target
/// chat available, send_message rejected by Telegram). Identity validation
/// happens at *resolve* time, not dispatch time.
async fn dispatch_user_question(
    bot: &teloxide::Bot,
    config: &Arc<StdRwLock<TelegramConfig>>,
    mgr: &UserQuestionManagerRef,
    question: UserQuestion,
) -> anyhow::Result<()> {
    let primary_chat_id = {
        let cfg = config.read().unwrap_or_else(|e| e.into_inner());
        cfg.primary_chat_id
    };

    // Extract the originating Telegram coordinates (if any). Questions
    // raised from web/cli turns carry a non-Telegram endpoint or none at
    // all — those always route to `primary_chat_id`.
    let origin_tg = match &question.endpoint {
        Some(Endpoint {
            address: EndpointAddress::Telegram { chat_id, thread_id },
            ..
        }) => Some((*chat_id, *thread_id)),
        _ => None,
    };

    // Resolve the destination (route_chat_id, route_thread_id).
    //
    // Sensitive prompts ALWAYS go to `primary_chat_id` (assumed to be the
    // user's DM) and never leak into a shared group/topic. If the prompt
    // originated in a different chat, we post a breadcrumb there so the
    // user knows to check their DM.
    let (route_chat_id, route_thread_id) = if question.sensitive {
        let Some(pm) = primary_chat_id else {
            anyhow::bail!(
                "sensitive question {} dropped — no primary_chat_id configured",
                question.id
            );
        };
        if let Some((origin_chat, origin_thread)) = origin_tg {
            if origin_chat != pm {
                let notice =
                    "🔒 I sent a private question to your DM. Please check there to answer.";
                let breadcrumb =
                    with_thread_id!(bot.send_message(ChatId(origin_chat), notice), origin_thread);
                if let Err(e) = breadcrumb.await {
                    // Breadcrumb failure is non-fatal — the real question still
                    // reaches the user via DM.
                    warn!(
                        error = %e,
                        question_id = %question.id,
                        "telegram: failed to post sensitive-question breadcrumb",
                    );
                }
            }
        }
        (pm, None)
    } else {
        match origin_tg {
            Some(loc) => loc,
            None => {
                let Some(pm) = primary_chat_id else {
                    anyhow::bail!(
                        "question {} dropped — non-Telegram endpoint and no primary_chat_id",
                        question.id
                    );
                };
                (pm, None)
            }
        }
    };

    // Compose the visible prompt. When options are present we will render
    // an inline keyboard below, so the "reply to this message" hint would
    // be misleading — suppress it.
    let body = guard_html_escape(&question.question);
    let text = if question.options.is_some() {
        format!("<b>❓ Agent Question</b>\n\n{body}")
    } else {
        format!("<b>❓ Agent Question</b>\n\n{body}\n\n<i>Reply to this message to answer.</i>",)
    };

    let send_result = if let Some(ref options) = question.options {
        // Inline keyboard: one button per option. `callback_data` encodes
        // the question UUID plus the option index; identity is re-checked
        // on the callback path using `callback.from.id`.
        let keyboard: Vec<Vec<InlineKeyboardButton>> = options
            .iter()
            .enumerate()
            .map(|(idx, label)| {
                vec![InlineKeyboardButton::callback(
                    label.clone(),
                    format!("au:{}:{}", question.id, idx),
                )]
            })
            .collect();
        let markup = InlineKeyboardMarkup::new(keyboard);
        let req = with_thread_id!(
            bot.send_message(ChatId(route_chat_id), &text)
                .parse_mode(ParseMode::Html)
                .reply_markup(markup),
            route_thread_id
        );
        req.await
    } else {
        let req = with_thread_id!(
            bot.send_message(ChatId(route_chat_id), &text)
                .parse_mode(ParseMode::Html),
            route_thread_id
        );
        req.await
    };

    let sent_msg = send_result.map_err(|e| anyhow::anyhow!("send_message failed: {e}"))?;
    let prompt_location = (route_chat_id, sent_msg.id.0);

    PENDING_USER_QUESTIONS.insert(
        question.id,
        PendingUserQuestion {
            question_id: question.id,
            question_text: question.question.clone(),
            manager: Arc::clone(mgr),
            expected_platform_user_id: question.expected_platform_user_id.clone(),
            options: question.options.clone(),
            prompt_location,
        },
    );
    PROMPT_MSG_TO_QID.insert(prompt_location, question.id);

    info!(
        question_id = %question.id,
        chat_id = route_chat_id,
        message_id = sent_msg.id.0,
        sensitive = question.sensitive,
        has_options = question.options.is_some(),
        "telegram question listener: sent question",
    );

    // Timeout cleanup (5 min ask-user timeout + grace period) prevents the
    // pending-question maps from growing unbounded when the user never
    // answers.
    let cleanup_qid = question.id;
    let cleanup_loc = prompt_location;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(330)).await;
        PENDING_USER_QUESTIONS.remove(&cleanup_qid);
        PROMPT_MSG_TO_QID.remove(&cleanup_loc);
    });

    Ok(())
}

/// Handle `au:<question_id>:<option_index>` inline-keyboard callbacks
/// emitted by [`dispatch_user_question`] when a question has options.
///
/// Validates `callback.from.id` against the pending question's expected
/// platform user id and rejects mismatches, preventing other group members
/// from pressing answer buttons on behalf of the original asker.
async fn handle_ask_user_callback(
    bot: &teloxide::Bot,
    callback: &teloxide::types::CallbackQuery,
    data: &str,
) {
    // `au:<uuid>:<idx>`
    let rest = match data.strip_prefix("au:") {
        Some(r) => r,
        None => return,
    };
    let Some((qid_str, idx_str)) = rest.split_once(':') else {
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("Malformed callback data")
            .await;
        return;
    };
    let Ok(qid) = Uuid::parse_str(qid_str) else {
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("Invalid question id")
            .await;
        return;
    };
    let Ok(idx) = idx_str.parse::<usize>() else {
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("Invalid option index")
            .await;
        return;
    };

    // Identity check against the asker recorded at dispatch time. We use
    // `callback.from.id` which Telegram guarantees to be the user who
    // pressed the button (platform-level attestation — cannot be spoofed
    // by another group member).
    let caller_pid = callback.from.id.0.to_string();
    {
        let Some(entry) = PENDING_USER_QUESTIONS.get(&qid) else {
            let _ = bot
                .answer_callback_query(callback.id.clone())
                .text("This question is no longer pending.")
                .await;
            return;
        };
        if let Some(ref expected) = entry.expected_platform_user_id {
            if expected != &caller_pid {
                let _ = bot
                    .answer_callback_query(callback.id.clone())
                    .text("Only the original asker can answer this question.")
                    .show_alert(true)
                    .await;
                return;
            }
        }
    }

    // Remove and resolve.
    let Some((_, pending)) = PENDING_USER_QUESTIONS.remove(&qid) else {
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("This question was already answered.")
            .await;
        return;
    };
    PROMPT_MSG_TO_QID.remove(&pending.prompt_location);

    let Some(options) = pending.options.as_ref() else {
        warn!(question_id = %qid, "ask-user callback without options on pending entry");
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("No options recorded for this question.")
            .await;
        return;
    };
    let Some(answer) = options.get(idx).cloned() else {
        // Put it back so the user can retry with a valid button.
        PENDING_USER_QUESTIONS.insert(qid, pending);
        let _ = bot
            .answer_callback_query(callback.id.clone())
            .text("Option index out of range.")
            .await;
        return;
    };

    match pending.manager.resolve(qid, answer.clone()) {
        Ok(()) => {
            info!(question_id = %qid, "telegram: resolved user question via inline callback");
            let (chat_id, msg_id) = pending.prompt_location;
            let answered_text = format!(
                "<b>✅ Answered</b>\n\n<s>{}</s>\n\n<i>→ {}</i>",
                guard_html_escape(&pending.question_text),
                guard_html_escape(&answer),
            );
            // Edit the prompt to strike through and append the selected
            // answer; also strips the inline keyboard by not re-attaching
            // a reply_markup.
            let _ = bot
                .edit_message_text(ChatId(chat_id), MessageId(msg_id), answered_text)
                .parse_mode(ParseMode::Html)
                .await;
            let _ = bot.answer_callback_query(callback.id.clone()).await;
        }
        Err(e) => {
            warn!(error = %e, question_id = %qid, "telegram: failed to resolve ask-user callback");
            let _ = bot
                .answer_callback_query(callback.id.clone())
                .text("Failed to record answer.")
                .await;
        }
    }
}

async fn handle_update(
    update: Update,
    handle: &KernelHandle,
    bot: &teloxide::Bot,
    allowed_chat_ids: &[i64],
    bot_username: &Arc<RwLock<Option<String>>>,
    config: &Arc<StdRwLock<TelegramConfig>>,
    stream_hub: &Arc<RwLock<Option<StreamHubRef>>>,
    active_streams: &Arc<DashMap<i64, StreamingMessage>>,
    command_handlers: &[Arc<dyn CommandHandler>],
    callback_handlers: &[Arc<dyn CallbackHandler>],
    stt_service: &Option<rara_stt::SttService>,
    voice_chat_ids: &Arc<DashSet<i64>>,
    settings: &Arc<dyn SettingsProvider>,
) {
    // Read a snapshot of the runtime config for this update.
    let cfg = match config.read() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    };

    // Handle callback queries by prefix routing:
    //   "guard:*"   → guard approval (approve/deny)
    //   "limit:*"   → tool-call limit continue/stop
    //   "au:*"      → ask-user inline option press
    //   "trace:*"   → execution trace toggle (show/hide detail view)
    //   other       → TODO: convert to RawPlatformMessage for kernel processing
    if let UpdateKind::CallbackQuery(callback) = &update.kind {
        if let Some(data) = &callback.data {
            // guard: callbacks do their own user-level auth internally,
            // so they are routed before the chat-level check.
            if data.starts_with("guard:") {
                handle_guard_callback(handle, bot, callback, data, allowed_chat_ids).await;
                return;
            }
            // limit: callbacks do their own chat-level auth internally.
            if data.starts_with("limit:") {
                handle_tool_call_limit_callback(handle, bot, callback, data, allowed_chat_ids)
                    .await;
                return;
            }
            // au: (ask-user option press) enforces per-question identity
            // (expected_platform_user_id) on its own, which is strictly
            // narrower than the shared chat-level auth below. Route it
            // before the chat gate so a legitimate asker in a shared but
            // unlisted chat can still answer their own prompt.
            if data.starts_with("au:") {
                handle_ask_user_callback(bot, callback, data).await;
                return;
            }

            // Chat-level authorization — applies to trace:, cas:, and all
            // other callback prefixes uniformly.
            let cb_chat_id = callback
                .message
                .as_ref()
                .map(|m| m.chat().id.0)
                .unwrap_or(0);
            if !allowed_chat_ids.is_empty() && !allowed_chat_ids.contains(&cb_chat_id) {
                tracing::warn!(
                    chat_id = cb_chat_id,
                    "telegram adapter: dropping callback from unauthorized chat"
                );
                let _ = bot.answer_callback_query(callback.id.clone()).await;
                return;
            }
            // Group-level auth: mirror the allowed_group_chat_id gate from
            // the normal message path.
            let cb_is_group = callback
                .message
                .as_ref()
                .is_some_and(|m| matches!(m.chat().kind, teloxide::types::ChatKind::Public(..)));
            if cb_is_group {
                if let Some(allowed_id) = cfg.allowed_group_chat_id {
                    if cb_chat_id != allowed_id {
                        tracing::warn!(
                            chat_id = cb_chat_id,
                            allowed_group_chat_id = allowed_id,
                            "telegram adapter: dropping callback from unauthorized group"
                        );
                        let _ = bot.answer_callback_query(callback.id.clone()).await;
                        return;
                    }
                }
            }

            // Prefix-routed callbacks (now behind auth).
            if data.starts_with("trace:") {
                handle_trace_callback(bot, callback, data, handle.trace_service()).await;
                return;
            }
            if data.starts_with("cas:") {
                handle_cascade_callback(bot, callback, data, handle).await;
                return;
            }
            if data.starts_with("dash:") {
                handle_dashboard_callback(bot, callback, data, handle).await;
                return;
            }

            for handler in callback_handlers {
                if data.starts_with(handler.prefix()) {
                    let chat_id = cb_chat_id;
                    let context = rara_kernel::channel::command::CallbackContext {
                        channel_type: ChannelType::Telegram,
                        session_key:  String::new(),
                        user:         ChannelUser {
                            platform_id:  callback.from.id.0.to_string(),
                            display_name: Some(callback.from.first_name.clone()),
                        },
                        data:         data.clone(),
                        message_id:   callback.message.as_ref().map(|m| m.id().0.to_string()),
                        metadata:     {
                            let mut m = HashMap::new();
                            m.insert(
                                "telegram_chat_id".to_string(),
                                serde_json::Value::Number(chat_id.into()),
                            );
                            m
                        },
                    };
                    match handler.handle(&context).await {
                        Ok(CallbackResult::SendMessage { text }) => {
                            let _ = bot
                                .send_message(teloxide::types::ChatId(chat_id), text)
                                .parse_mode(teloxide::types::ParseMode::Html)
                                .await;
                        }
                        Ok(CallbackResult::EditMessage { text }) => {
                            if let Some(msg) = &callback.message {
                                let _ = bot
                                    .edit_message_text(msg.chat().id, msg.id(), text)
                                    .parse_mode(teloxide::types::ParseMode::Html)
                                    .await;
                            }
                        }
                        Ok(CallbackResult::SendMessageWithKeyboard { text, keyboard }) => {
                            let rows: Vec<Vec<teloxide::types::InlineKeyboardButton>> = keyboard
                                .into_iter()
                                .map(|row| {
                                    row.into_iter()
                                        .map(|btn| {
                                            if let Some(url) = btn.url {
                                                teloxide::types::InlineKeyboardButton::url(
                                                    btn.text,
                                                    url.parse().unwrap(),
                                                )
                                            } else {
                                                teloxide::types::InlineKeyboardButton::callback(
                                                    btn.text,
                                                    btn.callback_data.unwrap_or_default(),
                                                )
                                            }
                                        })
                                        .collect()
                                })
                                .collect();
                            let markup = teloxide::types::InlineKeyboardMarkup::new(rows);
                            let _ = bot
                                .send_message(teloxide::types::ChatId(chat_id), text)
                                .parse_mode(teloxide::types::ParseMode::Html)
                                .reply_markup(markup)
                                .await;
                        }
                        Ok(CallbackResult::Ack) => {}
                        Err(e) => {
                            tracing::warn!(
                                %e,
                                prefix = handler.prefix(),
                                "callback handler error"
                            );
                        }
                    }
                    let _ = bot.answer_callback_query(callback.id.clone()).await;
                    return;
                }
            }
        }
        return;
    }

    let msg = match &update.kind {
        UpdateKind::Message(msg) | UpdateKind::EditedMessage(msg) => msg,
        _ => return,
    };

    let chat_id = msg.chat.id.0;
    // Extract forum thread_id early so it is available for command dispatch.
    let mut tg_thread_id: Option<i64> = msg.thread_id.map(|t| i64::from(t.0.0));

    // Detect forum supergroups so we can auto-create a topic when the
    // message lands in General (no thread_id).
    let is_forum_chat = matches!(
        msg.chat.kind,
        ChatKind::Public(ChatPublic {
            kind: PublicChatKind::Supergroup(PublicChatSupergroup { is_forum: true, .. }),
            ..
        })
    );
    // Capture message text before the teloxide `msg` is shadowed by
    // `handle.resolve()` — used as the forum topic name.
    let topic_text: Option<String> = msg.text().map(|t| t.to_owned());
    // Capture the user's original message id before the teloxide `msg`
    // is shadowed — used to reply-link the General notification below.
    let original_msg_id = msg.id;

    // Check if this chat is allowed.
    if !allowed_chat_ids.is_empty() && !allowed_chat_ids.contains(&chat_id) {
        warn!(
            chat_id,
            "telegram adapter: dropping message from unauthorized chat"
        );
        return;
    }

    // --- Reply-to-question interception ---
    // If the user replies to a pending question message, resolve it and skip
    // normal message processing so the reply text isn't ingested as a new
    // conversation turn.
    if let Some(reply) = msg.reply_to_message() {
        let lookup_key = (chat_id, reply.id.0);
        if let Some((_, qid)) = PROMPT_MSG_TO_QID.remove(&lookup_key) {
            let Some((_, pending)) = PENDING_USER_QUESTIONS.remove(&qid) else {
                // Primary entry gone (timed out or already answered); drop
                // the reply and fall through to normal processing.
                return;
            };

            // Identity gate: only the asker may answer. Other members of a
            // shared group/topic see the prompt but must not be able to
            // unblock the agent by replying with, say, a fabricated API
            // key. Re-insert the pending state on rejection so the real
            // asker can still answer later.
            let actual_pid = msg.from.as_ref().map(|u| u.id.0.to_string());
            if let Some(ref expected) = pending.expected_platform_user_id {
                if actual_pid.as_deref() != Some(expected.as_str()) {
                    PENDING_USER_QUESTIONS.insert(qid, pending);
                    PROMPT_MSG_TO_QID.insert(lookup_key, qid);
                    let _ = bot
                        .send_message(
                            ChatId(chat_id),
                            "⚠️ Only the original asker can answer this question.",
                        )
                        .reply_parameters(ReplyParameters::new(msg.id))
                        .await;
                    return;
                }
            }

            // Reject non-text replies — prompt user to reply with text.
            let Some(answer_text) = msg.text() else {
                PENDING_USER_QUESTIONS.insert(qid, pending);
                PROMPT_MSG_TO_QID.insert(lookup_key, qid);
                let _ = bot
                    .send_message(ChatId(chat_id), "⚠️ Please reply with a text message.")
                    .reply_parameters(ReplyParameters::new(msg.id))
                    .await;
                return;
            };

            let answer = answer_text.to_string();
            match pending.manager.resolve(qid, answer) {
                Ok(()) => {
                    info!(
                        question_id = %qid,
                        "telegram: resolved user question via reply"
                    );
                    // Edit the original question message to show it was answered.
                    let answered_text = format!(
                        "<b>✅ Answered</b>\n\n<s>{}</s>",
                        guard_html_escape(&pending.question_text),
                    );
                    let _ = bot
                        .edit_message_text(ChatId(chat_id), reply.id, answered_text)
                        .parse_mode(ParseMode::Html)
                        .await;
                }
                Err(e) => {
                    warn!(error = %e, question_id = %qid, "telegram: failed to resolve question");
                }
            }
            return;
        }
    }

    // --- Group chat authorization ---
    let group_chat = is_group_chat(msg);

    // Determine whether this is a group message where Rara was NOT directly
    // mentioned — these are "proactive candidates" that the kernel will
    // gate behind a lightweight LLM judgment via the GroupMessage event.
    let mut is_group_proactive = false;

    if group_chat {
        let trigger_text = msg.text().or_else(|| msg.caption()).unwrap_or_default();
        let username_guard = bot_username.read().await;
        let username_ref = username_guard.as_deref();

        let is_mentioned = is_group_mention(msg, trigger_text, username_ref)
            || contains_rara_keyword(trigger_text);

        match cfg.group_policy {
            GroupPolicy::Ignore => {
                debug!(chat_id, "group message ignored (group_policy=ignore)");
                return;
            }
            GroupPolicy::MentionOnly => {
                if !is_mentioned {
                    debug!(
                        chat_id,
                        "group message ignored (not mentioned, group_policy=mention_only)"
                    );
                    return;
                }
            }
            GroupPolicy::MentionOrSmallGroup => {
                let is_small = matches!(
                    bot.get_chat_member_count(msg.chat.id).await,
                    Ok(n) if n <= SMALL_GROUP_THRESHOLD
                );
                let directly_addressed = is_small || is_mentioned;
                if !directly_addressed {
                    is_group_proactive = true;
                }
            }
            GroupPolicy::ProactiveJudgment => {
                if !is_mentioned {
                    is_group_proactive = true;
                }
            }
            GroupPolicy::All => {
                // Respond to everything.
            }
        }

        // Check allowed group chat authorization.
        if let Some(allowed_id) = cfg.allowed_group_chat_id {
            if chat_id != allowed_id {
                warn!(
                    chat_id,
                    allowed_group_chat_id = allowed_id,
                    "telegram adapter: dropping group message from unauthorized group"
                );
                let _ = bot
                    .send_message(
                        ChatId(chat_id),
                        "This group is not authorized. Please configure the allowed group chat ID \
                         in the adapter settings.",
                    )
                    .await;
                return;
            }
        }

        drop(username_guard);
    }

    // --- Command dispatch ---
    // Intercept `/command` messages and route them to registered handlers
    // before the message enters the kernel pipeline.
    if let UpdateKind::Message(_) = &update.kind {
        if let Some(text) = msg.text() {
            if text.starts_with('/') && !command_handlers.is_empty() {
                let first_token = text.split_whitespace().next().unwrap_or("");
                // Strip leading `/` and optional `@botname` suffix.
                let cmd_name = first_token
                    .trim_start_matches('/')
                    .split('@')
                    .next()
                    .unwrap_or("");

                if !cmd_name.is_empty() {
                    // Find a handler whose `commands()` list contains this name.
                    let matched_handler = command_handlers
                        .iter()
                        .find(|h| h.commands().iter().any(|def| def.name == cmd_name));

                    if let Some(handler) = matched_handler {
                        let args = text[first_token.len()..].trim_start().to_owned();
                        let info = CommandInfo {
                            name: cmd_name.to_owned(),
                            args,
                            raw: text.to_owned(),
                        };

                        let user_id = msg
                            .from
                            .as_ref()
                            .map(|u| u.id.0.to_string())
                            .unwrap_or_default();
                        let display_name = msg.from.as_ref().and_then(|u| {
                            u.username.clone().or_else(|| Some(u.first_name.clone()))
                        });

                        let mut metadata = HashMap::new();
                        metadata.insert(
                            "telegram_chat_id".to_owned(),
                            serde_json::Value::Number(chat_id.into()),
                        );
                        if let Some(tid) = tg_thread_id {
                            metadata.insert(
                                "telegram_thread_id".to_owned(),
                                serde_json::Value::Number(tid.into()),
                            );
                        }

                        let ctx = CommandContext {
                            channel_type: ChannelType::Telegram,
                            session_key: String::new(),
                            user: ChannelUser {
                                platform_id: user_id,
                                display_name,
                            },
                            metadata,
                        };

                        match handler.handle(&info, &ctx).await {
                            Ok(result) => {
                                dispatch_command_result(bot, chat_id, tg_thread_id, result).await;
                            }
                            Err(e) => {
                                error!(
                                    command = cmd_name,
                                    error = %e,
                                    "telegram adapter: command handler failed"
                                );
                                let req = with_thread_id!(
                                    bot.send_message(
                                        ChatId(chat_id),
                                        format!("Command failed: {e}")
                                    ),
                                    tg_thread_id
                                );
                                let _ = req.await;
                            }
                        }
                        return;
                    }
                    // No handler matched — fall through to normal message
                    // processing.
                }
            }
        }
    }

    // Convert to RawPlatformMessage.
    let username_guard = bot_username.read().await;
    let username_ref = username_guard.as_deref().unwrap_or("");
    let raw = match telegram_to_raw_platform_message(msg, username_ref) {
        Some(raw) => raw,
        None => return,
    };
    drop(username_guard);

    // If the Telegram message has a photo, download and compress it for LLM vision.
    let raw = if let Some(photos) = msg.photo() {
        if let Some(largest) = photos.last() {
            match download_and_compress_photo(bot, &largest.file.id).await {
                Ok((media_type, b64_data, original_path, compressed_path)) => {
                    // Combine text + image into multimodal content.
                    let text = match raw.content {
                        MessageContent::Text(ref t) => t.clone(),
                        _ => String::new(),
                    };
                    let mut blocks = vec![];
                    if !text.is_empty() {
                        blocks.push(rara_kernel::channel::types::ContentBlock::Text { text });
                    }
                    blocks.push(rara_kernel::channel::types::ContentBlock::ImageBase64 {
                        media_type,
                        data: b64_data,
                    });
                    let mut updated_metadata = raw.metadata.clone();
                    updated_metadata.insert(
                        "image_original_path".to_owned(),
                        serde_json::Value::String(original_path.to_string_lossy().into_owned()),
                    );
                    updated_metadata.insert(
                        "image_compressed_path".to_owned(),
                        serde_json::Value::String(compressed_path.to_string_lossy().into_owned()),
                    );
                    RawPlatformMessage {
                        content: MessageContent::Multimodal(blocks),
                        metadata: updated_metadata,
                        ..raw
                    }
                }
                Err(e) => {
                    warn!(error = %e, "failed to download/compress photo, using text only");
                    raw
                }
            }
        } else {
            raw
        }
    } else {
        raw
    };

    // If the message has a voice note or audio attachment, transcribe via STT.
    let raw = if msg.voice().is_some() || msg.audio().is_some() {
        let file_id = msg
            .voice()
            .map(|v| &v.file.id)
            .or_else(|| msg.audio().map(|a| &a.file.id));

        let mime_hint = msg
            .audio()
            .and_then(|a| a.mime_type.as_ref())
            .map(|m| m.as_ref());

        if let (Some(file_id), Some(stt)) = (file_id, stt_service) {
            match download_voice_file(bot, file_id, mime_hint).await {
                Ok((audio_data, mime_type)) => match stt.transcribe(audio_data, &mime_type).await {
                    Ok(text) => {
                        tracing::info!(len = text.len(), "voice message transcribed");
                        // Mark this chat so egress replies with a voice note.
                        voice_chat_ids.insert(chat_id);
                        let combined = match raw.content {
                            MessageContent::Text(ref caption) if !caption.trim().is_empty() => {
                                format!("{caption}\n\n{text}")
                            }
                            _ => text,
                        };
                        RawPlatformMessage {
                            content: MessageContent::Text(combined),
                            ..raw
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            msg_id = msg.id.0,
                            "STT transcription failed, sending placeholder",
                        );
                        RawPlatformMessage {
                            content: MessageContent::Text(
                                "[voice message \u{2014} transcription failed]".to_owned(),
                            ),
                            ..raw
                        }
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "failed to download voice file, skipping");
                    return;
                }
            }
        } else {
            tracing::warn!("voice message received but no STT service configured, skipping");
            return;
        }
    } else {
        raw
    };

    // --- Auto-create forum topic BEFORE resolving session ---
    //
    // In forum-enabled supergroups, a message with no `thread_id` lands in
    // the "General" topic — treat this as "start a new conversation" and
    // create a dedicated topic so that `(chat_id, new_thread_id)` is an
    // unbound channel. `handle.resolve()` will then fall through to
    // `resolve_or_create` and mint a fresh session for the new topic,
    // giving each topic an independent session (ChatGPT-style UX).
    //
    // Creating the topic before `resolve()` (rather than after, as the
    // previous implementation did) is load-bearing: doing it after meant
    // `resolve()` saw `thread_id=None`, reused or created General's session,
    // and then the post-ingest binding rewrite silently pointed the new
    // topic at General's session — leaking context across conversations.
    let raw = if is_forum_chat && tg_thread_id.is_none() {
        // Acquire bot username once so we can sanitize the initial topic
        // name (strip @mentions and /command prefixes).
        let bot_username_snapshot = bot_username.read().await.clone();
        let topic_name =
            derive_initial_topic_name(topic_text.as_deref(), bot_username_snapshot.as_deref());

        match bot.create_forum_topic(ChatId(chat_id), &topic_name).await {
            Ok(topic) => {
                let new_tid = i64::from(topic.thread_id.0.0);
                tg_thread_id = Some(new_tid);

                // Notify the user in General with a clickable deep-link to
                // the newly created topic. Replying to the original message
                // keeps the notification threaded to their request.
                let link = forum_topic_link(chat_id, new_tid);
                let notice = format!(
                    "\u{1f4ac} New topic: <a href=\"{link}\">{name}</a>",
                    name = guard_html_escape(&topic_name),
                );
                let _ = bot
                    .send_message(ChatId(chat_id), notice)
                    .parse_mode(ParseMode::Html)
                    .reply_parameters(ReplyParameters::new(original_msg_id))
                    .await;

                // Send a brief intro in the new topic so the user sees
                // activity there (the original message stays in General —
                // Telegram does not allow moving messages between topics).
                let intro = format!("\u{1f4ac} {topic_name}");
                let _ =
                    with_thread_id!(bot.send_message(ChatId(chat_id), &intro), tg_thread_id).await;

                // Override `thread_id` in the raw message so session
                // resolution keys on `(chat_id, new_tid)` — unbound, so a
                // new session is created.
                let mut updated = raw;
                if let Some(ref mut ctx) = updated.reply_context {
                    ctx.thread_id = Some(new_tid.to_string());
                }
                updated
            }
            Err(e) => {
                warn!(error = %e, "failed to create forum topic, replying in General");
                raw
            }
        }
    } else {
        raw
    };

    let msg = match handle.resolve(raw).await {
        Ok(msg) => msg,
        Err(IOError::SystemBusy) => {
            let req = with_thread_id!(
                bot.send_message(
                    ChatId(chat_id),
                    "⚠️ System is busy, please try again later.",
                ),
                tg_thread_id
            );
            let _ = req.await;
            return;
        }
        Err(IOError::RateLimited { message }) => {
            let req = with_thread_id!(
                bot.send_message(ChatId(chat_id), format!("\u{26a0}\u{fe0f} {message}")),
                tg_thread_id
            );
            let _ = req.await;
            return;
        }
        Err(IOError::IdentityResolutionFailed { .. }) => {
            debug!("telegram adapter: unknown platform user, ignoring");
            return;
        }
        Err(other) => {
            error!(error = %other, "telegram adapter: ingest failed");
            return;
        }
    };

    let session_id = msg.session_key_opt().copied();
    let rara_message_id = msg.id.to_string();

    // Route: group proactive candidates go through GroupMessage event for
    // lightweight LLM judgment; directly-addressed messages go through the
    // normal UserMessage path. Both use the async ingest variants so
    // `IOError::Full` is retried with bounded backoff (and exhausted
    // envelopes routed to the dead-letter sink) rather than being silently
    // dropped — see issue #1148.
    let submit_result = if is_group_proactive {
        handle.ingest_group_message(msg).await
    } else {
        handle.ingest_user_message(msg).await
    };

    match submit_result {
        Ok(()) => {
            // Spawn stream forwarder for progressive editMessageText.
            // (Only needed for direct messages — group proactive messages
            // may not produce a reply, but the forwarder is harmless if idle.)
            if let Some(sid) = session_id {
                spawn_stream_forwarder(
                    Arc::clone(stream_hub),
                    Arc::clone(active_streams),
                    bot.clone(),
                    chat_id,
                    tg_thread_id,
                    sid,
                    handle.trace_service().clone(),
                    rara_message_id.clone(),
                    Arc::clone(settings),
                );
            }
        }
        Err(_) => {
            let req = with_thread_id!(
                bot.send_message(ChatId(chat_id), "⚠️ 系统繁忙，请稍后再试。"),
                tg_thread_id
            );
            let _ = req.await;
        }
    }
}

// ---------------------------------------------------------------------------
// Command result dispatch
// ---------------------------------------------------------------------------

/// Send a [`CommandResult`] back to the Telegram chat.
async fn dispatch_command_result(
    bot: &teloxide::Bot,
    chat_id: i64,
    thread_id: Option<i64>,
    result: CommandResult,
) {
    match result {
        CommandResult::Text(text) => {
            let req = with_thread_id!(bot.send_message(ChatId(chat_id), text), thread_id);
            let _ = req.await;
        }
        CommandResult::Html(html) => {
            let req = with_thread_id!(
                bot.send_message(ChatId(chat_id), html)
                    .parse_mode(ParseMode::Html),
                thread_id
            );
            let _ = req.await;
        }
        CommandResult::HtmlWithKeyboard { html, keyboard } => {
            let rows: Vec<Vec<InlineKeyboardButton>> = keyboard
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|btn| {
                            if let Some(url) = btn.url {
                                InlineKeyboardButton::url(btn.text, url.parse().unwrap())
                            } else {
                                InlineKeyboardButton::callback(
                                    btn.text,
                                    btn.callback_data.unwrap_or_default(),
                                )
                            }
                        })
                        .collect()
                })
                .collect();
            let markup = InlineKeyboardMarkup::new(rows);
            let req = with_thread_id!(
                bot.send_message(ChatId(chat_id), html)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(markup),
                thread_id
            );
            let _ = req.await;
        }
        CommandResult::Photo { data, caption } => {
            use teloxide::types::InputFile;

            let mut request = bot.send_photo(ChatId(chat_id), InputFile::memory(data));
            if let Some(caption) = caption {
                request = request.caption(caption);
            }
            let request = with_thread_id!(request, thread_id);
            let _ = request.await;
        }
        CommandResult::None => {}
    }
}

// ---------------------------------------------------------------------------
// Stream forwarder — progressive editMessageText
// ---------------------------------------------------------------------------

/// Strip all tool-call XML from text: first matched blocks, then orphaned tags.
///
/// Two-pass approach:
/// 1. Remove complete `<tag>…</tag>` blocks (including their content)
/// 2. Remove any remaining orphaned opening/closing tags
///
/// This handles every observed failure mode:
/// - Well-formed blocks (`<tool_call>…</tool_call>`)
/// - Mismatched names (`<toolcall>…</tool_call>`)
/// - Orphaned tags from streaming flush boundaries
/// - LLM-degraded XML emitted as plain text
fn strip_tool_call_xml(text: &str) -> String {
    let pass1 = TOOL_CALL_BLOCK_RE.replace_all(text, "");
    let pass2 = TOOL_CALL_TAG_RE.replace_all(&pass1, "");
    pass2.into_owned()
}

/// Spawn a background task that subscribes to the stream hub for the given
/// session and progressively updates a Telegram message via `editMessageText`.
fn spawn_stream_forwarder(
    stream_hub: Arc<RwLock<Option<StreamHubRef>>>,
    active_streams: Arc<DashMap<i64, StreamingMessage>>,
    bot: teloxide::Bot,
    chat_id: i64,
    thread_id: Option<i64>,
    session_id: rara_kernel::session::SessionKey,
    trace_service: rara_kernel::trace::TraceService,
    rara_message_id: String,
    settings: Arc<dyn SettingsProvider>,
) {
    use rara_kernel::io::{PlanStepStatus, StreamEvent};

    tokio::spawn(async move {
        let hub = {
            let guard = stream_hub.read().await;
            match guard.as_ref() {
                Some(hub) => Arc::clone(hub),
                None => return,
            }
        };

        // Poll until stream appears (event_loop opens it asynchronously).
        let mut attempts = 0;
        let subs = loop {
            let s = hub.subscribe_session(&session_id);
            if !s.is_empty() || attempts > 50 {
                break s;
            }
            attempts += 1;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };

        if subs.is_empty() {
            tracing::info!(session_id = %session_id, attempts, "tg stream forwarder: no streams found after polling");
            return;
        }

        tracing::info!(session_id = %session_id, attempts, stream_count = subs.len(), "tg stream forwarder: subscribed");

        // Initialize streaming state and capture its epoch so the delayed
        // cleanup task below can distinguish this turn's state from a
        // successor turn that may re-insert under the same chat_id.
        let my_epoch = {
            let fresh = StreamingMessage::new();
            let epoch = fresh.epoch;
            active_streams.insert(chat_id, fresh);
            epoch
        };

        // Handle the first stream (one agent turn per ingest).
        let (_stream_id, mut rx) = match subs.into_iter().next() {
            Some(s) => s,
            None => return,
        };

        let mut throttle = tokio::time::interval(MIN_EDIT_INTERVAL);
        throttle.tick().await; // skip immediate first tick

        let mut typing_interval = tokio::time::interval(std::time::Duration::from_secs(4));
        typing_interval.tick().await; // skip immediate first tick

        let mut progress = ProgressMessage::new(rara_message_id);
        let mut progress_dirty = false;

        // Pinned session card — a stable summary pinned to the chat top.
        // Restore persisted message ID so we can continue editing after restart.
        let session_label = session_id.to_string();
        let mut pinned = super::pinned_status::PinnedSessionCard::new(
            chat_id,
            session_label.clone(),
            session_label,
        );
        let pinned_settings_key = format!("telegram.pinned_message.{chat_id}");
        if let Some(raw) = settings.get(&pinned_settings_key).await {
            if let Ok(id) = raw.parse::<i32>() {
                pinned.message_id = Some(MessageId(id));
            }
        }

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(StreamEvent::TextDelta { text }) => {
                            // Check if we need to flush due to threshold.
                            let flush_req = {
                                if let Some(mut state) = active_streams.get_mut(&chat_id) {
                                    state.accumulated.push_str(&text);
                                    state.dirty = true;

                                    if state.accumulated.len() > STREAM_SPLIT_THRESHOLD {
                                        // 剥离 LLM 可能泄漏到 content 中的 tool call XML
                                        let cleaned = strip_tool_call_xml(&state.accumulated);
                                        let split_chars = cleaned.chars().count();
                                        let html = crate::telegram::markdown::markdown_to_telegram_html(&cleaned);
                                        Some(FlushRequest {
                                            message_ids: state.message_ids.clone(),
                                            text_html: html,
                                            split_chars,
                                        })
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                                // Guard dropped here.
                            };

                            if let Some(req) = flush_req {
                                let result = flush_edit(&bot, chat_id, thread_id, &req).await;
                                let split_applied =
                                    matches!(result, FlushResult::Sent(_) | FlushResult::Edited);
                                apply_flush_result(&active_streams, chat_id, result);
                                // Start a new message for overflow.
                                if split_applied {
                                    if let Some(mut state) = active_streams.get_mut(&chat_id) {
                                        state.streamed_prefix_chars =
                                            state.streamed_prefix_chars.saturating_add(req.split_chars);
                                        state.accumulated.clear();
                                        state.message_ids.push(MessageId(0)); // sentinel
                                        state.dirty = false;
                                    }
                                }
                            }
                        }
                        Ok(StreamEvent::TurnRationale { text }) => {
                            progress.turn_rationale = Some(text);
                            progress_dirty = true;
                        }
                        Ok(StreamEvent::ToolCallStart { name, id, arguments }) => {
                            // Transition out of thinking phase.
                            progress.thinking = false;
                            pinned.on_tool_start();

                            let (display, summary) = tool_display_info(&name, &arguments);
                            let activity = tool_activity_label(&name).to_owned();
                            progress.tools.push(ToolProgress {
                                id,
                                raw_name: name,
                                name: display,
                                activity,
                                summary,
                                started_at: Instant::now(),
                                finished: false,
                                success: false,
                                duration: None,
                                error: None,
                                result_hint: None,
                            });

                            // Send typing indicator before the first progress message.
                            if progress.message_id.is_none() {
                                let req = with_thread_id!(bot
                                    .send_chat_action(ChatId(chat_id), ChatAction::Typing), thread_id);
                                let _ = req.await;
                            }

                            let text = progress.render_text();
                            if progress.last_edit.elapsed() >= MIN_EDIT_INTERVAL {
                                match progress.message_id {
                                    Some(mid) => {
                                        let _ = bot
                                            .edit_message_text(ChatId(chat_id), mid, &text)
                                            .await;
                                    }
                                    None => {
                                        let req = with_thread_id!(bot
                                            .send_message(ChatId(chat_id), &text), thread_id);
                                        if let Ok(msg) = req.await
                                        {
                                            progress.message_id = Some(msg.id);
                                        }
                                    }
                                }
                                progress.last_edit = Instant::now();
                                progress_dirty = false;
                            } else {
                                progress_dirty = true;
                            }
                        }
                        Ok(StreamEvent::ToolCallEnd { id, result_preview, success, error }) => {
                            if let Some(tp) = progress.tools.iter_mut().find(|t| t.id == id) {
                                tp.finished = true;
                                tp.success = success;
                                tp.duration = Some(tp.started_at.elapsed());
                                tp.error = error;
                                tp.result_hint =
                                    crate::tool_display::tool_result_hint(&tp.raw_name, &result_preview);
                            }
                            pinned.on_tool_end();

                            let text = progress.render_text();
                            if progress.last_edit.elapsed() >= MIN_EDIT_INTERVAL {
                                match progress.message_id {
                                    Some(mid) => {
                                        let _ = bot
                                            .edit_message_text(ChatId(chat_id), mid, &text)
                                            .await;
                                    }
                                    None => {
                                        let req = with_thread_id!(bot
                                            .send_message(ChatId(chat_id), &text), thread_id);
                                        if let Ok(msg) = req.await
                                        {
                                            progress.message_id = Some(msg.id);
                                        }
                                    }
                                }
                                progress.last_edit = Instant::now();
                                progress_dirty = false;
                            } else {
                                progress_dirty = true;
                            }
                        }
                        Ok(StreamEvent::TextClear) => {
                            // Non-blocking narration clear: extract message IDs
                            // synchronously, then spawn deletion in background so
                            // we return to rx.recv() immediately and never lag.
                            let narration_msg_ids: Vec<MessageId> = {
                                if let Some(mut state) = active_streams.get_mut(&chat_id) {
                                    let ids: Vec<MessageId> = state.message_ids
                                        .iter()
                                        .copied()
                                        .filter(|mid| mid.0 != 0)
                                        .collect();
                                    state.message_ids.clear();
                                    state.accumulated.clear();
                                    state.streamed_prefix_chars = 0;
                                    state.dirty = false;
                                    ids
                                } else {
                                    Vec::new()
                                }
                                // Guard dropped here.
                            };
                            if !narration_msg_ids.is_empty() {
                                let bot_bg = bot.clone();
                                tokio::spawn(async move {
                                    for mid in narration_msg_ids {
                                        let _ = bot_bg.delete_message(ChatId(chat_id), mid).await;
                                    }
                                });
                            }
                        }
                        Ok(StreamEvent::PlanCreated { total_steps, goal, .. }) => {
                            if total_steps <= 1 {
                                // Micro: single-step plan, behave like reactive.
                                continue;
                            }
                            let steps: Vec<PlanStepState> = (0..total_steps)
                                .map(|_| PlanStepState {
                                    task: String::new(),
                                    status: StepStatus::Pending,
                                })
                                .collect();
                            progress.plan_steps = Some(steps);
                            progress.plan_goal = Some(goal.clone());

                            // Send initial plan message immediately.
                            let text = progress.render_text();
                            if !text.is_empty() {
                                let req = with_thread_id!(bot.send_message(ChatId(chat_id), &text), thread_id);
                                match req.await {
                                    Ok(msg) => { progress.message_id = Some(msg.id); }
                                    Err(e) => { warn!(chat_id, error = %e, "failed to send plan progress message"); }
                                }
                                progress.last_edit = Instant::now();
                            }
                        }
                        Ok(StreamEvent::PlanProgress { current_step, step_status, status_text, .. }) => {
                            if progress.plan_steps.is_some() {
                                // Detect step transition: clear tools when step changes.
                                if Some(current_step) != progress.plan_current_step {
                                    if let Some(ref mut steps) = progress.plan_steps {
                                        if let Some(prev_idx) = progress.plan_current_step {
                                            if let Some(prev) = steps.get_mut(prev_idx) {
                                                if matches!(prev.status, StepStatus::Running) {
                                                    prev.status = StepStatus::Done;
                                                }
                                            }
                                        }
                                    }
                                    progress.tools.clear();
                                    progress.plan_current_step = Some(current_step);
                                }

                                // Update current step using structured status.
                                if let Some(ref mut steps) = progress.plan_steps {
                                    // Dynamically extend steps if replan introduced new indices.
                                    while current_step >= steps.len() {
                                        steps.push(PlanStepState {
                                            task:   String::new(),
                                            status: StepStatus::Pending,
                                        });
                                    }
                                    if let Some(step) = steps.get_mut(current_step) {
                                        step.status = match &step_status {
                                            PlanStepStatus::Running => StepStatus::Running,
                                            PlanStepStatus::Done => StepStatus::Done,
                                            PlanStepStatus::Failed { reason } => StepStatus::Failed(reason.clone()),
                                            PlanStepStatus::NeedsReplan { reason } => StepStatus::Failed(reason.clone()),
                                        };
                                        // Extract task name from status_text on first Running.
                                        if step.task.is_empty() {
                                            if let Some(colon_pos) = status_text.find('\u{ff1a}') {
                                                let task: String = status_text[colon_pos + '\u{ff1a}'.len_utf8()..]
                                                    .trim_end_matches('\u{2026}')
                                                    .to_string();
                                                step.task = task;
                                            } else {
                                                step.task = status_text.clone();
                                            }
                                        }
                                    }
                                }

                                // Render and edit with throttle + dirty flag.
                                let text = progress.render_text();
                                if progress.last_edit.elapsed() >= MIN_EDIT_INTERVAL {
                                    if let Some(mid) = progress.message_id {
                                        let _ = bot.edit_message_text(ChatId(chat_id), mid, &text).await;
                                    }
                                    progress.last_edit = Instant::now();
                                    progress_dirty = false;
                                } else {
                                    progress_dirty = true;
                                }
                            }
                        }
                        Ok(StreamEvent::PlanReplan { reason }) => {
                            // Kernel behavior: after PlanReplan, the kernel replaces
                            // plan.steps with new steps re-indexed from
                            // base_index = past_steps.len(). It does NOT re-send
                            // PlanCreated. Subsequent PlanProgress events carry the
                            // new (higher) step indices, handled by dynamic expansion
                            // in the PlanProgress handler above.
                            if let Some(ref mut steps) = progress.plan_steps {
                                // Mark the current step as failed.
                                if let Some(cur) = progress.plan_current_step {
                                    if let Some(step) = steps.get_mut(cur) {
                                        step.status = StepStatus::Failed(reason.clone());
                                    }
                                }
                                // Remove remaining Pending steps — they won't execute
                                // after replan; new steps arrive via PlanProgress with
                                // higher indices and are dynamically expanded.
                                steps.retain(|s| !matches!(s.status, StepStatus::Pending));

                                let text = progress.render_text();
                                if let Some(mid) = progress.message_id {
                                    let _ = bot.edit_message_text(ChatId(chat_id), mid, &text).await;
                                }
                                progress.last_edit = Instant::now();
                            }
                        }
                        Ok(StreamEvent::PlanCompleted { summary }) => {
                            if progress.plan_steps.is_some() {
                                // Mark running steps as done; leave pending steps as-is
                                // (they were never started, so marking them "done" would
                                // be misleading).
                                if let Some(ref mut steps) = progress.plan_steps {
                                    for step in steps.iter_mut() {
                                        if matches!(step.status, StepStatus::Running) {
                                            step.status = StepStatus::Done;
                                        }
                                    }
                                    // Save plan steps for trace detail view.
                                    progress.saved_plan_steps = steps
                                        .iter()
                                        .enumerate()
                                        .map(|(i, s)| {
                                            let icon = match &s.status {
                                                StepStatus::Done => "\u{2705}",
                                                StepStatus::Failed(_) => "\u{274c}",
                                                _ => "\u{2b1c}",
                                            };
                                            format!("{icon} \u{7b2c}{}\u{6b65}\u{ff1a}{}", i + 1, s.task)
                                        })
                                        .collect();
                                    // Append completion summary.
                                    if !summary.is_empty() {
                                        progress.saved_plan_steps.push(format!("\u{2705} {summary}"));
                                    }
                                }

                                // Final render.
                                let text = progress.render_text();
                                if let Some(mid) = progress.message_id {
                                    let _ = bot.edit_message_text(ChatId(chat_id), mid, &text).await;
                                }
                            }
                        }
                        Ok(StreamEvent::UsageUpdate { input_tokens, output_tokens, thinking_ms }) => {
                            progress.input_tokens = input_tokens;
                            progress.output_tokens = output_tokens;
                            progress.thinking_ms = thinking_ms;
                            pinned.on_usage_update(input_tokens, output_tokens, thinking_ms);
                            // Trigger a progress re-render if we have a message
                            if progress.message_id.is_some() || !progress.tools.is_empty() {
                                let text = progress.render_text();
                                if progress.last_edit.elapsed() >= MIN_EDIT_INTERVAL {
                                    if let Some(mid) = progress.message_id {
                                        let _ = bot
                                            .edit_message_text(ChatId(chat_id), mid, &text)
                                            .await;
                                    }
                                    progress.last_edit = Instant::now();
                                } else {
                                    progress_dirty = true;
                                }
                            }
                        }
                        Ok(StreamEvent::ReasoningDelta { text }) => {
                            // Show thinking feedback on first reasoning token.
                            if !progress.thinking {
                                progress.thinking = true;
                                // Send initial thinking message immediately so
                                // the user sees feedback instead of silence.
                                if progress.message_id.is_none() {
                                    let req = with_thread_id!(bot
                                        .send_chat_action(ChatId(chat_id), ChatAction::Typing), thread_id);
                                    let _ = req.await;
                                    let text = progress.render_text();
                                    if !text.is_empty() {
                                        let req = with_thread_id!(bot
                                            .send_message(ChatId(chat_id), &text), thread_id);
                                        if let Ok(msg) = req.await
                                        {
                                            progress.message_id = Some(msg.id);
                                        }
                                    }
                                    progress.last_edit = Instant::now();
                                }
                            }

                            // Collect reasoning preview for trace detail view.
                            // Hard-truncated to ~500 chars to bound memory; the
                            // full reasoning stays in the kernel's TurnTrace.
                            // Uses .chars().count() for char-level limit to avoid
                            // panic on slicing multi-byte UTF-8 (中文, emoji, etc).
                            let current_chars = progress.reasoning_preview.chars().count();
                            if current_chars < 500 {
                                let remaining = 500 - current_chars;
                                let text_chars = text.chars().count();
                                if text_chars <= remaining {
                                    progress.reasoning_preview.push_str(&text);
                                } else {
                                    let safe_end: String = text.chars().take(remaining).collect();
                                    progress.reasoning_preview.push_str(&safe_end);
                                    progress.reasoning_preview.push('\u{2026}');
                                }
                            }

                            // Mark dirty so the periodic flush updates the
                            // progress message with the reasoning preview.
                            progress_dirty = true;
                        }
                        Ok(StreamEvent::TurnMetrics { model, iterations, .. }) => {
                            // TurnMetrics arrives just before stream close —
                            // stash for the ExecutionTrace built in RecvError::Closed.
                            progress.model = model;
                            pinned.on_turn_metrics(progress.model.clone());
                            progress.iterations = iterations;
                        }
                        // Tool call limit: send inline keyboard with continue/stop
                        // buttons. The callback data encodes session_key and
                        // limit_id so handle_tool_call_limit_callback can route the
                        // decision back to the correct oneshot channel.
                        Ok(StreamEvent::ToolCallLimit { session_key, limit_id, tool_calls_made, elapsed_secs }) => {
                            let text = format!(
                                "⚠️ <b>Agent Paused</b>\n\n\
                                 已执行 <b>{tool_calls_made}</b> 次工具调用（耗时 {elapsed_secs}s）。\n\
                                 是否继续？",
                            );
                            let keyboard = InlineKeyboardMarkup::new(vec![vec![
                                InlineKeyboardButton::callback(
                                    "▶️ 继续",
                                    format!("limit:continue:{session_key}:{limit_id}"),
                                ),
                                InlineKeyboardButton::callback(
                                    "⏹ 停止",
                                    format!("limit:stop:{session_key}:{limit_id}"),
                                ),
                            ]]);
                            let req = with_thread_id!(bot
                                .send_message(ChatId(chat_id), &text)
                                .parse_mode(ParseMode::Html)
                                .reply_markup(keyboard), thread_id);
                            let result = req.await;
                            if let Err(e) = result {
                                warn!(error = %e, "forward_stream: failed to send tool call limit prompt");
                            }
                        }
                        Ok(StreamEvent::ToolCallLimitResolved { .. }) => {
                            // Informational only — already handled by callback.
                        }
                        // ToolOutput is a live preview (e.g. bash stdout) — Telegram
                        // messages cannot be updated fast enough for streaming.
                        Ok(StreamEvent::ToolOutput { .. }) => {}
                        Ok(StreamEvent::BackgroundTaskStarted { task_id, agent_name, description }) => {
                            pinned.on_background_task_started(task_id.clone(), agent_name.clone());
                            progress.background_tasks.push(BackgroundTaskState {
                                task_id,
                                agent_name,
                                description,
                                started_at: Instant::now(),
                                finished: false,
                                status: None,
                            });
                            progress_dirty = true;
                        }
                        Ok(StreamEvent::BackgroundTaskDone { task_id, status }) => {
                            pinned.on_background_task_done(&task_id);
                            if let Some(task) = progress.background_tasks.iter_mut().find(|t| t.task_id == task_id) {
                                task.finished = true;
                                task.status = Some(status);
                                progress_dirty = true;
                            }
                        }
                        // Progress, DockTurnComplete, LoopBreakerTriggered
                        // — no Telegram UX for these.
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!(chat_id, skipped = n, "telegram stream forwarder lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            // Stream closed — do final flush.
                            let stream_state_snapshot = active_streams.get(&chat_id).map(|s| {
                                format!(
                                    "message_ids={:?} accumulated_len={} prefix_chars={} dirty={}",
                                    s.message_ids, s.accumulated.len(), s.streamed_prefix_chars, s.dirty,
                                )
                            });
                            tracing::info!(
                                chat_id,
                                state = ?stream_state_snapshot,
                                "tg stream forwarder: stream closed, doing final flush"
                            );
                            let flush_req = {
                                if let Some(state) = active_streams.get(&chat_id) {
                                    if state.dirty {
                                        // 剥离 LLM 可能泄漏到 content 中的 tool call XML
                                        let cleaned = strip_tool_call_xml(&state.accumulated);
                                        let html = crate::telegram::markdown::markdown_to_telegram_html(&cleaned);
                                        Some(FlushRequest {
                                            message_ids: state.message_ids.clone(),
                                            text_html: html,
                                            split_chars: 0,
                                        })
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                                // Guard dropped here.
                            };
                            if let Some(req) = flush_req {
                                let result = flush_edit(&bot, chat_id, thread_id, &req).await;
                                apply_flush_result(&active_streams, chat_id, result);
                            }

                            // ── Pinned status bar: final flush ──
                            pinned.on_stream_close();
                            flush_pinned_status(&bot, chat_id, thread_id, &mut pinned, &settings, &pinned_settings_key).await;

                            // ── Finalize: always create trace + compact summary ──
                            // Every agent turn (including pure text replies) gets a
                            // compact summary with trace/cascade buttons. If no
                            // progress message exists yet, send a new one.
                            {
                                let plan_steps = std::mem::take(&mut progress.saved_plan_steps);

                                let trace = ExecutionTrace {
                                    duration_secs:    progress.turn_started.elapsed().as_secs(),
                                    iterations:       progress.iterations,
                                    model:            std::mem::take(&mut progress.model),
                                    input_tokens:     progress.input_tokens,
                                    output_tokens:    progress.output_tokens,
                                    thinking_ms:      progress.thinking_ms,
                                    thinking_preview: std::mem::take(&mut progress.reasoning_preview),
                                    turn_rationale:   progress.turn_rationale.take(),
                                    plan_steps,
                                    tools:            progress.tools.iter().map(|t| ToolTraceEntry {
                                        name:        t.name.clone(),
                                        duration_ms: t.duration.map(|d| d.as_millis() as u64),
                                        success:     t.success,
                                        summary:     t.summary.clone(),
                                        error:       t.error.clone(),
                                    }).collect(),
                                    rara_message_id:  progress.rara_message_id.clone(),
                                };

                                let compact = render_compact_summary(&trace);
                                // Reuse existing progress message or send a new one
                                // (pure text replies have no progress message yet).
                                let mid = if let Some(mid) = progress.message_id {
                                    mid
                                } else {
                                    let req = with_thread_id!(bot
                                        .send_message(ChatId(chat_id), &compact)
                                        .parse_mode(ParseMode::Html), thread_id);
                                    match req.await
                                    {
                                        Ok(msg) => msg.id,
                                        Err(e) => {
                                            warn!(error = %e, "failed to send trace summary");
                                            break;
                                        }
                                    }
                                };

                                // Persist trace to SQLite, then show compact summary
                                // with inline button containing the trace_id.
                                let session_name = session_id.to_string();
                                match trace_service.save(&session_name, &trace).await {
                                    Ok(trace_id) => {
                                        let callback_data = format!(
                                            "trace:show:{}:{}:{trace_id}",
                                            chat_id, mid.0,
                                        );
                                        let cascade_cb = format!(
                                            "cas:show:{}:{}:{trace_id}",
                                            chat_id, mid.0,
                                        );
                                        let mut buttons = vec![
                                            InlineKeyboardButton::callback(
                                                "\u{1f4ca} \u{8be6}\u{60c5}",
                                                callback_data,
                                            ),
                                            InlineKeyboardButton::callback(
                                                "\u{1f50d} Cascade",
                                                cascade_cb,
                                            ),
                                        ];
                                        // Show Dashboard button when background tasks exist.
                                        // Include trace_id so the dashboard can offer a Back button.
                                        if !progress.background_tasks.is_empty() {
                                            let dash_cb = format!(
                                                "dash:tasks:{}:{}:{trace_id}",
                                                chat_id, mid.0,
                                            );
                                            buttons.push(InlineKeyboardButton::callback(
                                                "\u{1f4f1} Dashboard",
                                                dash_cb,
                                            ));
                                        }
                                        let keyboard = InlineKeyboardMarkup::new(vec![buttons]);

                                        let _ = bot
                                            .edit_message_text(ChatId(chat_id), mid, &compact)
                                            .parse_mode(ParseMode::Html)
                                            .reply_markup(keyboard)
                                            .await;
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "failed to persist execution trace");
                                        // For progress messages, edit to show compact
                                        // without buttons. For newly sent messages the
                                        // compact text is already visible.
                                        if progress.message_id.is_some() {
                                            let _ = bot
                                                .edit_message_text(ChatId(chat_id), mid, &compact)
                                                .parse_mode(ParseMode::Html)
                                                .await;
                                        }
                                    }
                                }
                            }

                            break;
                        }
                    }
                }
                _ = throttle.tick() => {
                    let flush_req = {
                        if let Some(state) = active_streams.get(&chat_id) {
                            if state.dirty && !state.accumulated.is_empty() {
                                let html = crate::telegram::markdown::markdown_to_telegram_html(&state.accumulated);
                                Some(FlushRequest {
                                    message_ids: state.message_ids.clone(),
                                    text_html: html,
                                    split_chars: 0,
                                })
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                        // Guard dropped here.
                    };
                    if let Some(req) = flush_req {
                        let result = flush_edit(&bot, chat_id, thread_id, &req).await;
                        apply_flush_result(&active_streams, chat_id, result);
                    }

                    // Flush throttled progress updates.
                    let should_refresh = if progress.message_id.is_some() {
                        // Once a progress message exists, always refresh the elapsed
                        // timer — even after thinking ends and before tools start
                        // (e.g. during LLM API wait or pure text generation).
                        true
                    } else {
                        // No message yet — only create one if there's content to show.
                        progress_dirty && (!progress.tools.is_empty() || progress.thinking)
                    };
                    if should_refresh {
                        let text = progress.render_text();
                        match progress.message_id {
                            Some(mid) => {
                                let _ = bot
                                    .edit_message_text(ChatId(chat_id), mid, &text)
                                    .await;
                            }
                            None => {
                                let req = with_thread_id!(bot
                                    .send_message(ChatId(chat_id), &text), thread_id);
                                if let Ok(msg) = req.await
                                {
                                    progress.message_id = Some(msg.id);
                                }
                            }
                        }
                        progress.last_edit = Instant::now();
                        progress_dirty = false;
                    }

                    // ── Pinned session card: flush on state change only ──
                    if pinned.needs_flush() {
                        flush_pinned_status(&bot, chat_id, thread_id, &mut pinned, &settings, &pinned_settings_key).await;
                    }
                }
                _ = typing_interval.tick() => {
                    let req = with_thread_id!(bot
                        .send_chat_action(ChatId(chat_id), ChatAction::Typing), thread_id);
                    let _ = req.await;
                }
            }
        }

        // Auto-cleanup after 120s if Reply never arrives. Scope the removal
        // to our own epoch so a successor turn that re-inserted under the
        // same chat_id within the window is not evicted.
        let streams = active_streams.clone();
        let cid = chat_id;
        let epoch = my_epoch;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(120)).await;
            let removed = streams.remove_if(&cid, |_, state| state.epoch == epoch);
            if removed.is_some() {
                warn!(
                    chat_id = cid,
                    epoch, "telegram stream forwarder: stale state cleaned up after 120s"
                );
            } else if streams.contains_key(&cid) {
                info!(
                    chat_id = cid,
                    epoch,
                    "telegram stream forwarder: stale state cleanup skipped — entry belongs to a \
                     newer turn"
                );
            }
        });
    });
}

/// Return the suffix after dropping a raw-character prefix.
///
/// If `prefix_chars` exceeds the string length, returns the original string to
/// avoid accidental truncation when the final response diverges from the
/// streamed content.
fn slice_after_char_prefix(content: &str, prefix_chars: usize) -> String {
    if prefix_chars == 0 {
        return content.to_owned();
    }
    let mut boundary = content.len();
    let mut seen = 0usize;
    for (idx, _) in content.char_indices() {
        if seen == prefix_chars {
            boundary = idx;
            break;
        }
        seen += 1;
    }
    if seen < prefix_chars {
        content.to_owned()
    } else if seen == prefix_chars && boundary == content.len() {
        String::new()
    } else {
        content[boundary..].to_owned()
    }
}

/// Return suffix after `prefix` only when `content` actually starts with it.
///
/// This guards against accidental truncation when the final assembled reply
/// diverges from streamed partial text (e.g. model self-correction).
fn slice_after_prefix_if_matches(content: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        return content.to_owned();
    }
    if let Some(rest) = content.strip_prefix(prefix) {
        rest.to_owned()
    } else {
        content.to_owned()
    }
}

/// Data extracted from [`StreamingMessage`] needed for a flush operation.
/// Allows dropping the DashMap guard before making async Telegram API calls.
struct FlushRequest {
    message_ids: Vec<MessageId>,
    text_html:   String,
    split_chars: usize,
}

/// Result of a flush operation — what to update back in state.
enum FlushResult {
    /// First message sent successfully with this ID.
    Sent(MessageId),
    /// Edit succeeded.
    Edited,
    /// Edit failed but not retryable.
    Failed,
    /// Rate limited — keep dirty for retry.
    RateLimited,
    /// Send failed.
    SendFailed,
}

/// Flush the pinned session card to Telegram.
///
/// Three scenarios, in priority order:
///
/// 1. **`message_id` is `Some` and edit succeeds** — normal path; the existing
///    pinned message is updated in-place.
///
/// 2. **`message_id` is `Some` but edit fails** — the persisted message was
///    deleted by the user or expired. We fall through to scenario 3.
///
/// 3. **`message_id` is `None`** (first flush of this turn, or fallback from
///    scenario 2) — send a new message, pin it silently, and persist the new ID
///    so subsequent turns reuse it instead of accumulating orphan messages.
async fn flush_pinned_status(
    bot: &teloxide::Bot,
    chat_id: i64,
    thread_id: Option<i64>,
    pinned: &mut super::pinned_status::PinnedSessionCard,
    settings: &Arc<dyn SettingsProvider>,
    settings_key: &str,
) {
    use teloxide::payloads::PinChatMessageSetters;

    let html = pinned.render();
    let need_new_msg = match pinned.message_id {
        Some(mid) => bot
            .edit_message_text(ChatId(chat_id), mid, &html)
            .parse_mode(ParseMode::Html)
            .await
            .is_err(),
        None => true,
    };
    if need_new_msg {
        let req = with_thread_id!(
            bot.send_message(ChatId(chat_id), &html)
                .parse_mode(ParseMode::Html),
            thread_id
        );
        if let Ok(msg) = req.await {
            pinned.message_id = Some(msg.id);
            let _ = bot
                .pin_chat_message(ChatId(chat_id), msg.id)
                .disable_notification(true)
                .await;
            let _ = settings.set(settings_key, &msg.id.0.to_string()).await;
        }
    }
    pinned.mark_flushed();
}

/// Flush accumulated text to Telegram via `sendMessage` (first time) or
/// `editMessageText` (subsequent).
///
/// This function does NOT hold any DashMap guard — the caller must extract
/// the data into a [`FlushRequest`] and drop the guard before calling.
async fn flush_edit(
    bot: &teloxide::Bot,
    chat_id: i64,
    thread_id: Option<i64>,
    req: &FlushRequest,
) -> FlushResult {
    if req.message_ids.is_empty() || req.message_ids.last().copied() == Some(MessageId(0)) {
        // First message or new split — send a new message.
        let req2 = with_thread_id!(
            bot.send_message(ChatId(chat_id), &req.text_html)
                .parse_mode(ParseMode::Html),
            thread_id
        );
        match req2.await {
            Ok(sent) => FlushResult::Sent(sent.id),
            Err(e) => {
                warn!(chat_id, error = %e, "telegram stream: failed to send message");
                FlushResult::SendFailed
            }
        }
    } else {
        let msg_id = *req.message_ids.last().unwrap();
        match bot
            .edit_message_text(ChatId(chat_id), msg_id, &req.text_html)
            .parse_mode(ParseMode::Html)
            .await
        {
            Ok(_) => FlushResult::Edited,
            Err(teloxide::RequestError::Api(ref api_err)) => {
                let err_str = format!("{api_err}");
                if err_str.contains("message is not modified") {
                    FlushResult::Edited
                } else if err_str.contains("Too Many Requests") || err_str.contains("retry after") {
                    warn!(
                        chat_id,
                        "telegram stream: rate limited, will retry next tick"
                    );
                    FlushResult::RateLimited
                } else {
                    warn!(chat_id, error = %api_err, "telegram stream: edit failed");
                    FlushResult::Failed
                }
            }
            Err(e) => {
                warn!(chat_id, error = %e, "telegram stream: edit request failed");
                FlushResult::Failed
            }
        }
    }
}

/// Apply a [`FlushResult`] back to the streaming state in the DashMap.
fn apply_flush_result(
    active_streams: &DashMap<i64, StreamingMessage>,
    chat_id: i64,
    result: FlushResult,
) {
    if let Some(mut state) = active_streams.get_mut(&chat_id) {
        match result {
            FlushResult::Sent(msg_id) => {
                if state.message_ids.last().copied() == Some(MessageId(0)) {
                    *state.message_ids.last_mut().unwrap() = msg_id;
                } else {
                    state.message_ids.push(msg_id);
                }
                state.last_edit = Instant::now();
                state.dirty = false;
            }
            FlushResult::Edited | FlushResult::Failed => {
                state.last_edit = Instant::now();
                state.dirty = false;
            }
            FlushResult::RateLimited => {
                // Leave dirty=true so the next tick retries.
            }
            FlushResult::SendFailed => {
                state.dirty = false;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RawPlatformMessage conversion
// ---------------------------------------------------------------------------

/// Convert a Telegram message to a [`RawPlatformMessage`].
///
/// Extracts user ID, chat ID, text/caption content, and reply context from
/// the Telegram message. Returns `None` if the message has no text content
/// (e.g. stickers, voice notes without caption).
pub fn telegram_to_raw_platform_message(
    msg: &teloxide::types::Message,
    bot_username: &str,
) -> Option<RawPlatformMessage> {
    // Extract text — try text first, then caption (for photos/documents).
    // Photos without caption still produce a valid message (empty text);
    // only skip when there is neither text nor a photo attachment.
    let raw_text = msg.text().or_else(|| msg.caption());
    if raw_text.is_none() && msg.photo().is_none() && msg.voice().is_none() && msg.audio().is_none()
    {
        return None;
    }
    let raw_text = raw_text.unwrap_or_default();

    // Strip @mention from text in group chats.
    let text = if is_group_chat(msg) {
        let username = if bot_username.is_empty() {
            None
        } else {
            Some(bot_username)
        };
        strip_group_mention(raw_text, username)
    } else {
        raw_text.to_owned()
    };

    if text.trim().is_empty()
        && msg.photo().is_none()
        && msg.voice().is_none()
        && msg.audio().is_none()
    {
        return None;
    }

    let platform_user_id = msg
        .from
        .as_ref()
        .map(|u| u.id.0.to_string())
        .unwrap_or_else(|| "unknown".to_owned());

    // Determine the interaction type.
    let interaction_type = if text.starts_with('/') {
        // Extract command name (strip leading '/' and any @botname suffix).
        let cmd_part = text.split_whitespace().next().unwrap_or(&text);
        let cmd_name = cmd_part
            .trim_start_matches('/')
            .split('@')
            .next()
            .unwrap_or("")
            .to_lowercase();
        if cmd_name.is_empty() {
            InteractionType::Message
        } else {
            InteractionType::Command(cmd_name)
        }
    } else {
        InteractionType::Message
    };

    // Build reply context.
    let reply_context = Some(ReplyContext {
        thread_id: msg.thread_id.map(|t| t.to_string()),
        reply_to_platform_msg_id: msg.reply_to_message().map(|r| r.id.0.to_string()),
        interaction_type,
    });

    // Build metadata (adapter-specific).
    let mut metadata = HashMap::new();
    if let Some(ref user) = msg.from {
        if let Some(ref username) = user.username {
            metadata.insert(
                "telegram_username".to_owned(),
                serde_json::Value::String(username.clone()),
            );
        }
        // Include display name for downstream enrichment.
        let display_name = if let Some(ref last) = user.last_name {
            format!("{} {last}", user.first_name)
        } else {
            user.first_name.clone()
        };
        metadata.insert(
            "telegram_display_name".to_owned(),
            serde_json::Value::String(display_name),
        );
    }
    if !bot_username.is_empty() {
        metadata.insert(
            "telegram_bot_username".to_owned(),
            serde_json::Value::String(bot_username.to_owned()),
        );
    }

    Some(RawPlatformMessage {
        channel_type: ChannelType::Telegram,
        platform_message_id: Some(msg.id.0.to_string()),
        platform_user_id,
        platform_chat_id: Some(msg.chat.id.0.to_string()),
        content: MessageContent::Text(text),
        reply_context,
        metadata,
    })
}

/// Download a photo from Telegram and compress it for LLM vision input.
///
/// Returns `(media_type, base64_data, original_path, compressed_path)`.
/// Both the original and compressed images are saved to `images_dir()`.
async fn download_and_compress_photo(
    bot: &teloxide::Bot,
    file_id: &teloxide::types::FileId,
) -> anyhow::Result<(String, String, std::path::PathBuf, std::path::PathBuf)> {
    use base64::Engine;
    use rara_kernel::llm::image::{DEFAULT_MAX_EDGE, DEFAULT_QUALITY};
    use teloxide::net::Download;

    let file = bot.get_file(file_id.clone()).send().await?;
    let mut buf = Vec::new();
    bot.download_file(&file.path, &mut buf).await?;

    let (compressed, media_type) =
        rara_kernel::llm::image::compress_image(&buf, DEFAULT_MAX_EDGE, DEFAULT_QUALITY)?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&compressed);

    // Determine file extension from media type.
    let ext = match media_type.as_str() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        _ => "bin",
    };

    // Save original and compressed images to images_dir.
    let images_dir = rara_paths::images_dir();
    tokio::fs::create_dir_all(images_dir).await?;

    let id = uuid::Uuid::new_v4();
    let original_path = images_dir.join(format!("photo_{id}.{ext}"));
    let compressed_path = images_dir.join(format!("photo_{id}_compressed.{ext}"));

    tokio::fs::write(&original_path, &buf).await?;
    tokio::fs::write(&compressed_path, &compressed).await?;

    tracing::info!(
        original = %original_path.display(),
        compressed = %compressed_path.display(),
        "saved uploaded photo to images_dir"
    );

    Ok((media_type, b64, original_path, compressed_path))
}

/// Maximum voice file size (25 MB). Telegram limits normal users to 20 MB,
/// but premium users can send larger files.
const MAX_VOICE_FILE_SIZE: u32 = 25 * 1024 * 1024;

/// Download a voice/audio file from Telegram and return the raw bytes + MIME
/// type.
async fn download_voice_file(
    bot: &teloxide::Bot,
    file_id: &teloxide::types::FileId,
    mime_hint: Option<&str>,
) -> anyhow::Result<(Vec<u8>, String)> {
    use teloxide::net::Download;

    let file = bot.get_file(file_id.clone()).send().await?;

    // Reject files that exceed the size limit.
    if file.size > MAX_VOICE_FILE_SIZE {
        anyhow::bail!(
            "voice file too large: {} bytes (max {MAX_VOICE_FILE_SIZE} bytes)",
            file.size,
        );
    }

    let mut buf = Vec::new();
    bot.download_file(&file.path, &mut buf).await?;

    // Telegram voice messages are OGG/Opus by default.
    let mime_type = mime_hint.unwrap_or("audio/ogg").to_owned();
    tracing::debug!(size = buf.len(), mime = %mime_type, "downloaded voice file");

    Ok((buf, mime_type))
}

pub fn format_session_key(chat_id: i64) -> String { format!("tg:{chat_id}") }

/// Parse a chat ID from a session key.
///
/// Supports the canonical `tg:{chat_id}` format as well as plain numeric
/// strings for convenience.
pub fn parse_chat_id(session_key: &str) -> Result<i64, KernelError> {
    let id_str = session_key.strip_prefix("tg:").unwrap_or(session_key);
    id_str.parse::<i64>().map_err(|_| KernelError::Other {
        message: format!("invalid telegram session key: {session_key}").into(),
    })
}

/// Parse a string into a teloxide [`MessageId`].
pub fn parse_message_id(id: &str) -> Result<MessageId, KernelError> {
    id.parse::<i32>()
        .map(MessageId)
        .map_err(|_| KernelError::Other {
            message: format!("invalid telegram message id: {id}").into(),
        })
}

/// Convert a kernel [`ReplyMarkup`] to a teloxide [`InlineKeyboardMarkup`].
///
/// Returns `None` if the input is `None` or [`ReplyMarkup::RemoveKeyboard`]
/// (which cannot be represented as an inline keyboard).
fn convert_reply_markup(markup: &Option<ReplyMarkup>) -> Option<InlineKeyboardMarkup> {
    match markup {
        Some(ReplyMarkup::InlineKeyboard { rows }) => {
            let tg_rows: Vec<Vec<InlineKeyboardButton>> = rows
                .iter()
                .map(|row| row.iter().map(convert_inline_button).collect())
                .collect();
            Some(InlineKeyboardMarkup::new(tg_rows))
        }
        Some(ReplyMarkup::RemoveKeyboard) | None => None,
    }
}

/// Convert a kernel [`InlineButton`] to a teloxide [`InlineKeyboardButton`].
fn convert_inline_button(button: &InlineButton) -> InlineKeyboardButton {
    if let Some(ref data) = button.callback_data {
        InlineKeyboardButton::callback(&button.text, data)
    } else if let Some(ref url) = button.url {
        match url.parse::<url::Url>() {
            Ok(parsed) => InlineKeyboardButton::url(&button.text, parsed),
            Err(_) => {
                // Fallback to callback with text as data if URL is invalid.
                InlineKeyboardButton::callback(&button.text, &button.text)
            }
        }
    } else {
        // Fallback: use text as callback data.
        InlineKeyboardButton::callback(&button.text, &button.text)
    }
}

fn mime_to_filename(mime: &str) -> String {
    match mime {
        "image/jpeg" | "image/jpg" => "photo.jpg".to_owned(),
        "image/png" => "photo.png".to_owned(),
        "image/gif" => "photo.gif".to_owned(),
        "image/webp" => "photo.webp".to_owned(),
        _ => "photo.bin".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Group chat helpers
// ---------------------------------------------------------------------------

fn is_group_chat(msg: &teloxide::types::Message) -> bool {
    matches!(msg.chat.kind, teloxide::types::ChatKind::Public(..))
}

/// Check whether the message contains an @mention of the bot via message
/// entities or a plain-text `@botname` substring.
fn is_group_mention(
    msg: &teloxide::types::Message,
    text: &str,
    bot_username: Option<&str>,
) -> bool {
    let Some(username) = bot_username else {
        return false;
    };
    let expected = username.to_lowercase();

    // Check structured entities first (most reliable).
    if let Some(entities) = msg.parse_entities() {
        for entity in entities {
            if matches!(entity.kind(), teloxide::types::MessageEntityKind::Mention) {
                let mention = entity.text().trim().trim_start_matches('@').to_lowercase();
                if mention == expected {
                    return true;
                }
            }
        }
    }

    // Fallback: substring check.
    let mention = format!("@{expected}");
    text.to_lowercase().contains(&mention)
}

/// Check whether the text contains any "rara" keyword variant.
///
/// Supported variants: "rara" (case-insensitive), Japanese hiragana/katakana,
/// and Chinese characters.
fn contains_rara_keyword(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("rara")
        || lower.contains("らら")
        || lower.contains("ララ")
        || lower.contains("拉拉")
}

/// Strip the bot @mention from message text.
///
/// Removes the `@botname` substring and trims surrounding whitespace.
fn strip_group_mention(text: &str, bot_username: Option<&str>) -> String {
    let Some(username) = bot_username else {
        return text.trim().to_owned();
    };
    let mention = format!("@{username}");
    text.replace(&mention, "").trim().to_owned()
}

#[cfg(test)]
mod strip_tool_call_xml_tests {
    use super::strip_tool_call_xml;

    #[test]
    fn matched_tags_are_stripped() {
        let input = "Hello <toolcall>grep something</toolcall> world";
        assert_eq!(strip_tool_call_xml(input), "Hello  world");
    }

    #[test]
    fn mismatched_tag_names_are_stripped() {
        let input = "Hello <toolcall>\n<function=grep>\n</function>\n</tool_call> world";
        let result = strip_tool_call_xml(input);
        assert!(!result.contains("toolcall"));
        assert!(!result.contains("tool_call"));
        assert!(!result.contains("function"));
        assert!(result.contains("Hello"));
        assert!(result.contains("world"));
    }

    #[test]
    fn orphaned_opening_tag_is_stripped() {
        let input = "Hello world\n<toolcall>\n<function=grep>";
        let result = strip_tool_call_xml(input);
        assert!(!result.contains("<toolcall>"));
        assert!(!result.contains("<function=grep>"));
    }

    #[test]
    fn orphaned_closing_tag_is_stripped() {
        let input = "</tool_call>\nHello world";
        let result = strip_tool_call_xml(input);
        assert!(!result.contains("</tool_call>"));
        assert_eq!(result.trim(), "Hello world");
    }

    #[test]
    fn self_closing_tag_is_stripped() {
        let input = "Hello <tool_call /> world";
        assert_eq!(strip_tool_call_xml(input), "Hello  world");
    }

    #[test]
    fn clean_text_is_unchanged() {
        let input = "Hello world, nothing to strip here.";
        assert_eq!(strip_tool_call_xml(input), input);
    }

    #[test]
    fn mixed_content_and_orphans() {
        let input = "Before <toolcall>inner</toolcall> middle </tool_call> after";
        let result = strip_tool_call_xml(input);
        assert!(!result.contains("toolcall"));
        assert!(!result.contains("tool_call"));
        assert!(result.contains("Before"));
        assert!(result.contains("after"));
    }
}

#[cfg(test)]
mod render_progress_tests {
    use super::*;

    /// Helper: build a minimal `ProgressMessage` for rendering tests.
    fn test_progress(turn_rationale: Option<&str>) -> ProgressMessage {
        let mut pm = ProgressMessage::new("test-msg-id".into());
        pm.turn_rationale = turn_rationale.map(String::from);
        pm
    }

    /// Helper: build a finished `ToolProgress` entry so the renderer has
    /// something to display.
    fn finished_tool(name: &str) -> ToolProgress {
        ToolProgress {
            id:          "tool-1".into(),
            raw_name:    name.into(),
            name:        name.into(),
            activity:    name.into(),
            summary:     String::new(),
            started_at:  Instant::now(),
            finished:    true,
            success:     true,
            duration:    Some(std::time::Duration::from_millis(100)),
            error:       None,
            result_hint: None,
        }
    }

    #[test]
    fn render_progress_omits_rationale_from_live_progress() {
        // Turn rationale is now shown only in the trace detail, not in live
        // progress — verify it does not appear.
        let pm = test_progress(Some("Reading config files"));
        let tools = vec![finished_tool("read_file")];
        let output = render_progress(&tools, std::time::Duration::from_secs(1), &pm);
        assert!(
            !output.contains("Reading config files"),
            "rationale should not appear in live progress, got: {output}"
        );
    }

    #[test]
    fn render_progress_omits_rationale_when_none() {
        let pm = test_progress(None);
        let tools = vec![finished_tool("read_file")];
        let output = render_progress(&tools, std::time::Duration::from_secs(1), &pm);
        // The thought-bubble emoji prefix used for rationale should be absent.
        assert!(
            !output.contains("\u{1f4ad}"),
            "expected no rationale line, got: {output}"
        );
    }

    #[test]
    fn render_progress_shows_loading_hint_when_thinking() {
        let mut pm = test_progress(None);
        pm.thinking = true;
        let output = render_progress(&[], std::time::Duration::from_secs(2), &pm);
        assert!(
            output.contains(&pm.loading_hint),
            "expected loading hint in output, got: {output}"
        );
        assert!(
            output.contains('\u{2733}'),
            "expected footer with elapsed time, got: {output}"
        );
    }

    #[test]
    fn render_progress_empty_when_not_thinking_and_no_tools() {
        let pm = test_progress(None);
        let output = render_progress(&[], std::time::Duration::from_secs(1), &pm);
        assert!(
            output.is_empty(),
            "expected empty output when not thinking and no tools, got: {output}"
        );
    }

    #[test]
    fn render_progress_shows_tools_not_hint_after_thinking_ends() {
        let mut pm = test_progress(None);
        pm.thinking = false;
        let tools = vec![finished_tool("read_file")];
        let output = render_progress(&tools, std::time::Duration::from_secs(1), &pm);
        assert!(
            !output.contains(&pm.loading_hint),
            "expected no loading hint once tools are present, got: {output}"
        );
        assert!(
            output.contains("read_file"),
            "expected tool name in output, got: {output}"
        );
    }
}

#[cfg(test)]
mod stream_suffix_tests {
    use super::{slice_after_char_prefix, slice_after_prefix_if_matches};

    #[test]
    fn slice_after_prefix_if_matches_returns_suffix() {
        let content = "hello world and beyond";
        let prefix = "hello world";
        assert_eq!(
            slice_after_prefix_if_matches(content, prefix),
            " and beyond"
        );
    }

    #[test]
    fn slice_after_prefix_if_matches_keeps_content_when_not_matched() {
        let content = "hello world and beyond";
        let prefix = "goodbye world";
        assert_eq!(slice_after_prefix_if_matches(content, prefix), content);
    }

    #[test]
    fn slice_after_char_prefix_handles_multibyte_chars() {
        let content = "你好世界abc";
        assert_eq!(slice_after_char_prefix(content, 4), "abc");
    }
}

#[cfg(test)]
mod forum_topic_tests {
    use super::{derive_initial_topic_name, forum_topic_link};

    #[test]
    fn forum_topic_link_strips_100_prefix_from_supergroup_chat_id() {
        let link = forum_topic_link(-1001234567890, 5);
        assert_eq!(link, "https://t.me/c/1234567890/5");
    }

    #[test]
    fn derive_initial_topic_name_falls_back_when_text_missing() {
        assert_eq!(derive_initial_topic_name(None, None), "New chat");
    }

    #[test]
    fn derive_initial_topic_name_falls_back_on_empty_text() {
        assert_eq!(derive_initial_topic_name(Some(""), None), "New chat");
    }

    #[test]
    fn derive_initial_topic_name_keeps_plain_text_verbatim() {
        assert_eq!(derive_initial_topic_name(Some("hello"), None), "hello");
    }

    #[test]
    fn derive_initial_topic_name_strips_bot_mention() {
        assert_eq!(
            derive_initial_topic_name(Some("@rarabot hello world"), Some("rarabot")),
            "hello world"
        );
    }

    #[test]
    fn derive_initial_topic_name_strips_leading_slash_command() {
        assert_eq!(
            derive_initial_topic_name(Some("/new my task"), None),
            "my task"
        );
    }

    #[test]
    fn derive_initial_topic_name_falls_back_when_only_command() {
        assert_eq!(derive_initial_topic_name(Some("/new"), None), "New chat");
    }
}
