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

pub mod fold;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, info_span, warn};

use crate::{
    error::{IoSnafu, KernelError, Result},
    guard::pipeline::{GuardPipeline, GuardVerdict},
    handle::KernelHandle,
    identity::Role,
    io::{StreamEvent, StreamHandle},
    llm,
    llm::ModelCapabilities,
    notification::{KernelNotification, NotificationBusRef},
    session::SessionKey,
};

/// Estimated chars-per-token ratio for context size estimation.
const CHARS_PER_TOKEN: usize = 4;
/// Context usage threshold (fraction) at which a SHOULD-handoff hint is
/// injected.
const CONTEXT_WARN_THRESHOLD: f64 = 0.70;
/// Context usage threshold (fraction) at which a MUST-handoff hint is injected.
const CONTEXT_CRITICAL_THRESHOLD: f64 = 0.85;
/// Large tool outputs that should trigger an explicit anchor reminder.
const LARGE_TOOL_RESULT_CHARS: usize = 8_000;
/// Multiple medium tool outputs in one phase should also trigger a reminder.
const MEDIUM_TOOL_RESULT_CHARS: usize = 3_000;
#[derive(Debug, Clone, Copy, PartialEq)]
enum ContextPressure {
    Normal,
    Warning {
        estimated_tokens: usize,
        usage_ratio:      f64,
    },
    Critical {
        estimated_tokens: usize,
        usage_ratio:      f64,
    },
}

/// Classification of an agent's functional role.
///
/// Roles enable callers to look up agents by function rather than by name.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, strum::Display, Default,
)]
#[strum(serialize_all = "snake_case")]
pub enum AgentRole {
    /// User-facing conversational agent (default chat entry point).
    #[default]
    Chat,
    /// Codebase recon / investigation agent.
    Scout,
    /// Task planning agent.
    Planner,
    /// Execution / coding agent.
    Worker,
}

/// Dispatch priority for agent messages.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    strum::Display,
)]
#[strum(serialize_all = "snake_case")]
pub enum Priority {
    /// Background tasks, batch jobs.
    Low = 0,
    /// Default priority for interactive messages.
    #[default]
    Normal = 1,
    /// Elevated priority (e.g., admin requests).
    High = 2,
    /// System-critical messages (bypass rate limiting).
    Critical = 3,
}

/// Configuration for agent file-system sandboxing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxConfig {
    /// Allowed file paths (read/write). Path-prefix matching.
    #[serde(default)]
    pub allowed_paths:      Vec<String>,
    /// Read-only paths (reads allowed, writes denied). Path-prefix matching.
    #[serde(default)]
    pub read_only_paths:    Vec<String>,
    /// Denied paths (takes precedence over allowed and read-only).
    #[serde(default)]
    pub denied_paths:       Vec<String>,
    /// Whether to create an isolated temp workspace for this agent.
    #[serde(default)]
    pub isolated_workspace: bool,
}

/// Execution mode for message processing routing.
///
/// Controls whether a session uses the standard reactive agent loop (v1)
/// or the plan-execute architecture (v2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Standard reactive agent loop (v1). The agent processes each message
    /// through the normal LLM → tool → LLM cycle.
    Reactive,
    /// Plan-execute mode (v2). The agent first generates a plan, then
    /// executes each step with verification.
    #[default]
    Plan,
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reactive => write!(f, "reactive"),
            Self::Plan => write!(f, "plan"),
        }
    }
}

impl ExecutionMode {
    /// Return the version number (1 for reactive, 2 for plan).
    pub fn version(&self) -> u8 {
        match self {
            Self::Reactive => 1,
            Self::Plan => 2,
        }
    }

    /// Parse from a version number string ("1" or "2").
    pub fn from_version_str(s: &str) -> Option<Self> {
        match s.trim() {
            "1" => Some(Self::Reactive),
            "2" => Some(Self::Plan),
            _ => None,
        }
    }
}

/// Agent "binary" — static definition, loadable from YAML or constructed
/// dynamically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Unique name identifying this agent definition.
    pub name:                   String,
    /// Agent's functional role (chat, scout, planner, worker).
    #[serde(default)]
    pub role:                   AgentRole,
    /// Human-readable description.
    pub description:            String,
    /// LLM model identifier.
    #[serde(default)]
    pub model:                  Option<String>,
    /// System prompt defining agent behavior.
    pub system_prompt:          String,
    /// Optional personality/mood/voice prompt.
    #[serde(default)]
    pub soul_prompt:            Option<String>,
    /// Optional hint for provider selection.
    #[serde(default)]
    pub provider_hint:          Option<String>,
    /// Maximum LLM iterations before forced completion.
    #[serde(default)]
    pub max_iterations:         Option<usize>,
    /// Tool names this agent is allowed to use (empty = inherit parent's
    /// tools).
    #[serde(default)]
    pub tools:                  Vec<String>,
    /// Maximum number of concurrent child agents this agent can spawn.
    #[serde(default)]
    pub max_children:           Option<usize>,
    /// Maximum context window size in tokens.
    #[serde(default)]
    pub max_context_tokens:     Option<usize>,
    /// Dispatch priority for scheduling.
    #[serde(default)]
    pub priority:               Priority,
    /// Arbitrary metadata for extension.
    #[serde(default)]
    pub metadata:               serde_json::Value,
    /// Optional sandbox configuration for file access control.
    #[serde(default)]
    pub sandbox:                Option<SandboxConfig>,
    /// Default execution mode for this agent ("reactive" or "plan").
    /// When set, sessions using this manifest default to this mode
    /// unless overridden by session-level `/msg_version`.
    #[serde(default)]
    pub default_execution_mode: Option<ExecutionMode>,
    /// Per-turn tool call threshold that triggers a pause requiring user
    /// confirmation. When `tool_calls_made >= pause_turn_threshold`, the
    /// agent loop pauses and waits for user input (continue/stop).
    /// Default: 15.
    #[serde(default)]
    pub pause_turn_threshold:   Option<usize>,
}

/// Process environment — isolated per-agent context.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AgentEnv {
    /// Optional workspace directory for file operations.
    pub workspace: Option<String>,
    /// Key-value environment variables.
    pub vars:      HashMap<String, String>,
}

/// Shared reference to the [`AgentRegistry`].
pub type AgentRegistryRef = Arc<AgentRegistry>;

pub struct AgentRegistry {
    builtin:       HashMap<String, AgentManifest>,
    custom:        DashMap<String, AgentManifest>,
    agents_dir:    PathBuf,
    /// Role → agent name mapping for default agent resolution.
    role_defaults: DashMap<Role, String>,
}

impl AgentRegistry {
    /// Build a registry from builtin agents, user-defined manifests, and an
    /// agents directory for persistence.
    ///
    /// Every agent must declare a [`Role`] — this determines which user role
    /// routes to it by default. The **first** agent registered for a given
    /// role wins the default slot; subsequent agents with the same role are
    /// still accessible by name but don't override the default.
    pub fn init(
        builtin: Vec<(AgentManifest, Role)>,
        loader: &ManifestLoader,
        agents_dir: PathBuf,
    ) -> Self {
        let role_defaults = DashMap::new();
        let builtin = builtin
            .into_iter()
            .map(|(m, role)| {
                // First agent registered for a role becomes the default.
                role_defaults.entry(role).or_insert_with(|| m.name.clone());
                (m.name.clone(), m)
            })
            .collect();
        let registry = Self {
            builtin,
            custom: DashMap::new(),
            agents_dir,
            role_defaults,
        };
        for manifest in loader.list() {
            let name = manifest.name.clone();
            if !registry.builtin.contains_key(&name) {
                registry.custom.insert(name, manifest.clone());
            }
        }
        registry
    }

    #[tracing::instrument(skip(self))]
    pub fn get(&self, name: &str) -> Option<AgentManifest> {
        // Custom first (shadow), then builtin.
        if let Some(m) = self.custom.get(name) {
            return Some(m.value().clone());
        }
        self.builtin.get(name).cloned()
    }

    pub fn list(&self) -> Vec<AgentManifest> {
        let mut result: HashMap<String, AgentManifest> = self.builtin.clone();
        for entry in &self.custom {
            result.insert(entry.key().clone(), entry.value().clone());
        }
        result.into_values().collect()
    }

