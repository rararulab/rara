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

/// Matches complete tool-call XML blocks (open + close, possibly mismatched names).
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
use dashmap::DashMap;
use rara_kernel::{
    channel::{
        adapter::ChannelAdapter,
        command::{CallbackHandler, CommandContext, CommandHandler, CommandInfo, CommandResult},
        types::{ChannelType, ChannelUser, GroupPolicy, InlineButton, MessageContent, ReplyMarkup},
    },
    error::KernelError,
    handle::KernelHandle,
    io::{
        EgressError, Endpoint, EndpointAddress, IOError, InteractionType, PlatformOutbound,
        RawPlatformMessage, ReplyContext, StreamHubRef,
    },
    security::{ApprovalDecision, ApprovalRequest},
};
use teloxide::{
    payloads::{
        AnswerCallbackQuerySetters, EditMessageTextSetters, GetUpdatesSetters, SendMessageSetters,
        SendPhotoSetters,
    },
    requests::{Request, Requester},
    types::{
        AllowedUpdate, ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, MessageId,
        ParseMode, ReplyParameters, Update, UpdateKind,
    },
};
use tokio::sync::{RwLock, watch};
use tracing::{debug, error, info, warn};

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
    id: String,
    name: String,
    activity: String,
    summary: String,
    started_at: Instant,
    finished: bool,
    success: bool,
    duration: Option<std::time::Duration>,
    error: Option<String>,
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
    message_id: Option<MessageId>,
    tools: Vec<ToolProgress>,
    last_edit: Instant,
    turn_started: Instant,
    input_tokens: u32,
    output_tokens: u32,
    thinking_ms: u64,
    /// Accumulated reasoning text for trace (truncated to ~500 chars).
    /// Collected from `StreamEvent::ReasoningDelta`; shown in expanded trace.
    reasoning_preview: String,
    /// Model name, populated from `StreamEvent::TurnMetrics` (arrives before
    /// stream close).
    model: String,
    /// Iteration count, populated from `StreamEvent::TurnMetrics`.
    iterations: usize,
    /// Rara internal message ID — the `InboundMessage.id` that triggered this turn.
    rara_message_id: String,
    /// Plan steps must be saved here because `PlanCompleted` sets `plan =
    /// None`. If we don't save them before that, the trace loses all plan
    /// information.
    saved_plan_steps: Vec<String>,
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
        }
    }
}

/// Snapshot of a completed agent turn, persisted to tape for the inline
/// "📊 详情" toggle.
///
/// Built from `ProgressMessage` fields when the stream closes. Written as a
/// `turn.execution_trace` tape event keyed by session. The lightweight
/// [`TraceIndex`] maps `"{chat_id}:{msg_id}"` to tape coordinates so the
/// callback handler can read it back on demand.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ExecutionTrace {
    duration_secs: u64,
    iterations: usize,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    thinking_ms: u64,
    /// Truncated reasoning text (first ~500 chars).
    thinking_preview: String,
    /// Plan steps with status.
    plan_steps: Vec<String>,
    /// Tool execution records.
    tools: Vec<ToolTraceEntry>,
    /// Rara internal message ID for end-to-end correlation.
    rara_message_id: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ToolTraceEntry {
    name: String,
    /// Duration in milliseconds (serializable replacement for
    /// `std::time::Duration`).
    duration_ms: Option<u64>,
    success: bool,
    summary: String,
    error: Option<String>,
}

/// Pre-rendered trace cache entry. Both views are rendered at trace-write time
/// so that callback handlers need zero I/O and zero computation.
///
/// Trade-off: detail_html is pre-rendered even if most users never click
/// "详情". This costs ~4KB extra per trace but avoids needing the full
/// `ExecutionTrace` struct in the cache (which would be harder to manage).
/// For typical traces this is negligible; if traces become very large,
/// consider lazy rendering on first click.
#[derive(Clone)]
struct TraceCacheEntry {
    compact_html: String,
    detail_html: String,
}

const TRACE_CACHE_CAPACITY: usize = 1000;

/// Two-tier trace index, keyed by `"{chat_id}:{msg_id}"`.
///
/// - `html_cache`: `trace_key → TraceCacheEntry` (pre-rendered HTML).
///   Evicted when exceeding [`TRACE_CACHE_CAPACITY`].
/// - `coord_index`: `trace_key → (tape_name, trace_id)`.
///   Lightweight (~100 bytes/entry), never evicted, persisted to disk so
///   old trace buttons remain functional after process restart.
struct TraceStore {
    /// `"{chat_id}:{msg_id}"` → pre-rendered compact/detail HTML.
    html_cache: DashMap<String, TraceCacheEntry>,
    /// `"{chat_id}:{msg_id}"` → `(tape_name, trace_id)` for tape lookup.
    coord_index: DashMap<String, (String, String)>,
}

impl TraceStore {
    /// Load coord_index from disk, or create empty if file doesn't exist.
    fn load() -> Self {
        let store = Self {
            html_cache: DashMap::new(),
            coord_index: DashMap::new(),
        };
        let path = Self::persist_path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, (String, String)>>(&data) {
                for (k, v) in map {
                    store.coord_index.insert(k, v);
                }
            }
        }
        store
    }

    /// Persist coord_index to disk. Best-effort, errors are logged.
    fn persist(&self) {
        let map: std::collections::HashMap<String, (String, String)> = self
            .coord_index
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect();
        let path = Self::persist_path();
        match serde_json::to_string(&map) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!(error = %e, "failed to persist trace coord index");
                }
            }
            Err(e) => tracing::warn!(error = %e, "failed to serialize trace coord index"),
        }
    }

    fn persist_path() -> std::path::PathBuf {
        rara_paths::data_dir().join("trace_coord_index.json")
    }
}

type TraceIndex = Arc<TraceStore>;

/// Display tier for plan messages in Telegram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanTier {
    /// steps <= 2, est < 10s: no plan message, just let final answer through.
    Micro,
    /// est 10-30s: 1-line status, keep editing.
    Medium,
    /// est > 30s or steps > 4: compact summary + status, single message edit.
    Heavy,
}

/// Plan display state for Telegram — three-tier strategy with single message
/// edit.
struct PlanDisplay {
    message_id: Option<MessageId>,
    total_steps: usize,
    estimated_duration_secs: Option<u32>,
    compact_summary: Option<String>,
    status_lines: Vec<String>,
    last_status: String,
    last_edit: Instant,
    tier: PlanTier,
}

impl PlanDisplay {
    fn new(
        total_steps: usize,
        estimated_duration_secs: Option<u32>,
        compact_summary: String,
    ) -> Self {
        let est = estimated_duration_secs.unwrap_or(0);
        let tier = if total_steps <= 2 && est < 10 {
            PlanTier::Micro
        } else if est <= 30 && total_steps <= 4 {
            PlanTier::Medium
        } else {
            PlanTier::Heavy
        };

        Self {
            message_id: None,
            total_steps,
            estimated_duration_secs,
            compact_summary: if tier == PlanTier::Heavy {
                Some(compact_summary)
            } else {
                None
            },
            status_lines: Vec::new(),
            last_status: String::new(),
            last_edit: Instant::now()
                .checked_sub(MIN_EDIT_INTERVAL)
                .unwrap_or_else(Instant::now),
            tier,
        }
    }

    /// Build the current message text for editing.
    fn render(&self) -> String {
        let mut text = String::new();
        if let Some(ref summary) = self.compact_summary {
            text.push_str(summary);
            text.push('\n');
        }
        if let Some(last) = self.status_lines.last() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(last);
        }
        text
    }
}