    #[tracing::instrument(skip(self, manifest), fields(agent_name = %manifest.name))]
    pub fn register(&self, manifest: AgentManifest, role: Role) -> Result<()> {
        let name = manifest.name.clone();
        // Persist to YAML.
        let path = self.agents_dir.join(format!("{}.yaml", name));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let yaml = serde_yaml::to_string(&manifest)
            .whatever_context::<_, KernelError>("failed to serialize manifest")?;
        std::fs::write(&path, yaml).context(IoSnafu)?;
        self.role_defaults
            .entry(role)
            .or_insert_with(|| name.clone());
        self.custom.insert(name, manifest);
        Ok(())
    }

    pub fn unregister(&self, name: &str) -> Result<()> {
        if self.builtin.contains_key(name) {
            return Err(KernelError::Other {
                message: format!("cannot unregister builtin agent: {name}").into(),
            });
        }
        self.custom.remove(name);
        let path = self.agents_dir.join(format!("{}.yaml", name));
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        Ok(())
    }

    /// Find the default agent for a given user role.
    pub fn agent_for_role(&self, role: Role) -> Option<AgentManifest> {
        let name = self.role_defaults.get(&role)?;
        self.get(name.value())
    }

    pub fn is_builtin(&self, name: &str) -> bool { self.builtin.contains_key(name) }

    pub fn agents_dir(&self) -> &Path { &self.agents_dir }
}

/// Loads [`AgentManifest`] definitions.
pub struct ManifestLoader {
    manifests: Vec<AgentManifest>,
}

impl ManifestLoader {
    /// Create an empty loader.
    pub fn new() -> Self {
        Self {
            manifests: Vec::new(),
        }
    }

    /// Load user-defined manifests from a directory.
    ///
    /// Returns the number of manifests successfully loaded.
    pub fn load_dir(&mut self, dir: &Path) -> Result<usize> {
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut count = 0;
        let entries = std::fs::read_dir(dir).context(IoSnafu)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
            {
                let content = std::fs::read_to_string(&path).context(IoSnafu)?;
                match serde_yaml::from_str::<AgentManifest>(&content) {
                    Ok(m) => {
                        self.manifests.retain(|existing| existing.name != m.name);
                        self.manifests.push(m);
                        count += 1;
                    }
                    Err(e) => {
                        warn!(
                            path = %path.display(),
                            error = %e,
                            "skipping invalid agent manifest"
                        );
                    }
                }
            }
        }
        Ok(count)
    }

    /// Load manifests from code-defined sources.
    pub fn load_manifests(&mut self, manifests: impl IntoIterator<Item = AgentManifest>) {
        for manifest in manifests {
            self.manifests.retain(|m| m.name != manifest.name);
            self.manifests.push(manifest);
        }
    }

    /// Get a manifest by name.
    pub fn get(&self, name: &str) -> Option<&AgentManifest> {
        self.manifests.iter().find(|m| m.name == name)
    }

    /// List all loaded manifests.
    pub fn list(&self) -> &[AgentManifest] { &self.manifests }
}

impl Default for ManifestLoader {
    fn default() -> Self { Self::new() }
}

/// Maximum byte length for result preview strings.
const RESULT_PREVIEW_MAX_BYTES: usize = 2048;

/// Truncate a string to at most `max_bytes` bytes on a valid char boundary.
fn truncate_preview(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let boundary = s.floor_char_boundary(max_bytes);
    format!("{}... (truncated)", &s[..boundary])
}

/// A tool call being incrementally assembled from streaming deltas.
struct PendingToolCall {
    id:            String,
    name:          String,
    arguments_buf: String,
}

/// Trace of a single tool call within an iteration.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolCallTrace {
    pub name:           String,
    pub id:             String,
    pub duration_ms:    u64,
    pub success:        bool,
    pub arguments:      serde_json::Value,
    pub result_preview: String,
    pub error:          Option<String>,
}

/// Trace of a single LLM iteration within a turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IterationTrace {
    pub index:          usize,
    pub first_token_ms: Option<u64>,
    pub stream_ms:      u64,
    /// First 200 chars of accumulated text.
    pub text_preview:   String,
    /// Full accumulated reasoning text for this iteration.
    pub reasoning_text: Option<String>,
    pub tool_calls:     Vec<ToolCallTrace>,
}

/// Complete trace of a single agent turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnTrace {
    pub duration_ms:      u64,
    pub model:            String,
    /// The user message that triggered this turn.
    pub input_text:       Option<String>,
    pub iterations:       Vec<IterationTrace>,
    pub final_text_len:   usize,
    pub total_tool_calls: usize,
    pub success:          bool,
    pub error:            Option<String>,
    /// Rara internal message ID for end-to-end correlation.
    /// For user-triggered turns this is the `InboundMessage.id`;
    /// for proactive turns a fresh ID is generated at dispatch time.
    pub rara_message_id:  crate::io::MessageId,
}

/// Result of a single agent turn.
#[derive(Debug)]
pub struct AgentTurnResult {
    /// The final text produced by the agent.
    pub text:       String,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Number of tool calls executed.
    pub tool_calls: usize,
    /// Model used for this turn.
    pub model:      String,
    /// Detailed trace of the turn for observability.
    pub trace:      TurnTrace,
}

impl AgentTurnResult {
    /// Create an empty result (no text, no tool calls) used when a proactive
    /// judgment decides Rara should not reply.
    pub fn empty() -> Self {
        Self {
            text:       String::new(),
            iterations: 0,
            tool_calls: 0,
            model:      String::new(),
            trace:      TurnTrace {
                duration_ms:      0,
                model:            String::new(),
                input_text:       None,
                iterations:       Vec::new(),
                final_text_len:   0,
                total_tool_calls: 0,
                success:          true,
                error:            None,
                rara_message_id:  crate::io::MessageId::new(),
            },
        }
    }
}

fn parse_tool_call_arguments(arguments: &str) -> std::result::Result<serde_json::Value, String> {
    let args = serde_json::from_str::<serde_json::Value>(arguments)
        .map_err(|err| format!("invalid tool arguments: {err}"))?;
    if !args.is_object() {
        return Err(format!(
            "invalid tool arguments: expected JSON object, got {args}"
        ));
    }
    Ok(args)
}

fn sanitize_messages_for_llm(messages: &[llm::Message]) -> Vec<llm::Message> {
    messages
        .iter()
        .cloned()
        .map(|mut message| {
            if !message.tool_calls.is_empty() {
                message
                    .tool_calls
                    .retain(|call| parse_tool_call_arguments(&call.arguments).is_ok());
            }
            message
        })
        .collect()
}

fn classify_context_pressure(
    estimated_tokens: usize,
    context_window_tokens: usize,
) -> ContextPressure {
    if context_window_tokens == 0 {
        return ContextPressure::Normal;
    }

    let usage_ratio = estimated_tokens as f64 / context_window_tokens as f64;

    if usage_ratio >= CONTEXT_CRITICAL_THRESHOLD {
        ContextPressure::Critical {
            estimated_tokens,
            usage_ratio,
        }
    } else if usage_ratio >= CONTEXT_WARN_THRESHOLD {
        ContextPressure::Warning {
            estimated_tokens,
            usage_ratio,
        }
    } else {
        ContextPressure::Normal
    }
}

fn should_remind_tape_search(input_text: &str) -> bool {
    let normalized = input_text.to_lowercase();
    let exact_fact_cues = [
        "exact",
        "credential",
        "secret",
        "token",
        "code",
        "id",
        "password",
        "quote",
    ];
    let history_cues = [
        "beginning of this conversation",
        "from the beginning",
        "earlier",
        "previous",
        "before",
        "first",
        "earlier in this conversation",
        "from earlier",
    ];

    exact_fact_cues.iter().any(|cue| normalized.contains(cue))
        && history_cues.iter().any(|cue| normalized.contains(cue))
}

fn should_remind_tape_anchor(tool_names: &[String], tool_results: &[serde_json::Value]) -> bool {
    let mut medium_results = 0usize;

    for (name, result) in tool_names.iter().zip(tool_results.iter()) {
        let serialized_len = result.to_string().len();
        let is_large_result = serialized_len >= LARGE_TOOL_RESULT_CHARS;
        let is_medium_result = serialized_len >= MEDIUM_TOOL_RESULT_CHARS;
        let is_high_context_tool = matches!(
            name.as_str(),
            "read-file" | "grep" | "bash" | "http-fetch" | "list-directory" | "find-files"
        );

        if is_large_result && is_high_context_tool {
            return true;
        }

        if is_medium_result && is_high_context_tool {
            medium_results += 1;
        }
    }

    medium_results >= 2
}

/// Resolve the soul prompt for an agent at runtime.
///
/// Loads the soul file and runtime state via `rara_soul::load_and_render`,
/// which renders the soul template with current mood, relationship stage,
/// emerged traits, and style drift.
///
/// Returns `None` for agents with no soul (e.g. worker, mita).
fn resolve_soul_prompt(agent_name: &str) -> Option<String> {
    match rara_soul::load_and_render(agent_name) {
        Ok(Some(rendered)) => {
            info!(
                agent = agent_name,
                "soul prompt rendered with runtime state"
            );
            Some(rendered)
        }
        Ok(None) => None,
        Err(e) => {
            warn!(
                agent = agent_name,
                error = %e,
                "failed to render soul prompt"
            );
            None
        }
    }
}

/// Load the external agent.md operational knowledge file.
///
/// Reads from `{config_dir}/agents/{agent_name}/agent.md`.
/// If the file doesn't exist, creates an empty placeholder so rara
/// can later populate it via `write-file`. Empty files are not injected.
fn load_agent_md(agent_name: &str) -> Option<String> {
    let agent_path = rara_paths::config_dir()
        .join("agents")
        .join(agent_name)
        .join("agent.md");

    if agent_path.exists() {
        match std::fs::read_to_string(&agent_path) {
            Ok(content) if !content.trim().is_empty() => {
                info!(agent = agent_name, path = %agent_path.display(), "loaded agent.md");
                return Some(content);
            }
            Ok(_) => {}
            Err(e) => {
                warn!(agent = agent_name, error = %e, "failed to read agent.md");
            }
        }
    }

    // Ensure the file exists with seed content for future updates
    ensure_agent_md(&agent_path, agent_name);
    None
}

/// Create the agent.md file and knowledge directory if they don't exist.
fn ensure_agent_md(path: &Path, agent_name: &str) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!(agent = agent_name, error = %e, "failed to create agent.md parent directory");
            return;
        }
        // Create knowledge/ subdirectory for per-tool knowledge files
        let knowledge_dir = parent.join("knowledge");
        if let Err(e) = std::fs::create_dir_all(&knowledge_dir) {
            warn!(agent = agent_name, error = %e, "failed to create knowledge directory");
        }
    }
    match std::fs::write(path, "") {
        Ok(()) => info!(agent = agent_name, path = %path.display(), "created empty agent.md"),
        Err(e) => warn!(agent = agent_name, error = %e, "failed to create agent.md"),
    }
}

fn build_runtime_contract_prompt(
    base_prompt: &str,
    has_kernel_tool: bool,
    max_children: Option<usize>,
) -> String {
    let mut prompt = format!(
        "{base_prompt}\n\n<context_contract>\nThe `tape` tool is your persistent memory.\n\n## \
         Tape actions\n- `anchor`: checkpoint + trim context. Older entries stay searchable via \
         `search`.\n- `search` / `entries`: recall details from before an anchor.\n- `anchors`: \
         list all checkpoints.\n- `checkout`: fork a new session from a past anchor (original \
         unchanged).\n\n## MUST anchor when:\n- Context is getting long or a [Context Usage \
         Warning] appears\n- A tool result exceeds ~2000 chars\n- Iterative tasks accumulate \
         large outputs (screenshots, scraping, listings)\n\n## SHOULD anchor when:\n- Completing \
         a logical work phase or switching subtasks\n- Processing multiple tool results in \
         sequence\n\n## MUST search before answering when:\n- The question refers to anything \
         before an anchor or outside current window\n- You need exact tokens, IDs, codes, names, \
         or quoted details from earlier context\n\n## Anchor best practices\n- Always include a \
         detailed `summary` and concrete `next_steps`\n- A missing summary = lost context\n- Use \
         `checkout` to retry from a past checkpoint or when the user asks to go \
         back\n\n</context_contract>"
    );

    let can_delegate = has_kernel_tool && max_children != Some(0);
    if can_delegate {
        prompt.push_str(
            "\n\n<delegation_contract>\nUse the `kernel` tool to delegate to child agents.\n\n## \
             MUST delegate when:\n- 2+ independent subtasks exist\n- Broad discovery + \
             implementation + verification across many files\n- Long tool-heavy execution that \
             would bloat your context\n\n## How:\n- `action: \"spawn\"`, `agent: \"worker\"` — \
             single focused task\n- `action: \"spawn_parallel\"` — multiple independent tasks\n- \
             Keep each worker's scope narrow; synthesize results in parent\n</delegation_contract>",
        );
    }

    prompt
}