/// Format a single tool-progress line.
/// Format a duration as a compact human-readable string.
fn format_duration_compact(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 1 {
        format!("{}ms", d.as_millis())
    } else if secs < 60 {
        format!("{}.{}s", secs, d.subsec_millis() / 100)
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

fn format_tool_line(t: &ToolProgress) -> String {
    if t.finished {
        let time = t
            .duration
            .map(|d| format!(" ({})", format_duration_compact(d)))
            .unwrap_or_default();
        if t.success {
            format!("\u{2705} {}{time}", t.activity)
        } else {
            match &t.error {
                Some(err) => {
                    let short_err: String = err.chars().take(60).collect();
                    format!("\u{274c} {}{time}: {short_err}", t.activity)
                }
                None => format!("\u{274c} {}{time}", t.activity),
            }
        }
    } else {
        format!("正在{}…", t.activity)
    }
}

/// A phase is a group of consecutive tools with the same activity label.
struct Phase {
    activity: String,
    count: usize,
    all_finished: bool,
    all_success: bool,
    total_duration: Option<std::time::Duration>,
    first_error: Option<String>,
}

/// Group consecutive tools by activity label into phases.
fn aggregate_phases(tools: &[ToolProgress]) -> Vec<Phase> {
    let mut phases: Vec<Phase> = Vec::new();

    for tool in tools {
        let merge = phases
            .last()
            .map(|p| p.activity == tool.activity)
            .unwrap_or(false);

        if merge {
            let phase = phases.last_mut().unwrap();
            phase.count += 1;
            phase.all_finished = phase.all_finished && tool.finished;
            phase.all_success = phase.all_success && tool.success;
            if let Some(d) = tool.duration {
                phase.total_duration =
                    Some(phase.total_duration.unwrap_or(std::time::Duration::ZERO) + d);
            }
            if phase.first_error.is_none() {
                phase.first_error = tool.error.clone();
            }
        } else {
            phases.push(Phase {
                activity: tool.activity.clone(),
                count: 1,
                all_finished: tool.finished,
                all_success: tool.success,
                total_duration: tool.duration,
                first_error: tool.error.clone(),
            });
        }
    }

    phases
}

/// Format a single phase line.
fn format_phase_line(phase: &Phase) -> String {
    if phase.all_finished {
        let time = phase
            .total_duration
            .map(|d| format!(" ({})", format_duration_compact(d)))
            .unwrap_or_default();
        if phase.all_success {
            format!("\u{2705} {}{time}", phase.activity)
        } else {
            match &phase.first_error {
                Some(err) => {
                    let short_err: String = err.chars().take(60).collect();
                    format!("\u{274c} {}{time}: {short_err}", phase.activity)
                }
                None => format!("\u{274c} {}{time}", phase.activity),
            }
        }
    } else {
        format!("正在{}…", phase.activity)
    }
}

/// Render tool progress lines for display in Telegram.
///
/// Consecutive tools with the same activity label are aggregated into a single
/// line to avoid noisy repetition (e.g. 3x "检查 MCP" becomes one line).
fn render_progress(
    tools: &[ToolProgress],
    turn_elapsed: std::time::Duration,
    progress: &ProgressMessage,
) -> String {
    if tools.is_empty() {
        return String::new();
    }

    // Aggregate consecutive tools with the same activity into phases.
    let phases = aggregate_phases(tools);
    let mut lines = Vec::new();

    // Count in-progress phases.
    let active = phases.iter().filter(|p| !p.all_finished).count();
    if active > 1 {
        lines.push(format!("\u{26a1} {active} 项任务并行中"));
    }

    let total_phases = phases.len();
    if total_phases <= 5 {
        for phase in &phases {
            lines.push(format_phase_line(phase));
        }
    } else {
        // Compact: collapse older finished phases.
        let finished_phases: Vec<_> = phases.iter().filter(|p| p.all_finished).collect();
        let in_progress_phases: Vec<_> = phases.iter().filter(|p| !p.all_finished).collect();
        let show_last = 2;
        let collapsed = finished_phases.len().saturating_sub(show_last);

        if collapsed > 0 {
            let collapsed_dur: std::time::Duration = finished_phases[..collapsed]
                .iter()
                .filter_map(|p| p.total_duration)
                .sum();
            let dur_str = if collapsed_dur.is_zero() {
                String::new()
            } else {
                format!(" ({})", format_duration_compact(collapsed_dur))
            };
            lines.push(format!("\u{22ef} 已完成 {collapsed} 步{dur_str}"));
        }

        // Last N finished phases.
        for phase in finished_phases.iter().skip(collapsed) {
            lines.push(format_phase_line(phase));
        }

        // In-progress phases.
        for phase in &in_progress_phases {
            lines.push(format_phase_line(phase));
        }
    }

    // Footer: elapsed + tokens + thinking
    if phases.iter().any(|p| !p.all_finished) || progress.input_tokens > 0 {
        let mut parts = vec![format_duration_compact(turn_elapsed)];

        if progress.input_tokens > 0 || progress.output_tokens > 0 {
            let in_str = format_token_count(progress.input_tokens);
            let out_str = format_token_count(progress.output_tokens);
            parts.push(format!("↑{in_str} ↓{out_str}"));
        }

        if progress.thinking_ms > 0 {
            let secs = progress.thinking_ms / 1000;
            if secs > 0 {
                parts.push(format!("thought {secs}s"));
            }
        }

        lines.push(format!("✳ {}", parts.join(" · ")));
    }

    lines.join("\n")
}

fn format_token_count(tokens: u32) -> String {
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

/// Minimal HTML escaping for text embedded in trace display.
fn trace_html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

use crate::tool_display::{tool_activity_label, tool_display_info};

/// Per-chat streaming state for progressive `editMessageText` updates.
struct StreamingMessage {
    /// All message IDs sent for this stream (multiple when splitting long
    /// content).
    message_ids: Vec<MessageId>,
    /// Accumulated raw text for the current (latest) message.
    accumulated: String,
    /// Number of raw characters already finalized into earlier split messages.
    streamed_prefix_chars: usize,
    /// Last successful `editMessageText` timestamp for throttling.
    last_edit: Instant,
    /// Whether new text has been appended since the last edit.
    dirty: bool,
}

impl StreamingMessage {
    fn new() -> Self {
        Self {
            message_ids: Vec::new(),
            accumulated: String::new(),
            streamed_prefix_chars: 0,
            last_edit: Instant::now(),
            dirty: false,
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
    pub primary_chat_id: Option<i64>,
    /// Allowed group chat ID. Only this group is authorized for bot
    /// interaction.
    pub allowed_group_chat_id: Option<i64>,
    /// How the bot handles group chat messages.
    pub group_policy: GroupPolicy,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            primary_chat_id: None,
            allowed_group_chat_id: None,
            group_policy: GroupPolicy::MentionOrSmallGroup,
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
    bot: teloxide::Bot,
    allowed_chat_ids: Vec<i64>,
    polling_timeout: u32,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    /// Bot username from getMe (set during start).
    bot_username: Arc<RwLock<Option<String>>>,
    /// Registered command handlers for slash commands.
    command_handlers: StdRwLock<Vec<Arc<dyn CommandHandler>>>,
    /// Registered callback handlers for interactive elements.
    callback_handlers: Vec<Arc<dyn CallbackHandler>>,
    /// Runtime-updatable configuration (primary chat ID, allowed group chat
    /// ID).
    config: Arc<StdRwLock<TelegramConfig>>,
    /// StreamHub for subscribing to real-time token deltas.
    stream_hub: Arc<RwLock<Option<StreamHubRef>>>,
    /// Per-chat active streaming state, keyed by `chat_id`.
    active_streams: Arc<DashMap<i64, StreamingMessage>>,
    /// Lightweight index for tape-persisted execution traces, keyed by
    /// "{chat_id}:{msg_id}" → (tape_name, trace_id).
    trace_index: TraceIndex,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter.
    ///
    /// # Arguments
    ///
    /// - `bot` — a configured [`teloxide::Bot`] instance
    /// - `allowed_chat_ids` — list of Telegram chat IDs that are permitted to
    ///   interact with the adapter. Pass an empty vec to allow all chats.
    pub fn new(bot: teloxide::Bot, allowed_chat_ids: Vec<i64>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            bot,
            allowed_chat_ids,
            polling_timeout: POLL_TIMEOUT_SECS,
            shutdown_tx,
            shutdown_rx,
            bot_username: Arc::new(RwLock::new(None)),
            command_handlers: StdRwLock::new(Vec::new()),
            callback_handlers: Vec::new(),
            config: Arc::new(StdRwLock::new(TelegramConfig::default())),
            stream_hub: Arc::new(RwLock::new(None)),
            active_streams: Arc::new(DashMap::new()),
            trace_index: Arc::new(TraceStore::load()),
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
    ) -> Result<Self, anyhow::Error> {
        let bot = build_bot(token, proxy)?;
        Ok(Self::new(bot, allowed_chat_ids))
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

    /// Register callback handlers.
    #[must_use]
    pub fn with_callback_handlers(mut self, handlers: Vec<Arc<dyn CallbackHandler>>) -> Self {
        self.callback_handlers = handlers;
        self
    }

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

    /// Return a shared handle to the runtime config.
    ///
    /// Callers can use this to update configuration at runtime (e.g. change the
    /// primary chat ID) without restarting the adapter. The polling loop reads
    /// the config on every update, so changes take effect immediately.
    pub fn config_handle(&self) -> Arc<StdRwLock<TelegramConfig>> {
        Arc::clone(&self.config)
    }

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
    async fn send_attachments(&self, chat_id: i64, attachments: &[rara_kernel::io::Attachment]) {
        use teloxide::types::InputFile;

        for attachment in attachments {
            let input_file = InputFile::memory(attachment.data.clone());
            let input_file = if let Some(ref name) = attachment.filename {
                input_file.file_name(name.clone())
            } else {
                input_file
            };

            if attachment.mime_type.starts_with("image/") {
                let _ = self
                    .bot
                    .send_photo(ChatId(chat_id), input_file)
                    .await
                    .map_err(|e| warn!("failed to send photo: {e}"));
            } else {
                let _ = self
                    .bot
                    .send_document(ChatId(chat_id), input_file)
                    .await
                    .map_err(|e| warn!("failed to send document: {e}"));
            }
        }
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn send(&self, endpoint: &Endpoint, msg: PlatformOutbound) -> Result<(), EgressError> {
        let (chat_id, _thread_id) = match &endpoint.address {
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
                let content = if let Some(state) = self.active_streams.get(&chat_id) {
                    slice_after_char_prefix(&content, state.streamed_prefix_chars)
                } else {
                    content
                };
                if content.is_empty() && attachments.is_empty() {
                    self.active_streams.remove(&chat_id);
                    return Ok(());
                }
                let html = crate::telegram::markdown::markdown_to_telegram_html(&content);
                let chunks = crate::telegram::markdown::chunk_message(&html, 4096);

                if self.active_streams.contains_key(&chat_id) {
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
                        if let Some(&last_msg_id) = stream_state.message_ids.last() {
                            if last_msg_id != MessageId(0) {
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
                                        let _ = self
                                            .bot
                                            .send_message(ChatId(chat_id), chunk)
                                            .parse_mode(ParseMode::Html)
                                            .await;
                                    }
                                    self.send_attachments(chat_id, &attachments).await;
                                    return Ok(());
                                }
                                warn!(
                                    chat_id,
                                    "telegram: edit streaming message failed, falling back to send"
                                );
                            }
                        }
                    }
                }

                if !content.is_empty() {
                    for (i, chunk) in chunks.iter().enumerate() {
                        let mut req = self
                            .bot
                            .send_message(ChatId(chat_id), chunk)
                            .parse_mode(ParseMode::Html);

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
                self.send_attachments(chat_id, &attachments).await;
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
                    let _ = self.bot.send_message(ChatId(chat_id), &delta).await;
                }
            }
            PlatformOutbound::Progress { .. } => {
                let _ = self
                    .bot
                    .send_chat_action(ChatId(chat_id), ChatAction::Typing)
                    .await;
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
        let trace_index = Arc::clone(&self.trace_index);
        let tape = handle.tape().clone();
        let command_handlers: Arc<[Arc<dyn CommandHandler>]> = self
            .command_handlers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .into();

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

        // Spawn approval request listener — sends inline keyboard to primary chat.
        {
            let approval_rx = handle.security().approval().subscribe_requests();
            let approval_bot = self.bot.clone();
            let approval_config = Arc::clone(&self.config);
            let mut approval_shutdown = self.shutdown_rx.clone();
            tokio::spawn(async move {
                approval_listener(
                    approval_bot,
                    approval_rx,
                    approval_config,
                    &mut approval_shutdown,
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
                trace_index,
                tape,
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
    trace_index: TraceIndex,
    tape: rara_kernel::memory::TapeService,
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
                    let trace_index = Arc::clone(&trace_index);
                    let tape = tape.clone();
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
                            &trace_index,
                            &tape,
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

    // Edit the original message to show the decision (remove buttons).
    if let Some(msg) = &callback.message {
        let (msg_id, chat_id, original_text) = match msg {
            teloxide::types::MaybeInaccessibleMessage::Regular(m) => {
                let text = m.text().unwrap_or("Guard decision").to_owned();
                (m.id, m.chat.id, text)
            }
            teloxide::types::MaybeInaccessibleMessage::Inaccessible(m) => {
                (m.message_id, m.chat.id, "Guard decision".to_owned())
            }
        };

        let status = match (&decision, &result) {
            (ApprovalDecision::Approved, Ok(_)) => format!("✅ <b>Approved</b> by @{decided_by}"),
            (ApprovalDecision::Denied, Ok(_)) => format!("❌ <b>Denied</b> by @{decided_by}"),
            (_, Err(e)) => format!("⚠️ Failed: {}", guard_html_escape(e)),
            _ => "Done".to_string(),
        };

        // Preserve original message content and append the decision status.
        let new_text = format!("{}\n\n{}", guard_html_escape(&original_text), status);
        let _ = bot
            .edit_message_text(chat_id, msg_id, new_text)
            .parse_mode(ParseMode::Html)
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

/// Handle a trace show/hide callback query from an inline keyboard button.
///
/// Callback data format: `"trace:{action}:{chat_id}:{msg_id}"`
/// - `action` = "show" → expand to full trace, button becomes "收起"
/// - `action` = "hide" → collapse back to compact summary, button becomes
///   "详情"
///
/// Pre-rendered HTML is read from [`TraceIndex`] cache (O(1), zero I/O).
/// On cache miss (eviction or process restart), falls back to reading from
/// tape — slower but still functional. The callback is always answered
/// immediately to eliminate the Telegram spinner.
async fn handle_trace_callback(
    bot: &teloxide::Bot,
    callback: &teloxide::types::CallbackQuery,
    data: &str,
    trace_index: &TraceIndex,
    tape: &rara_kernel::memory::TapeService,
) {
    // Parse: "trace:show:{chat_id}:{msg_id}" or "trace:hide:{chat_id}:{msg_id}"
    let parts: Vec<&str> = data.splitn(3, ':').collect();
    if parts.len() != 3 {
        return;
    }
    let action = parts[1];
    let trace_key = parts[2];

    // Answer callback immediately — removes Telegram spinner.
    let _ = bot.answer_callback_query(callback.id.clone()).await;

    // Hot path: read pre-rendered HTML from cache (O(1), zero I/O)
    let entry = if let Some(cached) = trace_index.html_cache.get(trace_key).map(|r| r.value().clone()) {
        cached
    } else {
        // Cold path: cache miss — fall back to tape read + render via coord_index
        let coords = trace_index.coord_index.get(trace_key).map(|r| r.value().clone());
        if let Some((tape_name, trace_id)) = coords {
            if let Some(trace) = read_trace_from_tape(tape, &tape_name, &trace_id).await {
                let rebuilt = TraceCacheEntry {
                    compact_html: render_compact_summary(&trace),
                    detail_html: render_trace_detail(&trace),
                };
                // Re-populate cache for subsequent clicks
                trace_index.html_cache.insert(trace_key.to_owned(), rebuilt.clone());
                rebuilt
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let (text, button_text, next_action) = match action {
        "show" => (
            &entry.detail_html,
            "\u{1f4ca} \u{6536}\u{8d77}",
            format!("trace:hide:{trace_key}"),
        ),
        _ => (
            &entry.compact_html,
            "\u{1f4ca} \u{8be6}\u{60c5}",
            format!("trace:show:{trace_key}"),
        ),
    };

    // Parse chat_id and msg_id from trace_key
    if let Some((chat_id_str, msg_id_str)) = trace_key.split_once(':') {
        if let (Ok(cid), Ok(mid)) =
            (chat_id_str.parse::<i64>(), msg_id_str.parse::<i32>())
        {
            let keyboard =
                InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                    button_text,
                    next_action,
                )]]);
            let _ = bot
                .edit_message_text(ChatId(cid), MessageId(mid), text)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await;
        }
    }
}

/// Read an execution trace from tape by scanning for a matching trace_id.
async fn read_trace_from_tape(
    tape: &rara_kernel::memory::TapeService,
    tape_name: &str,
    trace_id: &str,
) -> Option<ExecutionTrace> {
    let entries = tape.entries(tape_name).await.ok()?;

    // Scan backwards (most recent first) for the matching trace event.
    for entry in entries.iter().rev() {
        if entry.kind == rara_kernel::memory::TapEntryKind::Event {
            if let Some(name) = entry.payload.get("name").and_then(|v| v.as_str()) {
                if name == "turn.execution_trace" {
                    if let Some(data) = entry.payload.get("data") {
                        if let Some(id) = data.get("trace_id").and_then(|v| v.as_str()) {
                            if id == trace_id {
                                let trace_data = data.get("data")?;
                                return serde_json::from_value(trace_data.clone()).ok();
                            }
                        }
                    }
                }
            }
        }
    }

    None
}


/// Listens for new approval requests and sends inline keyboard messages
/// to the primary Telegram chat so the user can approve or deny.
async fn approval_listener(
    bot: teloxide::Bot,
    mut rx: tokio::sync::broadcast::Receiver<ApprovalRequest>,
    config: Arc<StdRwLock<TelegramConfig>>,
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

                let chat_id = {
                    let cfg = config.read().unwrap_or_else(|e| e.into_inner());
                    cfg.primary_chat_id
                };
                let Some(chat_id) = chat_id else {
                    warn!("telegram approval listener: no primary_chat_id configured, cannot send approval prompt");
                    continue;
                };

                let (_display, args_summary) = tool_display_info(&req.tool_name, &req.tool_args);
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
                text.push_str(&format!(
                    "<b>Reason:</b> {summary}\n\
                     <b>Risk:</b> {risk:?}\n\n\
                     Approve or deny this action:",
                    summary = guard_html_escape(&req.summary),
                    risk = req.risk_level,
                ));

                let keyboard = InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback("✅ Approve", format!("guard:approve:{}", req.id)),
                    InlineKeyboardButton::callback("❌ Deny", format!("guard:deny:{}", req.id)),
                ]]);

                let result = bot
                    .send_message(ChatId(chat_id), &text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await;

                if let Err(e) = result {
                    warn!(error = %e, "telegram approval listener: failed to send approval prompt");
                }
            }
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
    trace_index: &TraceIndex,
    tape: &rara_kernel::memory::TapeService,
) {
    // Read a snapshot of the runtime config for this update.
    let cfg = match config.read() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    };

    // Handle callback queries by prefix routing:
    //   "guard:*"  → guard approval (approve/deny)
    //   "trace:*"  → execution trace toggle (show/hide detail view)
    //   other      → TODO: convert to RawPlatformMessage for kernel processing
    if let UpdateKind::CallbackQuery(callback) = &update.kind {
        if let Some(data) = &callback.data {
            if data.starts_with("guard:") {
                handle_guard_callback(handle, bot, callback, data, allowed_chat_ids).await;
                return;
            }
            if data.starts_with("trace:") {
                handle_trace_callback(bot, callback, data, trace_index, tape).await;
                return;
            }
        }
        return;
    }

    let msg = match &update.kind {
        UpdateKind::Message(msg) | UpdateKind::EditedMessage(msg) => msg,
        _ => return,
    };

    let chat_id = msg.chat.id.0;

    // Check if this chat is allowed.
    if !allowed_chat_ids.is_empty() && !allowed_chat_ids.contains(&chat_id) {
        warn!(
            chat_id,
            "telegram adapter: dropping message from unauthorized chat"
        );
        return;
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
                                dispatch_command_result(bot, chat_id, result).await;
                            }
                            Err(e) => {
                                error!(
                                    command = cmd_name,
                                    error = %e,
                                    "telegram adapter: command handler failed"
                                );
                                let _ = bot
                                    .send_message(ChatId(chat_id), format!("Command failed: {e}"))
                                    .await;
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

    let msg = match handle.resolve(raw).await {
        Ok(msg) => msg,
        Err(IOError::SystemBusy) => {
            let _ = bot
                .send_message(
                    ChatId(chat_id),
                    "⚠️ System is busy, please try again later.",
                )
                .await;
            return;
        }
        Err(IOError::RateLimited { message }) => {
            let _ = bot
                .send_message(ChatId(chat_id), format!("\u{26a0}\u{fe0f} {message}"))
                .await;
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

    let session_id = msg.session_key.clone();
    let rara_message_id = msg.id.to_string();

    // Route: group proactive candidates go through GroupMessage event for
    // lightweight LLM judgment; directly-addressed messages go through the
    // normal UserMessage path.
    let submit_result = if is_group_proactive {
        handle.submit_group_message(msg)
    } else {
        handle.submit_message(msg)
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
                    sid,
                    Arc::clone(trace_index),
                    tape.clone(),
                    rara_message_id.clone(),
                );
            }
        }
        Err(_) => {
            let _ = bot
                .send_message(ChatId(chat_id), "⚠️ 系统繁忙，请稍后再试。")
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Command result dispatch
// ---------------------------------------------------------------------------

/// Send a [`CommandResult`] back to the Telegram chat.
async fn dispatch_command_result(bot: &teloxide::Bot, chat_id: i64, result: CommandResult) {
    match result {
        CommandResult::Text(text) => {
            let _ = bot.send_message(ChatId(chat_id), text).await;
        }
        CommandResult::Html(html) => {
            let _ = bot
                .send_message(ChatId(chat_id), html)
                .parse_mode(ParseMode::Html)
                .await;
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
            let _ = bot
                .send_message(ChatId(chat_id), html)
                .parse_mode(ParseMode::Html)
                .reply_markup(markup)
                .await;
        }
        CommandResult::Photo { data, caption } => {
            use teloxide::types::InputFile;

            let mut request = bot.send_photo(ChatId(chat_id), InputFile::memory(data));
            if let Some(caption) = caption {
                request = request.caption(caption);
            }
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

/// Spawn a background task that subscribes to [`StreamHub`] for the given
/// session and progressively updates a Telegram message via `editMessageText`.
fn spawn_stream_forwarder(
    stream_hub: Arc<RwLock<Option<StreamHubRef>>>,
    active_streams: Arc<DashMap<i64, StreamingMessage>>,
    bot: teloxide::Bot,
    chat_id: i64,
    session_id: rara_kernel::session::SessionKey,
    trace_index: TraceIndex,
    tape: rara_kernel::memory::TapeService,
    rara_message_id: String,
) {
    use rara_kernel::io::StreamEvent;

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
            tracing::debug!(session_id = %session_id, "telegram stream forwarder: no streams found");
            return;
        }

        // Initialize streaming state.
        active_streams.insert(chat_id, StreamingMessage::new());

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
        let mut plan: Option<PlanDisplay> = None;

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
                                let result = flush_edit(&bot, chat_id, &req).await;
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
                        Ok(StreamEvent::ToolCallStart { name, id, arguments }) => {
                            let (display, summary) = tool_display_info(&name, &arguments);
                            let activity = tool_activity_label(&name).to_owned();
                            progress.tools.push(ToolProgress {
                                id,
                                name: display,
                                activity,
                                summary,
                                started_at: Instant::now(),
                                finished: false,
                                success: false,
                                duration: None,
                                error: None,
                            });

                            // Send typing indicator before the first progress message.
                            if progress.message_id.is_none() {
                                let _ = bot
                                    .send_chat_action(ChatId(chat_id), ChatAction::Typing)
                                    .await;
                            }

                            let text = render_progress(&progress.tools, progress.turn_started.elapsed(), &progress);
                            if progress.last_edit.elapsed() >= MIN_EDIT_INTERVAL {
                                match progress.message_id {
                                    Some(mid) => {
                                        let _ = bot
                                            .edit_message_text(ChatId(chat_id), mid, &text)
                                            .await;
                                    }
                                    None => {
                                        if let Ok(msg) = bot
                                            .send_message(ChatId(chat_id), &text)
                                            .await
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
                        Ok(StreamEvent::ToolCallEnd { id, success, error, .. }) => {
                            if let Some(tp) = progress.tools.iter_mut().find(|t| t.id == id) {
                                tp.finished = true;
                                tp.success = success;
                                tp.duration = Some(tp.started_at.elapsed());
                                tp.error = error;
                            }

                            let text = render_progress(&progress.tools, progress.turn_started.elapsed(), &progress);
                            if progress.last_edit.elapsed() >= MIN_EDIT_INTERVAL {
                                match progress.message_id {
                                    Some(mid) => {
                                        let _ = bot
                                            .edit_message_text(ChatId(chat_id), mid, &text)
                                            .await;
                                    }
                                    None => {
                                        if let Ok(msg) = bot
                                            .send_message(ChatId(chat_id), &text)
                                            .await
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
                        Ok(StreamEvent::PlanCreated { total_steps, compact_summary, estimated_duration_secs, .. }) => {
                            let mut p = PlanDisplay::new(total_steps, estimated_duration_secs, compact_summary);
                            // Micro tier: don't send any plan message.
                            if p.tier != PlanTier::Micro {
                                let text = p.render();
                                if !text.is_empty() {
                                    match bot.send_message(ChatId(chat_id), &text).await {
                                        Ok(msg) => { p.message_id = Some(msg.id); }
                                        Err(e) => { warn!(chat_id, error = %e, "failed to send plan message"); }
                                    }
                                }
                                p.last_edit = Instant::now();
                            }
                            plan = Some(p);
                        }
                        Ok(StreamEvent::PlanProgress { status_text, .. }) => {
                            if let Some(ref mut p) = plan {
                                if p.tier == PlanTier::Micro {
                                    continue; // micro: suppress all plan updates
                                }
                                // Dedup: skip if identical to last status.
                                if status_text == p.last_status {
                                    continue;
                                }
                                p.last_status = status_text.clone();
                                p.status_lines.push(status_text);
                                let text = p.render();
                                if p.last_edit.elapsed() >= MIN_EDIT_INTERVAL {
                                    if let Some(mid) = p.message_id {
                                        let _ = bot.edit_message_text(ChatId(chat_id), mid, &text).await;
                                    }
                                    p.last_edit = Instant::now();
                                }
                            }
                        }
                        Ok(StreamEvent::PlanReplan { reason }) => {
                            if let Some(ref mut p) = plan {
                                if p.tier == PlanTier::Micro {
                                    continue;
                                }
                                let status = format!("方案调整中…{reason}");
                                p.status_lines.push(status.clone());
                                p.last_status = status;
                                let text = p.render();
                                if let Some(mid) = p.message_id {
                                    let _ = bot.edit_message_text(ChatId(chat_id), mid, &text).await;
                                }
                                p.last_edit = Instant::now();
                            }
                        }
                        Ok(StreamEvent::PlanCompleted { summary }) => {
                            if let Some(ref mut p) = plan {
                                if p.tier != PlanTier::Micro {
                                    let done_text = format!("\u{2705} {summary}");
                                    p.status_lines.push(done_text);
                                    let text = p.render();
                                    if let Some(mid) = p.message_id {
                                        let _ = bot.edit_message_text(ChatId(chat_id), mid, &text).await;
                                    }
                                }
                                // IMPORTANT: save plan steps BEFORE `plan = None` below.
                                // PlanCompleted clears the plan, but we need the steps
                                // for the post-completion trace detail view.
                                progress.saved_plan_steps = p.status_lines.clone();
                            }
                            plan = None;
                        }
                        Ok(StreamEvent::UsageUpdate { input_tokens, output_tokens, thinking_ms }) => {
                            progress.input_tokens = input_tokens;
                            progress.output_tokens = output_tokens;
                            progress.thinking_ms = thinking_ms;
                            // Trigger a progress re-render if we have a message
                            if progress.message_id.is_some() || !progress.tools.is_empty() {
                                let text = render_progress(&progress.tools, progress.turn_started.elapsed(), &progress);
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
                        }
                        Ok(StreamEvent::TurnMetrics { model, iterations, .. }) => {
                            // TurnMetrics arrives just before stream close —
                            // stash for the ExecutionTrace built in RecvError::Closed.
                            progress.model = model;
                            progress.iterations = iterations;
                        }
                        Ok(_) => {} // Ignore: Progress (stage changes have no TG UX)
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!(chat_id, skipped = n, "telegram stream forwarder lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            // Stream closed — do final flush.
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
                                let result = flush_edit(&bot, chat_id, &req).await;
                                apply_flush_result(&active_streams, chat_id, result);
                            }

                            // ── Finalize: convert progress → compact summary + trace button ──
                            // The progress message (which showed live tool activity) is
                            // replaced with a one-line summary. An inline keyboard button
                            // lets the user toggle the full execution trace on demand.
                            // The trace is stored in TraceStore with 1-hour TTL.
                            if let Some(mid) = progress.message_id {
                                // Plan steps come from either:
                                // 1. saved_plan_steps (if PlanCompleted fired), or
                                // 2. the still-active plan (if stream closed mid-plan).
                                let plan_steps = if progress.saved_plan_steps.is_empty() {
                                    plan.as_ref().map(|p| p.status_lines.clone()).unwrap_or_default()
                                } else {
                                    std::mem::take(&mut progress.saved_plan_steps)
                                };

                                let trace = ExecutionTrace {
                                    duration_secs:    progress.turn_started.elapsed().as_secs(),
                                    iterations:       progress.iterations,
                                    model:            std::mem::take(&mut progress.model),
                                    input_tokens:     progress.input_tokens,
                                    output_tokens:    progress.output_tokens,
                                    thinking_ms:      progress.thinking_ms,
                                    thinking_preview: std::mem::take(&mut progress.reasoning_preview),
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

                                let compact_html = render_compact_summary(&trace);
                                let detail_html = render_trace_detail(&trace);
                                let trace_key = format!("{}:{}", chat_id, mid.0);
                                let keyboard = InlineKeyboardMarkup::new(vec![vec![
                                    InlineKeyboardButton::callback(
                                        "\u{1f4ca} \u{8be6}\u{60c5}",
                                        format!("trace:show:{trace_key}"),
                                    ),
                                ]]);

                                let _ = bot
                                    .edit_message_text(ChatId(chat_id), mid, &compact_html)
                                    .parse_mode(ParseMode::Html)
                                    .reply_markup(keyboard)
                                    .await;

                                // Persist trace to tape
                                let trace_id = ulid::Ulid::new().to_string();
                                let tape_name = session_id.to_string();
                                let trace_json = serde_json::to_value(&trace).unwrap_or_default();

                                if let Err(e) = tape.append_event(
                                    &tape_name,
                                    "turn.execution_trace",
                                    serde_json::json!({
                                        "trace_id": trace_id,
                                        "data": trace_json,
                                    }),
                                ).await {
                                    warn!(error = %e, "failed to persist execution trace to tape");
                                }

                                // Store pre-rendered HTML for instant callback response.
                                // Evict oldest entries when cache exceeds capacity.
                                // Evict oldest HTML entries when cache exceeds capacity.
                                // coord_index is never evicted (~100 bytes/entry).
                                if trace_index.html_cache.len() >= TRACE_CACHE_CAPACITY {
                                    let keys: Vec<String> = trace_index.html_cache
                                        .iter()
                                        .take(TRACE_CACHE_CAPACITY / 2)
                                        .map(|r| r.key().clone())
                                        .collect();
                                    for k in keys {
                                        trace_index.html_cache.remove(&k);
                                    }
                                }
                                trace_index.html_cache.insert(trace_key.clone(), TraceCacheEntry {
                                    compact_html,
                                    detail_html,
                                });
                                trace_index.coord_index.insert(trace_key, (tape_name, trace_id));
                                trace_index.persist();
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
                        let result = flush_edit(&bot, chat_id, &req).await;
                        apply_flush_result(&active_streams, chat_id, result);
                    }

                    // Flush throttled progress updates.
                    // Also refresh when tools are still running so the elapsed
                    // timer keeps ticking even without new stream events.
                    let has_running = progress.tools.iter().any(|t| !t.finished);
                    if (progress_dirty || has_running) && !progress.tools.is_empty() {
                        let text = render_progress(&progress.tools, progress.turn_started.elapsed(), &progress);
                        match progress.message_id {
                            Some(mid) => {
                                let _ = bot
                                    .edit_message_text(ChatId(chat_id), mid, &text)
                                    .await;
                            }
                            None => {
                                if let Ok(msg) = bot
                                    .send_message(ChatId(chat_id), &text)
                                    .await
                                {
                                    progress.message_id = Some(msg.id);
                                }
                            }
                        }
                        progress.last_edit = Instant::now();
                        progress_dirty = false;
                    }
                }
                _ = typing_interval.tick() => {
                    let _ = bot
                        .send_chat_action(ChatId(chat_id), ChatAction::Typing)
                        .await;
                }
            }
        }

        // Auto-cleanup after 120s if Reply never arrives.
        let streams = active_streams.clone();
        let cid = chat_id;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(120)).await;
            if streams.remove(&cid).is_some() {
                warn!(
                    chat_id = cid,
                    "telegram stream forwarder: stale state cleaned up after 120s"
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

/// Data extracted from [`StreamingMessage`] needed for a flush operation.
/// Allows dropping the DashMap guard before making async Telegram API calls.
struct FlushRequest {
    message_ids: Vec<MessageId>,
    text_html: String,
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

/// Flush accumulated text to Telegram via `sendMessage` (first time) or
/// `editMessageText` (subsequent).
///
/// This function does NOT hold any DashMap guard — the caller must extract
/// the data into a [`FlushRequest`] and drop the guard before calling.
async fn flush_edit(bot: &teloxide::Bot, chat_id: i64, req: &FlushRequest) -> FlushResult {
    if req.message_ids.is_empty() || req.message_ids.last().copied() == Some(MessageId(0)) {
        // First message or new split — send a new message.
        match bot
            .send_message(ChatId(chat_id), &req.text_html)
            .parse_mode(ParseMode::Html)
            .await
        {
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
    if raw_text.is_none() && msg.photo().is_none() {
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

    if text.trim().is_empty() && msg.photo().is_none() {
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

pub fn format_session_key(chat_id: i64) -> String {
    format!("tg:{chat_id}")
}

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