/// Execute a single agent turn inline: build messages, stream LLM responses,
/// execute tool calls, and emit [`StreamEvent`]s directly.
///
/// Uses the new `LlmDriver` abstraction with first-class `reasoning_content`
/// (thinking tokens) support. The driver sends `StreamDelta` events through
/// an `mpsc` channel, which this function consumes.
///
/// # Cancellation
///
/// Respects `turn_cancel` at every `tokio::select!` point — both before the
/// stream starts and during delta consumption.
#[tracing::instrument(
    skip(
        handle,
        stream_handle,
        turn_cancel,
        tape,
        tape_name,
        output_interceptor,
        guard_pipeline,
        notification_bus
    ),
    fields(
        session_key = %session_key,
    )
)]
pub(crate) async fn run_agent_loop(
    handle: &KernelHandle,
    session_key: SessionKey,
    user_text: String,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
    tape: crate::memory::TapeService,
    tape_name: &str,
    mut tool_context: crate::tool::ToolContext,
    milestone_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
    output_interceptor: crate::tool::DynamicOutputInterceptor,
    guard_pipeline: Arc<GuardPipeline>,
    notification_bus: NotificationBusRef,
    rara_message_id: crate::io::MessageId,
) -> crate::error::Result<AgentTurnResult> {
    // Query context via syscalls.
    let manifest =
        handle
            .session_manifest(&session_key)
            .await
            .map_err(|e| KernelError::AgentExecution {
                message: format!("failed to get manifest: {e}"),
            })?;
    let full_tools = handle
        .session_tool_registry(session_key)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to get tool registry: {e}"),
        })?;

    // Filter tools by manifest.tools whitelist.
    let manifest_filtered = full_tools.filtered(&manifest.tools);
    if manifest_filtered.len() < full_tools.len() {
        info!(
            agent = %manifest.name,
            total = full_tools.len(),
            allowed = manifest_filtered.len(),
            allowlist = ?manifest.tools,
            "filtered tools by agent manifest allowlist"
        );
    }

    // Filter tools by user permissions — users can only see tools they are
    // authorized to use.  This prevents the LLM from even attempting to call
    // tools the user lacks permission for.
    let tools = {
        let user_id = &tool_context.user_id;
        match handle.security().user_store().get_by_name(user_id).await {
            Ok(Some(user)) => {
                let filtered = manifest_filtered.filtered_by_user(&user);
                if filtered.len() < manifest_filtered.len() {
                    let denied: Vec<String> = manifest_filtered
                        .iter()
                        .filter(|(name, _)| !user.can_use_tool(name))
                        .map(|(name, _)| name.to_string())
                        .collect();
                    info!(
                        user_id = user_id.as_str(),
                        ?denied,
                        "filtered tools by user permissions"
                    );
                }
                Arc::new(filtered)
            }
            _ => Arc::new(manifest_filtered),
        }
    };

    let max_iterations = manifest.max_iterations.unwrap_or(25);
    let has_kernel_tool = tools.get("kernel").is_some();
    let (effective_prompt, has_soul) = {
        let soul_text = resolve_soul_prompt(&manifest.name);
        match soul_text {
            Some(soul) => (format!("{soul}\n\n---\n\n{}", manifest.system_prompt), true),
            None => (manifest.system_prompt.clone(), false),
        }
    };
    // Append external agent.md (tool knowledge, CLI guides, operational notes)
    let effective_prompt = if let Some(agent_md) = load_agent_md(&manifest.name) {
        format!("{effective_prompt}\n\n<agent_knowledge>\n{agent_md}\n</agent_knowledge>")
    } else {
        effective_prompt
    };
    let effective_prompt =
        build_runtime_contract_prompt(&effective_prompt, has_kernel_tool, manifest.max_children);
    // Append available skills *after* the static runtime contract so that
    // the stable prefix (soul + system_prompt + agent.md + contract) stays
    // intact for provider-side prompt caching.
    let skills_block = handle.skills_prompt();
    let effective_prompt = if skills_block.is_empty() {
        effective_prompt
    } else {
        format!("{effective_prompt}\n\n{skills_block}")
    };
    let provider_hint = manifest.provider_hint.as_deref();

    // Resolve driver + model via the DriverRegistry syscall.
    let (driver, model) = handle
        .session_resolve_driver(session_key)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to resolve LLM driver: {e}"),
        })?;

    tracing::Span::current().record("model", model.as_str());

    let capabilities = ModelCapabilities::detect(provider_hint, &model);
    tool_context.context_window_tokens = capabilities.context_window_tokens;
    let input_text = user_text.clone();
    let tool_execution_timeout = handle.config().tool_execution_timeout;

    // Check model tool support
    let mut tool_defs = if tools.is_empty() {
        vec![]
    } else if capabilities.supports_tools {
        tools.to_llm_tool_definitions()
    } else {
        warn!(
            model_name = %model,
            provider_hint = ?provider_hint,
            reason = capabilities.tools_disabled_reason.unwrap_or("unknown"),
            "disabling tool calling for model without tool support"
        );
        vec![]
    };

    let mut tool_calls_made = 0usize;
    let mut last_accumulated_text = String::new();
    let turn_start = Instant::now();
    let mut iteration_traces: Vec<IterationTrace> = Vec::new();
    let mut llm_error_recovery_used = false;
    let mut context_window_recovery_used = false;
    let mut consecutive_silent_iters: usize = 0;
    let mut needs_anchor_reminder = false;
    let mut context_pressure_warning: Option<String> = None;
    let mut llm_error_recovery_message: Option<String> = None;
    // ── Pause turn state ─────────────────────────────────────────────
    let pause_interval = manifest.pause_turn_threshold.unwrap_or(15);
    let mut next_pause_at: usize = pause_interval;
    // ── Token & thinking metrics for UsageUpdate (#303) ──────────────
    // These are *cumulative* across all iterations within the turn.
    // `cumulative_output_tokens` sums completion_tokens from every iteration;
    // `input_tokens` in the emitted event is always the *latest* iteration's
    // prompt_tokens (= current context size), NOT a cumulative sum — because
    // each iteration re-sends the full context.
    let mut cumulative_output_tokens: u32 = 0;
    let mut cumulative_thinking_ms: u64 = 0;
    let user_id = Some(tool_context.user_id.as_str());

    // ── Context folding state ────────────────────────────────────────
    let fold_config = &handle.config().context_folding;
    // Recover last auto-fold anchor's entry ID from tape so the cooldown
    // survives across turns (not just within a single run_agent_loop call).
    let mut last_fold_entry_id: Option<u64> =
        fold::find_last_auto_fold_entry_id(&tape, tape_name).await;
    let mut fold_failed_this_turn = false;
    let context_folder = if fold_config.enabled {
        let fold_model = fold_config
            .fold_model
            .clone()
            .unwrap_or_else(|| model.clone());
        Some(fold::ContextFolder::new(driver.clone(), fold_model))
    } else {
        None
    };

    for iteration in 0..max_iterations {
        // ── Auto-fold: pressure-driven context compression ───────────
        // Runs BEFORE rebuild so the new anchor (if created) takes effect
        // in this iteration's context.  Disabled for the remainder of this
        // turn after any fold failure to avoid repeated failing LLM calls.
        if let Some(folder) = &context_folder {
            if !fold_failed_this_turn {
                if let Ok(tape_info) = tape.info(tape_name).await {
                    let pressure = tape_info.estimated_context_tokens as f64
                        / capabilities.context_window_tokens as f64;

                    if pressure > fold_config.fold_threshold {
                        let entries_since_fold = match last_fold_entry_id {
                            Some(id) => tape
                                .entries_after(tape_name, id)
                                .await
                                .map(|e| e.len())
                                .unwrap_or(0),
                            None => tape_info.entries,
                        };

                        if entries_since_fold >= fold_config.min_entries_between_folds {
                            info!(
                                pressure = %format!("{:.0}%", pressure * 100.0),
                                entries_since_fold,
                                "auto-fold: context pressure exceeded threshold, \
                                 creating anchor",
                            );

                            // Fetch current LLM messages and prior anchor summary.
                            let fold_messages = tape.build_llm_context(tape_name).await;
                            let prior_entries = tape.from_last_anchor(tape_name, None).await;
                            let prior_summary = prior_entries.as_ref().ok().and_then(|entries| {
                                crate::memory::anchor_summary_from_entries(entries)
                            });

                            match fold_messages {
                                Ok(msgs) => {
                                    match folder
                                        .fold_with_prior(
                                            prior_summary.as_deref(),
                                            &msgs,
                                            tape_info.estimated_context_tokens as usize,
                                        )
                                        .await
                                    {
                                        Ok(summary) => {
                                            let handoff = fold::ContextFolder::to_handoff_state(
                                                &summary, pressure,
                                            );
                                            if let Err(e) =
                                                tape.handoff(tape_name, "auto-fold", handoff).await
                                            {
                                                fold_failed_this_turn = true;
                                                warn!(
                                                    error = %e,
                                                    "auto-fold: failed to persist \
                                                     anchor, disabling for this turn"
                                                );
                                            } else {
                                                last_fold_entry_id =
                                                    tape.last_entry_id(tape_name).await.ok();
                                            }
                                        }
                                        Err(e) => {
                                            fold_failed_this_turn = true;
                                            warn!(
                                                error = %e,
                                                "auto-fold: LLM summarization \
                                                 failed, disabling for this turn; \
                                                 0.70/0.85 pressure warnings remain \
                                                 as fallback"
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    fold_failed_this_turn = true;
                                    warn!(
                                        error = %e,
                                        "auto-fold: failed to build LLM context \
                                         for folding, disabling for this turn"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Rebuild messages from tape each iteration (single source of truth) ──
        let mut messages = tape
            .rebuild_messages_for_llm(tape_name, user_id, &effective_prompt)
            .await
            .map_err(|e| KernelError::AgentExecution {
                message: format!("failed to rebuild messages from tape: {e}"),
            })?;

        // Conditional injections (tape search reminder only on first iteration)
        if iteration == 0 && should_remind_tape_search(&input_text) {
            messages.push(llm::Message::user(
                "[Recall Verification] The user is asking about a precise fact that may come from \
                 earlier context. If you don't have clear evidence in your current context, you \
                 MUST use tape.search to verify before answering.",
            ));
        }

        // Inject anchor reminder from previous iteration's large tool output
        if needs_anchor_reminder {
            messages.push(llm::Message::user(
                "[Large Tool Output] You just processed a large tool result that significantly \
                 bloats context. Before continuing, use tape with action:\"anchor\" to create a \
                 handoff with summary and next_steps. Use tape.search later for older details.",
            ));
            needs_anchor_reminder = false;
        }

        // Inject context pressure warning from previous iteration
        if let Some(warning) = context_pressure_warning.take() {
            messages.push(llm::Message::user(warning));
        }

        // Inject LLM error recovery message from previous iteration
        if let Some(recovery_msg) = llm_error_recovery_message.take() {
            messages.push(llm::Message::user(recovery_msg));
        }

        // Inject active background tasks status (first iteration only to
        // avoid repeated token cost in multi-iteration turns).
        let bg_tasks = if iteration == 0 {
            handle.background_tasks(&session_key)
        } else {
            Vec::new()
        };
        if !bg_tasks.is_empty() {
            let task_list: String = bg_tasks
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let elapsed = jiff::Timestamp::now()
                        .since(t.created_at)
                        .ok()
                        .map(|d| {
                            let secs = d.get_seconds();
                            if secs < 60 {
                                format!("{secs}s ago")
                            } else {
                                format!("{}m ago", secs / 60)
                            }
                        })
                        .unwrap_or_else(|| "just now".to_string());
                    format!(
                        "  {}. task_id={} name={} — {} (started {}, triggered_by={})",
                        i + 1,
                        t.child_key,
                        t.agent_name,
                        t.description,
                        elapsed,
                        t.trigger_message_id,
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            messages.push(crate::llm::Message::user(format!(
                "[Active Background Tasks]\nYou have {} background task(s) \
                 running:\n{task_list}\nResults will be delivered automatically when complete. \
                 Use cancel-background(task_id) to cancel if needed.",
                bg_tasks.len()
            )));
        }

        messages = sanitize_messages_for_llm(&messages);

        let iter_span = info_span!(
            "llm_iteration",
            iter = iteration,
            model = model.as_str(),
            first_token_ms = tracing::field::Empty,
            stream_ms = tracing::field::Empty,
            has_tools = tracing::field::Empty,
            tool_count = tracing::field::Empty,
        );
        let _iter_guard = iter_span.enter();

        stream_handle.emit(StreamEvent::Progress {
            stage: crate::io::stages::THINKING.to_string(),
        });
        info!(
            iteration,
            messages_count = messages.len(),
            "calling LLM (inline streaming via LlmDriver)"
        );
        info!(
            iteration,
            model = model.as_str(),
            tools_count = tool_defs.len(),
            messages = %serde_json::to_string(&messages).unwrap_or_default(),
            "LLM request"
        );

        // Build completion request
        let request = llm::CompletionRequest {
            model:               model.clone(),
            messages:            messages.clone(),
            tools:               tool_defs.clone(),
            temperature:         Some(0.7),
            max_tokens:          None,
            thinking:            None,
            tool_choice:         if tool_defs.is_empty() {
                llm::ToolChoice::None
            } else {
                llm::ToolChoice::Auto
            },
            parallel_tool_calls: !tool_defs.is_empty() && capabilities.supports_parallel_tool_calls,
            // Prevent LLM repetition loops — small models (e.g. step-3.5-flash) are
            // especially prone to generating the same paragraph 3-4 times without
            // a penalty. 0.3 is a conservative value that curbs repetition without
            // degrading output quality. See #317.
            frequency_penalty:   Some(0.3),
        };

        // Start streaming via LlmDriver
        let (tx, mut rx) = mpsc::channel::<llm::StreamDelta>(128);
        let driver_clone = Arc::clone(&driver);
        let request_clone = request;

        // Spawn driver.stream() — it sends deltas to tx and returns when done.
        let stream_task = tokio::spawn(async move { driver_clone.stream(request_clone, tx).await });

        stream_handle.emit(StreamEvent::Progress {
            stage: format!("Waiting for LLM response (iteration {})...", iteration + 1),
        });

        // Consume streaming deltas
        let stream_start = Instant::now();
        let mut first_token_at: Option<Instant> = None;
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut pending_tool_calls: HashMap<u32, PendingToolCall> = HashMap::new();
        let mut has_tool_calls = false;
        let mut last_usage: Option<llm::Usage> = None;
        // Per-iteration reasoning timer — set on the first ReasoningDelta,
        // settled (added to cumulative_thinking_ms) on either:
        //   a) the first TextDelta (reasoning → content transition), or
        //   b) the Done delta (model finished without content, e.g. tool-only).
        // Must be `take()`-d exactly once per iteration to avoid double-counting.
        let mut reasoning_start: Option<Instant> = None;

        loop {
            let delta = tokio::select! {
                delta = rx.recv() => delta,
                _ = turn_cancel.cancelled() => {
                    stream_task.abort();
                    info!("LLM turn cancelled during streaming");
                    return Err(KernelError::Interrupted);
                }
            };

            let Some(delta) = delta else {
                // Channel closed — driver finished (or errored).
                break;
            };

            match delta {
                llm::StreamDelta::TextDelta { text } => {
                    if !text.is_empty() {
                        if first_token_at.is_none() {
                            first_token_at = Some(Instant::now());
                            iter_span.record(
                                "first_token_ms",
                                first_token_at
                                    .unwrap()
                                    .duration_since(stream_start)
                                    .as_millis() as u64,
                            );
                        }
                        // Settle reasoning timer on the FIRST TextDelta only
                        // (accumulated_text is still empty → this is the transition
                        // from reasoning to content). Uses take() so it fires once.
                        if accumulated_text.is_empty() {
                            if let Some(rs) = reasoning_start.take() {
                                cumulative_thinking_ms += rs.elapsed().as_millis() as u64;
                            }
                        }
                        accumulated_text.push_str(&text);
                        stream_handle.emit(StreamEvent::TextDelta { text });
                    }
                }
                llm::StreamDelta::ReasoningDelta { text } => {
                    if !text.is_empty() {
                        if first_token_at.is_none() {
                            first_token_at = Some(Instant::now());
                        }
                        if reasoning_start.is_none() {
                            reasoning_start = Some(Instant::now());
                        }
                        accumulated_reasoning.push_str(&text);
                        // KEY: emit ReasoningDelta to the stream!
                        stream_handle.emit(StreamEvent::ReasoningDelta { text });
                    }
                }
                llm::StreamDelta::ToolCallStart { index, id, name } => {
                    pending_tool_calls
                        .entry(index)
                        .or_insert_with(|| PendingToolCall {
                            id,
                            name,
                            arguments_buf: String::new(),
                        });
                }
                llm::StreamDelta::ToolCallArgumentsDelta { index, arguments } => {
                    if let Some(tc) = pending_tool_calls.get_mut(&index) {
                        tc.arguments_buf.push_str(&arguments);
                    }
                }
                llm::StreamDelta::Done { stop_reason, usage } => {
                    has_tool_calls = stop_reason == llm::StopReason::ToolCalls;
                    if let Some(u) = usage {
                        if let Err(e) = tape
                            .append_event(
                                tape_name,
                                "llm.run",
                                serde_json::json!({
                                    "usage": {
                                        "prompt_tokens": u.prompt_tokens,
                                        "completion_tokens": u.completion_tokens,
                                        "total_tokens": u.total_tokens
                                    }
                                }),
                            )
                            .await
                        {
                            warn!(error = %e, "failed to persist llm usage event");
                        }
                    }
                    last_usage = usage;
                    // Fallback: settle reasoning if no TextDelta arrived
                    // (e.g. tool-only iteration with extended thinking).
                    if let Some(rs) = reasoning_start.take() {
                        cumulative_thinking_ms += rs.elapsed().as_millis() as u64;
                    }
                    // Emit cumulative usage to stream consumers (Telegram progress UX).
                    // input_tokens = latest iteration's prompt_tokens (current context size);
                    // output_tokens = sum of all iterations' completion_tokens.
                    if let Some(ref u) = last_usage {
                        cumulative_output_tokens =
                            cumulative_output_tokens.saturating_add(u.completion_tokens);
                        stream_handle.emit(StreamEvent::UsageUpdate {
                            input_tokens:  u.prompt_tokens,
                            output_tokens: cumulative_output_tokens,
                            thinking_ms:   cumulative_thinking_ms,
                        });
                    }
                    break;
                }
            }
        }

        // Signal forwarder to discard intermediate narration text.
        // This MUST happen before ToolCallStart so the forwarder clears
        // state while the broadcast channel is not congested.
        if has_tool_calls {
            stream_handle.emit(StreamEvent::TextClear);
        }

        // Wait for the stream task to complete (the driver accumulates the
        // full response internally).
        let driver_result = match stream_task.await {
            Ok(result) => result,
            Err(join_err) if join_err.is_cancelled() => {
                return Err(KernelError::Interrupted);
            }
            Err(join_err) => {
                return Err(KernelError::AgentExecution {
                    message: format!("driver stream task panicked: {join_err}"),
                });
            }
        };

        if let Err(ref e) = driver_result {
            if !context_window_recovery_used && matches!(e, KernelError::ContextWindow) {
                context_window_recovery_used = true;
            }

            if !llm_error_recovery_used && crate::error::is_retryable_provider_error(e) {
                warn!(
                    iteration,
                    model = model.as_str(),
                    error = %e,
                    "LLM stream error, attempting recovery without tools"
                );
                llm_error_recovery_used = true;
                llm_error_recovery_message = Some(format!(
                    "[System] The previous request encountered a server error ({e}). Please reply \
                     to the user's question directly without using tools."
                ));
                tool_defs = vec![];
                continue;
            }

            error!(
                iteration,
                model = model.as_str(),
                error = %e,
                "LLM driver stream error"
            );
            return Err(KernelError::AgentExecution {
                message: format!("Model \"{model}\" returned an error during streaming: {e}"),
            });
        }

        iter_span.record("stream_ms", stream_start.elapsed().as_millis() as u64);
        iter_span.record("has_tools", has_tool_calls);

        {
            let text_preview: String = accumulated_text.chars().take(500).collect();
            let tool_call_names: Vec<&str> = pending_tool_calls
                .values()
                .map(|tc| tc.name.as_str())
                .collect();
            info!(
                iteration,
                stream_ms = stream_start.elapsed().as_millis() as u64,
                has_tool_calls,
                tool_calls = ?tool_call_names,
                text_preview = %text_preview,
                "LLM response"
            );
        }

        // Terminal response (no tool calls, or recovery iteration must exit)
        if !has_tool_calls || llm_error_recovery_used {
            // Persist final assistant message to tape.
            let meta = {
                let mut m = serde_json::json!({
                    "rara_message_id": rara_message_id.to_string(),
                });
                if let Some(u) = last_usage.as_ref() {
                    m["usage"] = serde_json::json!({
                        "prompt_tokens": u.prompt_tokens,
                        "completion_tokens": u.completion_tokens,
                        "total_tokens": u.total_tokens,
                    });
                }
                Some(m)
            };
            let _ = tape
                .append_message(
                    tape_name,
                    serde_json::json!({
                        "role": "assistant",
                        "content": &accumulated_text,
                    }),
                    meta.clone(),
                )
                .await;

            let first_token_ms =
                first_token_at.map(|t| t.duration_since(stream_start).as_millis() as u64);
            let stream_ms = stream_start.elapsed().as_millis() as u64;
            let text_preview: String = accumulated_text.chars().take(200).collect();
            iteration_traces.push(IterationTrace {
                index: iteration,
                first_token_ms,
                stream_ms,
                text_preview,
                reasoning_text: if accumulated_reasoning.is_empty() {
                    None
                } else {
                    Some(accumulated_reasoning)
                },
                tool_calls: vec![],
            });
            let trace = TurnTrace {
                duration_ms: turn_start.elapsed().as_millis() as u64,
                model: model.clone(),
                input_text: Some(input_text.clone()),
                iterations: iteration_traces,
                final_text_len: accumulated_text.len(),
                total_tool_calls: tool_calls_made,
                success: true,
                error: None,
                rara_message_id,
            };
            // Best-effort mood update — failure is silently logged, never
            // blocks the response.
            if has_soul {
                if let Some(inf) = crate::mood::infer_mood(&messages) {
                    crate::mood::update_soul_mood(&manifest.name, &inf);
                }
            }

            return Ok(AgentTurnResult {
                text: accumulated_text,
                iterations: iteration + 1,
                tool_calls: tool_calls_made,
                model: model.clone(),
                trace,
            });
        }

        // Stash for partial-result reporting
        last_accumulated_text = accumulated_text.clone();

        // Assemble and execute tool calls
        let mut sorted_indices: Vec<u32> = pending_tool_calls.keys().copied().collect();
        sorted_indices.sort_unstable();

        let tool_call_list: Vec<PendingToolCall> = sorted_indices
            .into_iter()
            .filter_map(|idx| pending_tool_calls.remove(&idx))
            .map(|mut tc| {
                // LLM may omit argument deltas for no-arg tool calls,
                // leaving arguments_buf empty. Normalize to valid JSON
                // so downstream parsing and LLM API round-trips succeed.
                if tc.arguments_buf.trim().is_empty() {
                    tc.arguments_buf = "{}".to_string();
                }
                tc
            })
            .collect();

        // Parse and validate tool calls
        let mut valid_tool_calls = Vec::new();
        let mut assistant_tool_calls = Vec::new();
        for tool_call in tool_call_list {
            tool_calls_made += 1;

            // Emit ToolCallStart BEFORE parsing so the forwarder always
            // receives it — even if argument parsing fails below.
            stream_handle.emit(StreamEvent::ToolCallStart {
                name:      tool_call.name.clone(),
                id:        tool_call.id.clone(),
                arguments: serde_json::Value::Object(Default::default()),
            });

            let args = match parse_tool_call_arguments(&tool_call.arguments_buf) {
                Ok(args) => args,
                Err(error_message) => {
                    warn!(tool = %tool_call.name, %error_message, "tool argument parsing failed");
                    // Persist parse-error tool result to tape so the next
                    // iteration rebuild includes it.
                    let _ = tape
                        .append_tool_result(
                            tape_name,
                            serde_json::json!({
                                "results": [{
                                    "tool_call_id": &tool_call.id,
                                    "error": &error_message,
                                }]
                            }),
                            Some(
                                serde_json::json!({"rara_message_id": rara_message_id.to_string()}),
                            ),
                        )
                        .await;
                    let raw_args: String = tool_call.arguments_buf.chars().take(100).collect();
                    stream_handle.emit(StreamEvent::ToolCallEnd {
                        id:             tool_call.id,
                        result_preview: error_message.chars().take(200).collect(),
                        success:        false,
                        error:          Some(format!("{error_message} | args: {raw_args}")),
                    });
                    continue;
                }
            };

            assistant_tool_calls.push(llm::ToolCallRequest {
                id:        tool_call.id.clone(),
                name:      tool_call.name.clone(),
                arguments: tool_call.arguments_buf.clone(),
            });
            if let Some(ref mtx) = milestone_tx {
                let _ = mtx
                    .send(crate::io::AgentEvent::Milestone {
                        stage:  "tool_call_start".to_string(),
                        detail: Some(tool_call.name.clone()),
                    })
                    .await;
            }
            valid_tool_calls.push((tool_call.id, tool_call.name, args));
        }

        // Persist assistant message with tool calls to tape.
        if !assistant_tool_calls.is_empty() {
            let calls_json: Vec<serde_json::Value> = assistant_tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "function": { "name": tc.name, "arguments": tc.arguments }
                    })
                })
                .collect();
            let tool_call_meta = {
                let mut m = serde_json::json!({
                    "rara_message_id": rara_message_id.to_string(),
                });
                if let Some(u) = last_usage.as_ref() {
                    m["usage"] = serde_json::json!({
                        "prompt_tokens": u.prompt_tokens,
                        "completion_tokens": u.completion_tokens,
                        "total_tokens": u.total_tokens,
                    });
                }
                Some(m)
            };
            let _ = tape
                .append_tool_call(
                    tape_name,
                    serde_json::json!({ "calls": calls_json }),
                    tool_call_meta,
                )
                .await;
        }

        iter_span.record("tool_count", valid_tool_calls.len());

        // Resolve user for runtime permission guard (defense in depth).
        let runtime_user = handle
            .security()
            .user_store()
            .get_by_name(&tool_context.user_id)
            .await
            .ok()
            .flatten();

        // Execute all tool calls concurrently (with timing for traces)
        let tool_futures: Vec<_> = valid_tool_calls
            .iter()
            .map(|(_id, name, args)| {
                let tool = tools.get(name);
                let args = args.clone();
                let name = name.clone();
                let tc = tool_context.clone();
                let user_ref = runtime_user.clone();
                let tool_cancel = turn_cancel.clone();
                let output_interceptor = output_interceptor.clone();
                let guard_pipeline = guard_pipeline.clone();
                let notification_bus = notification_bus.clone();
                let approval_manager = Arc::clone(handle.security().approval());
                let session_key_for_guard = session_key;
                let tool_span = info_span!(
                    "tool_exec",
                    tool_name = name.as_str(),
                    success = tracing::field::Empty,
                );
                async move {
                    let _guard = tool_span.enter();
                    let tool_start = Instant::now();
                    info!(tool = %name, args = %args, "tool call started");

                    // Runtime permission guard — deny if user cannot use this tool.
                    if let Some(ref user) = user_ref {
                        if !user.can_use_tool(&name) {
                            tool_span.record("success", false);
                            let err = format!(
                                "permission denied: user '{}' cannot use tool '{name}'",
                                user.name
                            );
                            warn!("{err}");
                            let dur = tool_start.elapsed().as_millis() as u64;
                            return (
                                false,
                                crate::tool::ToolOutput::from(serde_json::json!({ "error": &err })),
                                Some(err),
                                dur,
                            );
                        }
                    }

                    // Security guard check — taint + pattern.
                    // If blocked, request user approval before denying.
                    if let GuardVerdict::Blocked {
                        layer,
                        reason,
                        tool_name: blocked_tool,
                    } = guard_pipeline.pre_execute(&session_key_for_guard, &name, &args)
                    {
                        warn!(
                            tool = %blocked_tool,
                            %layer,
                            %reason,
                            "tool call blocked by guard, requesting user approval"
                        );

                        let risk_level = crate::security::ApprovalManager::classify_risk(&blocked_tool);
                        let approval_req = crate::security::ApprovalRequest {
                            id:           uuid::Uuid::new_v4(),
                            session_key:  session_key_for_guard,
                            tool_name:    blocked_tool.clone(),
                            tool_args:    args.clone(),
                            summary:      format!("Guard blocked ({layer}): {reason}"),
                            risk_level,
                            requested_at: jiff::Timestamp::now(),
                            timeout_secs: 120,
                        };

                        let decision = approval_manager.request_approval(approval_req).await;

                        match decision {
                            crate::security::ApprovalDecision::Approved => {
                                info!(
                                    tool = %blocked_tool,
                                    %layer,
                                    %reason,
                                    "guard block overridden by user approval"
                                );
                                // Fall through to normal tool execution.
                            }
                            _ => {
                                // Denied or timed out — block the tool call.
                                tool_span.record("success", false);

                                let agent_id = session_key_for_guard.into_inner();
                                notification_bus
                                    .publish(KernelNotification::GuardDenied {
                                        agent_id,
                                        tool_name: blocked_tool.clone(),
                                        reason: reason.clone(),
                                        timestamp: jiff::Timestamp::now(),
                                    })
                                    .await;

                                let err = format!("security guard ({layer}): {reason}");
                                let dur = tool_start.elapsed().as_millis() as u64;
                                return (
                                    false,
                                    crate::tool::ToolOutput::from(serde_json::json!({ "error": &err })),
                                    Some(err),
                                    dur,
                                );
                            }
                        }
                    }

                    if let Some(tool) = tool {
                        let args_snapshot = args.to_string();
                        let tool_result = tokio::select! {
                            result = tool.execute(args, &tc) => result,
                            _ = tool_cancel.cancelled() => {
                                let dur = tool_start.elapsed().as_millis() as u64;
                                tool_span.record("success", false);
                                return (
                                    false,
                                    crate::tool::ToolOutput::from(
                                        serde_json::json!({ "error": "interrupted by user" }),
                                    ),
                                    Some("interrupted by user".to_string()),
                                    dur,
                                );
                            }
                        };

                        match tool_result {
                            Ok(result) => {
                                tool_span.record("success", true);
                                let dur = tool_start.elapsed().as_millis() as u64;
                                info!(tool = %name, duration_ms = dur, "tool call succeeded");
                                let result = {
                                    let guard = output_interceptor.read().await;
                                    if let Some(ref interceptor) = *guard {
                                        interceptor.intercept(&name, result).await
                                    } else {
                                        result
                                    }
                                };
                                guard_pipeline.post_execute(&session_key_for_guard, &name);
                                (true, result, None::<String>, dur)
                            }
                            Err(e) => {
                                tool_span.record("success", false);
                                warn!(tool = %name, args = %args_snapshot, error = %e, "tool execution failed");
                                let dur = tool_start.elapsed().as_millis() as u64;
                                (
                                    false,
                                    crate::tool::ToolOutput::from(
                                        serde_json::json!({ "error": e.to_string() }),
                                    ),
                                    Some(e.to_string()),
                                    dur,
                                )
                            }
                        }
                    } else {
                        tool_span.record("success", false);
                        let err = format!("tool not found: {name}");
                        warn!(%err);
                        let dur = tool_start.elapsed().as_millis() as u64;
                        (
                            false,
                            crate::tool::ToolOutput::from(serde_json::json!({ "error": &err })),
                            Some(err),
                            dur,
                        )
                    }
                }
            })
            .collect();

        let results = tokio::select! {
            results = tokio::time::timeout(tool_execution_timeout, futures::future::join_all(tool_futures)) => {
                match results {
                    Ok(results) => results,
                    Err(_) => {
                        return Err(KernelError::AgentExecution {
                            message: format!(
                                "tool execution timed out after {}s",
                                tool_execution_timeout.as_secs()
                            ),
                        });
                    }
                }
            }
            _ = turn_cancel.cancelled() => {
                return Err(KernelError::Interrupted);
            }
        };

        // Persist tool results to tape.
        if !results.is_empty() {
            let results_json: Vec<serde_json::Value> = results
                .iter()
                .map(|(_success, result, _err, _dur)| result.json.clone())
                .collect();
            let tool_names: Vec<String> = valid_tool_calls
                .iter()
                .map(|(_id, name, _args)| name.clone())
                .collect();
            let _ = tape
                .append_tool_result(
                    tape_name,
                    serde_json::json!({ "results": results_json.clone() }),
                    Some(serde_json::json!({"rara_message_id": rara_message_id.to_string()})),
                )
                .await;
            if should_remind_tape_anchor(&tool_names, &results_json) {
                needs_anchor_reminder = true;
            }
        }

        // Build tool call traces from results
        let mut tool_call_traces: Vec<ToolCallTrace> = Vec::with_capacity(results.len());

        // Emit ToolCallEnd events and append tool response messages
        for ((id, name, args), (success, result, err, duration_ms)) in
            valid_tool_calls.iter().zip(results.iter())
        {
            let result_str = result.json.to_string();
            let result_preview = truncate_preview(&result_str, RESULT_PREVIEW_MAX_BYTES);

            stream_handle.emit(StreamEvent::ToolCallEnd {
                id:             id.clone(),
                result_preview: result_preview.clone(),
                success:        *success,
                error:          err.clone(),
            });
            if let Some(ref mtx) = milestone_tx {
                let _ = mtx
                    .send(crate::io::AgentEvent::Milestone {
                        stage:  "tool_call_end".to_string(),
                        detail: Some(format!(
                            "{}: {}",
                            name,
                            if *success { "ok" } else { "error" }
                        )),
                    })
                    .await;
            }

            tool_call_traces.push(ToolCallTrace {
                name: name.clone(),
                id: id.clone(),
                duration_ms: *duration_ms,
                success: *success,
                arguments: args.clone(),
                result_preview,
                error: err.clone(),
            });
        }

        // Collect iteration trace (with tool calls)
        {
            let first_token_ms =
                first_token_at.map(|t| t.duration_since(stream_start).as_millis() as u64);
            let stream_ms = stream_start.elapsed().as_millis() as u64;
            let text_preview: String = accumulated_text.chars().take(200).collect();
            iteration_traces.push(IterationTrace {
                index: iteration,
                first_token_ms,
                stream_ms,
                text_preview,
                reasoning_text: if accumulated_reasoning.is_empty() {
                    None
                } else {
                    Some(accumulated_reasoning.clone())
                },
                tool_calls: tool_call_traces,
            });
        }

        // ── Pause turn check ─────────────────────────────────────────
        // When cumulative tool calls reach the threshold, pause and ask
        // the user whether to continue or stop.
        if pause_interval > 0 && tool_calls_made >= next_pause_at {
            let elapsed_secs = turn_start.elapsed().as_secs();
            stream_handle.emit(StreamEvent::PauseTurn {
                session_key: session_key.to_string(),
                tool_calls_made,
                elapsed_secs,
            });

            let (tx, rx) = tokio::sync::oneshot::channel();
            handle.register_pause_turn(&session_key, tx);

            info!(
                tool_calls_made,
                next_pause_at,
                elapsed_secs,
                "agent loop paused at tool call threshold, awaiting user decision"
            );

            let decision = tokio::select! {
                result = tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    rx,
                ) => result,
                _ = turn_cancel.cancelled() => {
                    return Err(KernelError::Interrupted);
                }
            };

            match decision {
                Ok(Ok(crate::io::PauseTurnDecision::Continue)) => {
                    info!(tool_calls_made, "user chose to continue agent loop");
                    next_pause_at = tool_calls_made + pause_interval;
                    stream_handle.emit(StreamEvent::PauseTurnResolved {
                        session_key: session_key.to_string(),
                        continued: true,
                    });
                }
                _ => {
                    // Stop or Timeout or channel closed
                    warn!(tool_calls_made, "agent loop stopped by user or timeout");
                    stream_handle.emit(StreamEvent::PauseTurnResolved {
                        session_key: session_key.to_string(),
                        continued: false,
                    });
                    break;
                }
            }
        }

        // ── Runtime context guard ──────────────────────────────────────
        // Evaluate context pressure from the tape's estimated token count
        // (which reflects actual usage metadata) rather than from the
        // post-trim rebuilt messages, to avoid underestimating pressure.
        if let Ok(tape_info) = tape.info(tape_name).await {
            let pressure = classify_context_pressure(
                tape_info.estimated_context_tokens as usize,
                capabilities.context_window_tokens,
            );
            match pressure {
                ContextPressure::Critical { usage_ratio, .. } => {
                    context_pressure_warning = Some(format!(
                        "[Context Usage Critical] Current context ~{} tokens ({:.0}%), context \
                         window capacity {} tokens. You MUST immediately create a tape anchor \
                         with summary and next_steps.",
                        tape_info.estimated_context_tokens,
                        usage_ratio * 100.0,
                        capabilities.context_window_tokens,
                    ));
                }
                ContextPressure::Warning { usage_ratio, .. } => {
                    context_pressure_warning = Some(format!(
                        "[Context Usage Warning] Current context ~{} tokens ({:.0}%), context \
                         window capacity {} tokens. You SHOULD consider creating a tape anchor.",
                        tape_info.estimated_context_tokens,
                        usage_ratio * 100.0,
                        capabilities.context_window_tokens,
                    ));
                }
                ContextPressure::Normal => {}
            }
        }

        // Track consecutive silent (tool-only, no text) iterations and emit
        // a Progress event so the user knows we're still working.
        if accumulated_text.len() == last_accumulated_text.len() {
            consecutive_silent_iters += 1;
        } else {
            consecutive_silent_iters = 0;
        }
        if consecutive_silent_iters >= 3 {
            stream_handle.emit(StreamEvent::Progress {
                stage: format!("Processing... ({tool_calls_made} steps completed)"),
            });
            consecutive_silent_iters = 0;
        }
    }

    // Max iterations exhausted — return partial results with failure markers
    warn!(
        max_iterations,
        tool_calls_made, "inline agent loop hit max iterations limit, returning partial results"
    );
    let exhaustion_error = format!(
        "max iterations exhausted ({max_iterations} iterations, {tool_calls_made} tool calls)"
    );
    // Emit a stream warning so adapters (Telegram, SSE) can surface it to the
    // user immediately.
    stream_handle.emit(StreamEvent::Progress {
        stage: format!("[警告] 已达到最大迭代次数（{max_iterations}），任务可能未完成。"),
    });
    // If the agent spent the entire turn doing tool calls and produced no
    // visible text, synthesise a fallback message so the user is not left with
    // a blank response.
    if last_accumulated_text.is_empty() {
        last_accumulated_text =
            format!("[已达到最大迭代次数，任务未完成。已执行 {tool_calls_made} 次工具调用。]");
    }
    let trace = TurnTrace {
        duration_ms: turn_start.elapsed().as_millis() as u64,
        model: model.clone(),
        input_text: Some(input_text.clone()),
        iterations: iteration_traces,
        final_text_len: last_accumulated_text.len(),
        total_tool_calls: tool_calls_made,
        success: false,
        error: Some(exhaustion_error),
        rara_message_id,
    };
    // Best-effort mood update — failure is silently logged, never blocks the
    // response.
    if has_soul {
        if let Ok(msgs) = tape
            .rebuild_messages_for_llm(tape_name, user_id, &effective_prompt)
            .await
        {
            if let Some(inf) = crate::mood::infer_mood(&msgs) {
                crate::mood::update_soul_mood(&manifest.name, &inf);
            }
        }
    }

    Ok(AgentTurnResult {
        text: last_accumulated_text,
        iterations: max_iterations,
        tool_calls: tool_calls_made,
        model: model.clone(),
        trace,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ContextPressure, build_runtime_contract_prompt, classify_context_pressure,
        resolve_soul_prompt, should_remind_tape_anchor, should_remind_tape_search,
    };

    #[test]
    fn classify_context_pressure_returns_normal_below_threshold() {
        assert_eq!(
            classify_context_pressure(500, 1_000),
            ContextPressure::Normal
        );
    }

    #[test]
    fn classify_context_pressure_returns_warning_at_warn_threshold() {
        // 750 / 1000 = 0.75, above CONTEXT_WARN_THRESHOLD (0.70)
        assert!(matches!(
            classify_context_pressure(750, 1_000),
            ContextPressure::Warning { .. }
        ));
    }

    #[test]
    fn classify_context_pressure_returns_critical_at_critical_threshold() {
        // 900 / 1000 = 0.90, above CONTEXT_CRITICAL_THRESHOLD (0.85)
        assert!(matches!(
            classify_context_pressure(900, 1_000),
            ContextPressure::Critical { .. }
        ));
    }

    #[test]
    fn recall_questions_trigger_tape_search_reminder() {
        assert!(should_remind_tape_search(
            "What is the exact credential from the beginning of this conversation?"
        ));
        assert!(!should_remind_tape_search(
            "Summarize the current implementation in two bullets."
        ));
    }

    #[test]
    fn large_file_results_trigger_anchor_reminder() {
        assert!(should_remind_tape_anchor(
            &[String::from("read-file")],
            &[json!({ "content": "x".repeat(8_000) })]
        ));
        assert!(!should_remind_tape_anchor(
            &[String::from("read-file")],
            &[json!({ "content": "short" })]
        ));
    }

    #[test]
    fn runtime_contract_prompt_includes_tape_and_delegation_rules() {
        let prompt = build_runtime_contract_prompt("base", true, None);
        assert!(prompt.contains("<context_contract>"));
        assert!(prompt.contains("`tape`"));
        assert!(prompt.contains("- `anchor`: checkpoint + trim context."));
        assert!(prompt.contains("- `search` / `entries`: recall details from before an anchor."));
        assert!(prompt.contains(
            "You need exact tokens, IDs, codes, names, or quoted details from earlier context"
        ));
        assert!(prompt.contains("Always include a detailed `summary` and concrete `next_steps`"));
        assert!(prompt.contains("<delegation_contract>"));
        assert!(prompt.contains("action: \"spawn\""));
        assert!(prompt.contains("action: \"spawn_parallel\""));
        assert!(prompt.contains("agent: \"worker\""));
    }

    #[test]
    fn runtime_contract_prompt_keeps_tape_rules_without_kernel() {
        let prompt = build_runtime_contract_prompt("base", false, None);
        assert!(prompt.contains("<context_contract>"));
        assert!(!prompt.contains("<delegation_contract>"));
        assert!(!prompt.contains("action: \"spawn_parallel\""));
    }

    #[test]
    fn runtime_contract_prompt_skips_delegation_when_children_disabled() {
        let prompt = build_runtime_contract_prompt("base", true, Some(0));
        assert!(prompt.contains("<context_contract>"));
        assert!(!prompt.contains("<delegation_contract>"));
        assert!(!prompt.contains("action: \"spawn\""));
    }

    #[test]
    fn resolve_soul_prompt_returns_none_for_unknown_agent() {
        let result = resolve_soul_prompt("__nonexistent_test_agent__");
        assert!(result.is_none());
    }

    #[test]
    fn resolve_soul_prompt_returns_some_for_builtin_agent() {
        let result = resolve_soul_prompt("rara");
        assert!(result.is_some());
        assert!(result.unwrap().contains("Identity: rara"));
    }
}
