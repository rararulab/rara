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

pub mod effect;
pub mod fold;
pub(crate) mod loop_breaker;
pub mod machine;
pub(crate) mod repetition;
pub mod runner;

/// Maximum **byte** length for child/worker agent results passed back to
/// the parent context.  Child agents are instructed to self-summarize
/// (target ≈ 1 500 chars) via their system prompt; this generous byte-level
/// fallback only triggers when self-summarization produces unexpectedly
/// large output.  Truncation uses `str::floor_char_boundary()` to avoid
/// splitting multi-byte UTF-8 characters.
pub(crate) const CHILD_RESULT_SAFETY_LIMIT_BYTES: usize = 8000;

/// Structured-output instructions appended to child agent system prompts
/// so they self-summarize before returning results to the parent.
pub(crate) const STRUCTURED_OUTPUT_SUFFIX: &str =
    "\n\nWhen done, provide a structured result:\n1. Summary (2-3 sentences of what you did and \
     the outcome)\n2. Key changes or findings (bullet points)\n3. Issues encountered (if \
     any)\nKeep your final response concise — under 1500 characters.";

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
    guard::pipeline::{GuardLayer, GuardPipeline, GuardVerdict},
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
/// User-turn count since last anchor at which a session-length reminder is
/// injected.  Unlike the pressure-based thresholds this is a pure turn count
/// and does not depend on token estimates.
const TURN_REMINDER_THRESHOLD: usize = 8;
const _: () = {
    assert!(TURN_REMINDER_THRESHOLD >= 4, "threshold too low");
    assert!(TURN_REMINDER_THRESHOLD <= 20, "threshold too high");
};
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
    #[default]
    Reactive,
    /// Plan-execute mode (v2). The agent first generates a plan, then
    /// executes each step with verification. Activated via `/plan` prefix
    /// or `default_execution_mode: plan` in agent manifest.
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
    pub tools:                  Vec<crate::tool::ToolName>,
    /// Tool names the agent is NOT allowed to use (denylist).
    ///
    /// Applied after `tools` allowlist filtering. If `tools` is empty (inherit
    /// all), excluded tools are removed from the full set. If `tools` is
    /// explicit, excluded tools are additionally removed.
    #[serde(default)]
    pub excluded_tools:         Vec<crate::tool::ToolName>,
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
    /// Per-turn tool call ceiling that triggers a limit requiring user
    /// confirmation before the agent loop continues.
    ///
    /// When cumulative `tool_calls_made >= tool_call_limit`, the loop
    /// emits a [`StreamEvent::ToolCallLimit`] and blocks on a oneshot channel
    /// for up to 120 seconds. If the user continues, the next limit fires
    /// after another `tool_call_limit` calls (i.e. the threshold is
    /// additive, not reset to zero).
    ///
    /// **Default: `0` (disabled).** Set to a positive value in the agent
    /// manifest YAML to enable. Only channels with interactive decision UI
    /// (e.g. Telegram inline keyboard) should enable this; channels without
    /// UI would hit the 120s timeout and silently stop.
    #[serde(default)]
    pub tool_call_limit:        Option<usize>,
    /// Timeout in seconds for plan-mode worker steps. When a worker exceeds
    /// this duration it is terminated and the step is treated as failed.
    ///
    /// **Default: `None` (uses 300s fallback).**
    #[serde(default)]
    pub worker_timeout_secs:    Option<u64>,
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
pub(crate) fn truncate_preview(s: &str, max_bytes: usize) -> String {
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
    /// Structured cascade trace built in real time during the turn.
    pub cascade:    crate::cascade::CascadeTrace,
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
            cascade:    crate::cascade::CascadeTrace::empty(),
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

/// Returns `true` if any tool result JSON indicates a tape anchor was created.
///
/// Checks result payloads for known anchor-creation signatures rather than
/// hardcoding tool names, so new anchor-creating tools are automatically
/// covered.
fn did_create_anchor(results_json: &[serde_json::Value]) -> bool {
    results_json.iter().any(|json| {
        // `tape-anchor` tool returns {"anchor_name": ...}
        json.get("anchor_name").is_some()
            // Legacy tape-handoff returns {"output": "handoff created: ..."}
            || json
                .get("output")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.starts_with("handoff created"))
    })
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

/// Build the full agent system prompt (soul + manifest + agent.md + runtime
/// contract + skills).
///
/// Used by both the reactive agent loop and the plan executor to ensure
/// consistent agent identity across execution modes.
///
/// Returns `(prompt, has_soul)` so callers can avoid re-calling
/// `resolve_soul_prompt`.
pub(crate) fn build_agent_system_prompt(
    manifest: &AgentManifest,
    tool_registry: &crate::tool::ToolRegistry,
) -> (String, bool) {
    // 1. Soul prompt: prepend soul text if available, otherwise use manifest prompt
    //    as-is.
    let (effective_prompt, has_soul) = match resolve_soul_prompt(&manifest.name) {
        Some(soul) => (format!("{soul}\n\n---\n\n{}", manifest.system_prompt), true),
        None => (manifest.system_prompt.clone(), false),
    };
    // 2. Append external agent.md (tool knowledge, CLI guides, operational notes).
    let effective_prompt = if let Some(agent_md) = load_agent_md(&manifest.name) {
        format!("{effective_prompt}\n\n<agent_knowledge>\n{agent_md}\n</agent_knowledge>")
    } else {
        effective_prompt
    };
    // 3. Append runtime contract (tape actions, discoverable tool catalog, system
    //    paths).
    let empty = std::collections::HashSet::new();
    let deferred_catalog = tool_registry.deferred_catalog(&empty);
    let system_paths = format!(
        "\n**System Paths** (use these instead of guessing):\n- Home: {}\n- Config: {}\n- Data: \
         {}\n- Workspace: {}",
        rara_paths::home_dir().display(),
        rara_paths::config_dir().display(),
        rara_paths::data_dir().display(),
        rara_paths::workspace_dir().display(),
    );
    let effective_prompt =
        build_runtime_contract_prompt(&effective_prompt, &deferred_catalog, &system_paths);
    (effective_prompt, has_soul)
}

fn build_runtime_contract_prompt(
    base_prompt: &str,
    deferred_catalog: &[(String, String)],
    system_paths: &str,
) -> String {
    let tool_list = if deferred_catalog.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = {
            let mut catalog = deferred_catalog.to_vec();
            catalog.sort_by(|a, b| a.0.cmp(&b.0));
            catalog
                .iter()
                .map(|(name, desc)| {
                    // Truncate description to first sentence for brevity.
                    let short = desc.find(". ").map_or(desc.as_str(), |pos| &desc[..=pos]);
                    format!("  - {name}: {short}")
                })
                .collect()
        };
        format!(
            "\n**Discoverable tools** (these are keywords, NOT callable tool names — you MUST \
             call `discover-tools` first to load them):\n{}",
            entries.join("\n")
        )
    };
    format!(
        r#"{base_prompt}

<context_contract>
## Context Management

**Tape tools**: `tape-anchor` (checkpoint + trim), `tape-search` (recall old context).

**MANDATORY: On-demand tool activation**: You MUST call `discover-tools` BEFORE using any tool \
from the list below. These names are search keywords, NOT callable tools. \
Example: `discover-tools({{"query":"marketplace"}})` — this loads the real tools so you can call them. \
NEVER call a listed name directly. NEVER tell the user a tool is unavailable — \
call `discover-tools` to load it first.{tool_list}
{system_paths}

**MUST anchor when:**
- Context is long or [Context Usage Warning] appears
- Tool result exceeds ~2000 chars (anchor the key findings, not the raw output)
- User switches topic or starts a new task

**MUST search when:**
- Question refers to content before an anchor
- You need exact details from earlier in the conversation

Always include `summary` and `next_steps` in anchors — they are your future self's entry point.
</context_contract>"#
    )
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
    guard_pipeline: Arc<GuardPipeline>,
    notification_bus: NotificationBusRef,
    rara_message_id: crate::io::MessageId,
) -> crate::error::Result<AgentTurnResult> {
    // Query context via syscalls.
    let manifest =
        handle
            .session_manifest(session_key)
            .map_err(|e| KernelError::AgentExecution {
                message: format!("failed to get manifest: {e}"),
            })?;
    let full_tools = handle
        .session_tool_registry(session_key)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to get tool registry: {e}"),
        })?;

    // Filter tools by manifest allowlist, then remove excluded tools.
    let manifest_filtered = full_tools.filtered_for_manifest(&manifest.tools);
    let manifest_filtered = if manifest.excluded_tools.is_empty() {
        manifest_filtered
    } else {
        manifest_filtered.without(&manifest.excluded_tools)
    };
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

    let max_iterations = manifest
        .max_iterations
        .unwrap_or(handle.config().default_max_iterations);
    let (effective_prompt, has_soul) = build_agent_system_prompt(&manifest, tools.as_ref());
    let provider_hint = manifest.provider_hint.as_deref();

    // Resolve driver + model via the DriverRegistry syscall.
    let (driver, model) =
        handle
            .session_resolve_driver(session_key)
            .map_err(|e| KernelError::AgentExecution {
                message: format!("failed to resolve LLM driver: {e}"),
            })?;

    tracing::Span::current().record("model", model.as_str());

    let mut capabilities = ModelCapabilities::detect(provider_hint, &model);

    // Context window priority: manifest override > provider API > default.
    if let Some(t) = manifest.max_context_tokens {
        capabilities = capabilities.with_context_window(t);
    } else if let Some(api_len) = driver.model_context_length(&model).await {
        capabilities = capabilities.with_context_window(api_len);
    }
    if let Some(has_vision) = driver.model_supports_vision(&model).await {
        capabilities = capabilities.with_vision(has_vision);
    }
    tool_context.context_window_tokens = capabilities.context_window_tokens;
    // Provide the live registry (with dynamic MCP tools) so discover-tools
    // can query the full catalog at runtime, not a boot-time snapshot.
    tool_context.tool_registry = Some(tools.clone());
    let input_text = user_text.clone();
    let tool_execution_timeout = handle.config().tool_execution_timeout;
    let default_tool_timeout = handle.config().default_tool_timeout;

    // Deferred tool activation state — persists across turns within the same
    // session so the LLM does not need to re-discover tools after each message.
    let mut activated_deferred: std::collections::HashSet<crate::tool::ToolName> = handle
        .process_table()
        .with(&session_key, |s| s.activated_deferred.clone())
        .unwrap_or_default();

    // Check model tool support
    let mut tool_defs = if tools.is_empty() {
        vec![]
    } else if capabilities.supports_tools {
        tools.to_llm_tool_definitions_active(&activated_deferred)
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
    let mut cascade_asm = crate::cascade::CascadeAssembler::new(rara_message_id.to_string());
    cascade_asm.push_user(&input_text, jiff::Timestamp::now(), None);
    // Maximum number of LLM error recoveries (tools-disabled retries) allowed
    // per agent turn before the error becomes fatal.
    const MAX_LLM_ERROR_RECOVERIES: u32 = 3;
    let mut llm_error_recovery_count: u32 = 0;
    // Snapshot of tool definitions before any recovery disables them, so we
    // can restore tool access after a successful recovery iteration.
    let original_tool_defs = tool_defs.clone();
    let mut empty_response_nudged = false;
    let mut last_progress_at = Instant::now();

    let mut needs_anchor_reminder = false;
    let mut context_pressure_warning: Option<String> = None;
    let mut llm_error_recovery_message: Option<String> = None;
    // True while the current iteration is a recovery attempt (tools disabled).
    // Reset after the recovery iteration produces a successful response.
    let mut in_llm_error_recovery = false;
    let mut loop_breaker =
        loop_breaker::ToolCallLoopBreaker::new(loop_breaker::LoopBreakerConfig::builder().build());
    let mut loop_breaker_warning: Option<String> = None;

    // ── Session length reminder state ─────────────────────────────────
    // Count user turns since the last anchor to detect long sessions that
    // may benefit from a handoff.  Queried once from the tape at turn
    // start; incremented by 1 for the current user message; reset when
    // the agent creates an anchor (detected via tool names).
    let user_turns_since_anchor: usize = {
        tape.from_last_anchor(tape_name, None)
            .await
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| {
                        e.kind == crate::memory::TapEntryKind::Message
                            && e.payload.get("role").and_then(|v| v.as_str()) == Some("user")
                    })
                    .count()
            })
            .unwrap_or(0)
    };
    // +1 for the current user message that triggered this turn.
    let mut user_turns_since_anchor = user_turns_since_anchor + 1;
    let mut session_length_warned = false;
    // ── Tool call limit circuit breaker ──────────────────────────────────
    // Prevents runaway tool loops by pausing execution every N tool calls
    // and asking the user whether to continue. 0 = disabled (default).
    let limit_interval = manifest.tool_call_limit.unwrap_or(0);
    // Absolute tool call count at which the next limit fires. After each
    // continue decision this advances by `limit_interval` (additive).
    let mut next_limit_at: usize = limit_interval;
    // Monotonically increasing counter ensuring each limit event has a
    // unique ID. Prevents stale Telegram inline buttons from resolving a
    // newer limit (handle.resolve_tool_call_limit checks ID match).
    let mut limit_id_counter: u64 = 0;
    // Distinguishes "user stopped via limit" from "max iterations exhausted"
    // in the post-loop exit logic — they produce different user messages.
    let mut stopped_by_limit = false;
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
    // Set by ToolHint::SuggestFold — bypasses pressure threshold on next iteration.
    let mut force_fold_next_iteration = false;
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

                    let should_fold =
                        pressure > fold_config.fold_threshold || force_fold_next_iteration;
                    if should_fold {
                        if force_fold_next_iteration {
                            info!("auto-fold: triggered by ToolHint::SuggestFold");
                            force_fold_next_iteration = false;
                        }
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
                 MUST use tape-search to verify before answering.",
            ));
        }

        // Inject anchor reminder from previous iteration's large tool output
        if needs_anchor_reminder {
            messages.push(llm::Message::user(
                "[Large Tool Output] You just processed a large tool result that significantly \
                 bloats context. Before continuing, use tape-anchor to create a handoff with \
                 summary and next_steps. Use tape-search later for older details.",
            ));
            needs_anchor_reminder = false;
        }

        // Inject context pressure warning from previous iteration
        if let Some(warning) = context_pressure_warning.take() {
            messages.push(llm::Message::user(warning));
        }

        // Inject loop breaker warning from previous iteration
        if let Some(warning) = loop_breaker_warning.take() {
            messages.push(llm::Message::user(warning));
        }

        // Inject LLM error recovery message from previous iteration
        if let Some(recovery_msg) = llm_error_recovery_message.take() {
            messages.push(llm::Message::user(recovery_msg));
        }

        // Inject active background tasks status (first iteration only to
        // avoid repeated token cost in multi-iteration turns).
        let bg_tasks = if iteration == 0 {
            handle.background_tasks(session_key)
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

        // Inject available skills list (first iteration only — stable across
        // iterations so no need to repeat).
        if iteration == 0 {
            let skills_prompt = handle.skills_prompt();
            if !skills_prompt.is_empty() {
                messages.push(crate::llm::Message::user(format!(
                    "<system-reminder>\n{skills_prompt}</system-reminder>"
                )));
            }
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

        // Strip image content blocks when the model lacks vision support so
        // the provider does not reject the request.
        let request_messages = if capabilities.supports_vision {
            messages.clone()
        } else {
            messages.iter().map(|m| m.strip_images()).collect()
        };

        // Build completion request
        let request = llm::CompletionRequest {
            model:               model.clone(),
            messages:            request_messages,
            tools:               tool_defs.clone(),
            temperature:         Some(0.7),
            max_tokens:          Some(2048),
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
            top_p:               None,
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
        let mut repetition_guard = repetition::RepetitionGuard::new();
        let mut repetition_aborted = false;
        let mut accumulated_reasoning = String::new();
        let mut pending_tool_calls: HashMap<u32, PendingToolCall> = HashMap::new();
        let mut has_tool_calls = false;
        let mut last_usage: Option<llm::Usage> = None;
        let mut last_stop_reason: Option<llm::StopReason> = None;
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

                        // Check for LLM repetition loop.
                        if let Some(trunc_byte) = repetition_guard.feed(&text, &accumulated_text) {
                            warn!(
                                iteration,
                                total_len = accumulated_text.len(),
                                truncated_at = trunc_byte,
                                "repetition loop detected, truncating output"
                            );
                            accumulated_text.truncate(trunc_byte);
                            repetition_aborted = true;
                            stream_task.abort();
                            break;
                        }

                        // Emit AFTER repetition check: when the guard fires, the
                        // triggering delta is intentionally not forwarded. Prior
                        // deltas (including repeated text) were already streamed;
                        // the final Reply will carry the truncated version, so the
                        // Telegram adapter's prefix-slicing reconciles the mismatch.
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
                    last_stop_reason = Some(stop_reason);
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
        // When repetition_aborted is true, stream_task was intentionally
        // aborted — treat cancellation as success with valid truncated text.
        // We skip the driver_result error path entirely since the stream
        // was cut short on purpose and last_usage will be None (P1: accepted,
        // logged as warning below).
        let driver_result = match stream_task.await {
            Ok(result) => {
                if repetition_aborted {
                    // Stream completed before abort took effect; result is
                    // available but we already truncated accumulated_text.
                    None
                } else {
                    Some(result)
                }
            }
            Err(join_err) if join_err.is_cancelled() && repetition_aborted => {
                // Expected: we aborted the stream intentionally.
                warn!(
                    iteration,
                    "repetition abort: token usage unavailable for this iteration"
                );
                None
            }
            Err(join_err) if join_err.is_cancelled() => {
                return Err(KernelError::Interrupted);
            }
            Err(join_err) => {
                return Err(KernelError::AgentExecution {
                    message: format!("driver stream task panicked: {join_err}"),
                });
            }
        };

        if let Some(Err(ref e)) = driver_result {
            // Rate limit (429): immediately answer with available context —
            // do not keep retrying, the limit won't lift in time.
            if crate::error::is_rate_limit_error(e) && tool_calls_made > 0 {
                warn!(
                    iteration,
                    model = model.as_str(),
                    error = %e,
                    tool_calls_made,
                    "rate limited — answering with available context"
                );
                llm_error_recovery_message = Some(
                    "[System] You hit a rate limit. Do NOT call any more tools. Summarize the \
                     information you already have and answer the user's question now."
                        .to_string(),
                );
                tool_defs = vec![];
                in_llm_error_recovery = true;
                // Force fold to shrink context for the final answer call.
                force_fold_next_iteration = true;
                continue;
            }

            if llm_error_recovery_count < MAX_LLM_ERROR_RECOVERIES
                && crate::error::is_retryable_provider_error(e)
            {
                llm_error_recovery_count += 1;
                warn!(
                    iteration,
                    model = model.as_str(),
                    error = %e,
                    recovery_attempt = llm_error_recovery_count,
                    max_recoveries = MAX_LLM_ERROR_RECOVERIES,
                    "LLM stream error, attempting recovery without tools"
                );
                llm_error_recovery_message = Some(format!(
                    "[System] The previous request encountered a server error ({e}). Please reply \
                     to the user's question directly without using tools."
                ));
                tool_defs = vec![];
                in_llm_error_recovery = true;
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
        // driver_result is None when repetition_aborted — skip error path above.

        // After a successful LLM response, restore tool definitions if we were
        // in a recovery iteration (tools disabled).  This allows the agent to
        // resume normal tool-using behaviour on subsequent iterations instead
        // of being permanently degraded after a transient provider error.
        if in_llm_error_recovery {
            info!(
                iteration,
                recovery_count = llm_error_recovery_count,
                "LLM error recovery succeeded, restoring tool definitions"
            );
            tool_defs = original_tool_defs.clone();
            in_llm_error_recovery = false;
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

        // Compute timing metrics once, used by both tape entries and traces.
        let stream_ms = stream_start.elapsed().as_millis() as u64;
        let first_token_ms =
            first_token_at.map(|t| t.duration_since(stream_start).as_millis() as u64);

        // Persist LLM usage event to tape with performance metrics.
        if let Some(u) = last_usage {
            let event = crate::memory::LlmRunEvent {
                usage: u,
                model: model.clone(),
                stop_reason: last_stop_reason.unwrap_or(llm::StopReason::Stop),
                iteration,
                stream_ms,
                first_token_ms,
            };
            if let Err(e) = tape
                .append_event(
                    tape_name,
                    "llm.run",
                    serde_json::to_value(&event).unwrap_or_default(),
                )
                .await
            {
                warn!(error = %e, "failed to persist llm usage event");
            }
        }

        // Nudge: if the LLM stopped without producing any visible text but we
        // already executed tool calls this turn, give it one more chance to
        // respond instead of returning an empty message to the user.
        if !has_tool_calls
            && accumulated_text.is_empty()
            && tool_calls_made > 0
            && !empty_response_nudged
        {
            warn!(
                iteration,
                tool_calls_made, "LLM returned empty text after tool calls, injecting nudge"
            );
            messages.push(llm::Message::user(
                "You executed tool calls but produced no visible response. Please summarize the \
                 results for the user."
                    .to_string(),
            ));
            empty_response_nudged = true;
            continue;
        }

        // Empty stream detection: when the LLM returned no text, no tool
        // calls, AND no usage info, the provider likely rejected the request
        // silently (e.g. context window exceeded on free-tier models).  Treat
        // this as a retryable error: trigger an auto-fold to compress context
        // and retry with tools disabled.
        if !has_tool_calls
            && accumulated_text.is_empty()
            && last_usage.is_none()
            && llm_error_recovery_count < MAX_LLM_ERROR_RECOVERIES
        {
            llm_error_recovery_count += 1;
            warn!(
                iteration,
                model = model.as_str(),
                recovery_attempt = llm_error_recovery_count,
                max_recoveries = MAX_LLM_ERROR_RECOVERIES,
                "LLM stream returned empty (no text, no tools, no usage) — likely context window \
                 exceeded, attempting fold + recovery"
            );

            // Force an auto-fold on the next iteration to compress context
            // before retrying.  The fold runs at the top of the loop (line
            // ~1110) when force_fold_next_iteration is set.
            force_fold_next_iteration = true;

            llm_error_recovery_message = Some(
                "[System] The previous request produced an empty response (possible context \
                 window limit). Context has been compressed. Please reply to the user's question \
                 directly without using tools."
                    .to_string(),
            );
            tool_defs = vec![];
            in_llm_error_recovery = true;
            continue;
        }

        // Terminal response: exit when the LLM produced no tool calls.
        // Recovery iterations always land here because tools were disabled,
        // but subsequent iterations (after tool restoration) can resume
        // normal tool-calling flow.
        if !has_tool_calls {
            // Persist final assistant message to tape.
            let meta = serde_json::to_value(crate::memory::LlmEntryMetadata {
                rara_message_id: rara_message_id.to_string(),
                usage: last_usage,
                model: model.clone(),
                iteration,
                stream_ms,
                first_token_ms,
                reasoning_content: if accumulated_reasoning.is_empty() {
                    None
                } else {
                    Some(accumulated_reasoning.clone())
                },
            })
            .ok();
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

            cascade_asm.push_assistant(
                &accumulated_text,
                if accumulated_reasoning.is_empty() {
                    None
                } else {
                    Some(&accumulated_reasoning)
                },
                jiff::Timestamp::now(),
                None,
            );

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

            let cascade = cascade_asm.finish();
            let _ = tape
                .append_event(
                    tape_name,
                    "cascade.trace",
                    serde_json::to_value(&cascade).unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "failed to serialize cascade trace");
                        serde_json::Value::Null
                    }),
                )
                .await;

            return Ok(AgentTurnResult {
                text: accumulated_text,
                iterations: iteration + 1,
                tool_calls: tool_calls_made,
                model: model.clone(),
                trace,
                cascade,
            });
        }

        // Stash for partial-result reporting
        last_accumulated_text = accumulated_text.clone();

        // Emit turn-level rationale — the LLM's reasoning for the upcoming
        // tool calls.  Prefer extended-thinking (accumulated_reasoning) over
        // regular content (accumulated_text) since the former is the model's
        // internal chain-of-thought.
        if has_tool_calls {
            let rationale_source = if accumulated_reasoning.trim().is_empty() {
                &accumulated_text
            } else {
                &accumulated_reasoning
            };
            let trimmed = rationale_source.trim();
            if !trimmed.is_empty() {
                stream_handle.emit(StreamEvent::TurnRationale {
                    text: trimmed.to_owned(),
                });
            }
        }

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
            // Pre-parse arguments for display purposes (summary extraction);
            // fall back to empty object if the buffer is malformed.
            let start_args = serde_json::from_str(&tool_call.arguments_buf)
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
            stream_handle.emit(StreamEvent::ToolCallStart {
                name:      tool_call.name.clone(),
                id:        tool_call.id.clone(),
                arguments: start_args,
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
                            serde_json::to_value(crate::memory::ToolResultMetadata {
                                rara_message_id: rara_message_id.to_string(),
                                tool_metrics:    vec![crate::memory::ToolMetric {
                                    name:        tool_call.name.clone(),
                                    duration_ms: 0,
                                    success:     false,
                                    error:       Some(error_message.clone()),
                                }],
                            })
                            .ok(),
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

        // Persist intermediate assistant message to tape so that
        // `build_cascade` can detect tick boundaries between iterations.
        // Without this, the cascade trace always shows a single tick.
        {
            let mut meta = crate::memory::LlmEntryMetadata {
                rara_message_id: rara_message_id.to_string(),
                usage: last_usage,
                model: model.clone(),
                iteration,
                stream_ms,
                first_token_ms,
                reasoning_content: None,
            };
            if !accumulated_reasoning.is_empty() {
                meta.reasoning_content = Some(accumulated_reasoning.clone());
            }
            let _ = tape
                .append_message(
                    tape_name,
                    serde_json::json!({
                        "role": "assistant",
                        "content": &accumulated_text,
                    }),
                    serde_json::to_value(&meta).ok(),
                )
                .await;
        }

        cascade_asm.push_assistant(
            &accumulated_text,
            if accumulated_reasoning.is_empty() {
                None
            } else {
                Some(&accumulated_reasoning)
            },
            jiff::Timestamp::now(),
            None,
        );

        // Persist tool calls to tape.
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
            let tool_call_meta = serde_json::to_value(crate::memory::LlmEntryMetadata {
                rara_message_id: rara_message_id.to_string(),
                usage: last_usage,
                model: model.clone(),
                iteration,
                stream_ms,
                first_token_ms,
                reasoning_content: None,
            })
            .ok();
            let _ = tape
                .append_tool_call(
                    tape_name,
                    serde_json::json!({ "calls": calls_json }),
                    tool_call_meta,
                )
                .await;
        }

        {
            let calls_for_cascade: Vec<(&str, &str)> = assistant_tool_calls
                .iter()
                .map(|tc| (tc.name.as_str(), tc.arguments.as_str()))
                .collect();
            cascade_asm.push_tool_calls(&calls_for_cascade, jiff::Timestamp::now(), None);
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

        // Execute all tool calls concurrently via FuturesUnordered so we
        // can harvest partial results if the global wave timeout fires.
        let tool_futures: Vec<_> = valid_tool_calls
            .iter()
            .map(|(id, name, args)| {
                let tool = tools.get(name);
                let args = args.clone();
                let name = name.clone();
                let mut tc = tool_context.clone();
                tc.stream_handle = Some(stream_handle.clone());
                tc.tool_call_id = Some(id.clone());
                let user_ref = runtime_user.clone();
                let tool_cancel = turn_cancel.clone();
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

                        let risk_level = crate::security::ApprovalManager::classify_risk(blocked_tool.as_str());
                        let approval_req = crate::security::ApprovalRequest {
                            id:           uuid::Uuid::new_v4(),
                            session_key:  session_key_for_guard,
                            tool_name:    blocked_tool.to_string(),
                            tool_args:    args.clone(),
                            summary:      format!("Guard blocked ({layer}): {reason}"),
                            risk_level,
                            requested_at: jiff::Timestamp::now(),
                            timeout_secs: 120,
                            context:      None,
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
                                // Remember approved path so the user is not
                                // prompted again for the same directory tree.
                                if layer == GuardLayer::PathScope {
                                    guard_pipeline.approve_path_scope(&name, &args);
                                }
                                // Fall through to normal tool execution.
                            }
                            _ => {
                                // Denied or timed out — block the tool call.
                                tool_span.record("success", false);

                                let agent_id = session_key_for_guard.into_inner();
                                notification_bus
                                    .publish(KernelNotification::GuardDenied {
                                        agent_id,
                                        tool_name: blocked_tool.to_string(),
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
                        let per_tool_timeout = tool.execution_timeout().unwrap_or(default_tool_timeout);

                        // Semantic validation runs after the security guard
                        // and before execute. This is the place to surface
                        // cross-field invariants ("old != new") or refuse
                        // no-op edits without spending the execute budget.
                        if let Err(e) = tool.validate(&args).await {
                            tool_span.record("success", false);
                            warn!(tool = %name, args = %args_snapshot, error = %e, "tool validation failed");
                            let dur = tool_start.elapsed().as_millis() as u64;
                            return (
                                false,
                                crate::tool::ToolOutput::from(
                                    serde_json::json!({ "error": e.to_string() }),
                                ),
                                Some(e.to_string()),
                                dur,
                            );
                        }

                        let tool_result = tokio::select! {
                            result = tokio::time::timeout(per_tool_timeout, tool.execute(args, &tc)) => {
                                match result {
                                    Ok(inner) => inner,
                                    Err(_elapsed) => {
                                        warn!(tool = %name, timeout_secs = per_tool_timeout.as_secs(), "per-tool timeout exceeded");
                                        Err(anyhow::anyhow!(
                                            "tool execution timed out after {}s",
                                            per_tool_timeout.as_secs()
                                        ))
                                    }
                                }
                            }
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

        // Use FuturesUnordered so we can harvest partial results if the
        // global wave timeout fires (completed tools keep their real results).
        use futures::stream::{FuturesUnordered, StreamExt};
        let num_tools = tool_futures.len();
        let mut futs: FuturesUnordered<_> = tool_futures
            .into_iter()
            .enumerate()
            .map(|(idx, fut)| async move { (idx, fut.await) })
            .collect();

        let mut indexed_results: Vec<Option<(bool, crate::tool::ToolOutput, Option<String>, u64)>> =
            (0..num_tools).map(|_| None).collect();
        let deadline = tokio::time::sleep(tool_execution_timeout);
        tokio::pin!(deadline);

        let timed_out = loop {
            tokio::select! {
                item = futs.next() => {
                    match item {
                        Some((idx, result)) => { indexed_results[idx] = Some(result); }
                        None => break false, // all futures completed
                    }
                }
                _ = &mut deadline => {
                    warn!("global tool wave timeout exceeded after {}s", tool_execution_timeout.as_secs());
                    break true;
                }
                _ = turn_cancel.cancelled() => {
                    return Err(KernelError::Interrupted);
                }
            }
        };

        // Fill missing slots with synthetic timeout errors.
        let results: Vec<_> = indexed_results
            .into_iter()
            .map(|slot| {
                slot.unwrap_or_else(|| {
                    let msg = if timed_out {
                        format!(
                            "tool wave timed out after {}s",
                            tool_execution_timeout.as_secs()
                        )
                    } else {
                        "tool task failed".to_string()
                    };
                    (
                        false,
                        crate::tool::ToolOutput::from(serde_json::json!({
                            "status": "timeout",
                            "error": &msg,
                        })),
                        Some(msg),
                        tool_execution_timeout.as_millis() as u64,
                    )
                })
            })
            .collect();

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
            let tool_metrics: Vec<crate::memory::ToolMetric> = results
                .iter()
                .zip(valid_tool_calls.iter())
                .map(|((success, _, err, duration_ms), (_id, name, _args))| {
                    crate::memory::ToolMetric {
                        name:        name.clone(),
                        duration_ms: *duration_ms,
                        success:     *success,
                        error:       err.clone(),
                    }
                })
                .collect();
            let _ = tape
                .append_tool_result(
                    tape_name,
                    serde_json::json!({ "results": results_json.clone() }),
                    serde_json::to_value(crate::memory::ToolResultMetadata {
                        rara_message_id: rara_message_id.to_string(),
                        tool_metrics,
                    })
                    .ok(),
                )
                .await;
            {
                let results_strs: Vec<String> = results_json
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect();
                let results_refs: Vec<&str> = results_strs.iter().map(|s| s.as_str()).collect();
                cascade_asm.push_tool_results(&results_refs, jiff::Timestamp::now(), None);
            }
            if should_remind_tape_anchor(&tool_names, &results_json) {
                needs_anchor_reminder = true;
            }
            // If discover-tools was called, extract activated tool names and
            // regenerate tool_defs so newly activated tools are available in
            // the next iteration.
            if tool_names.iter().any(|n| n == "discover-tools") {
                for (name, result) in tool_names.iter().zip(results_json.iter()) {
                    if name == "discover-tools" {
                        if let Ok(parsed) = serde_json::from_value::<crate::tool::DiscoverToolsResult>(
                            result.clone(),
                        ) {
                            for entry in &parsed.tools {
                                activated_deferred.insert(crate::tool::ToolName::new(&entry.name));
                            }
                        }
                    }
                }
                tool_defs = tools.to_llm_tool_definitions_active(&activated_deferred);
                // Persist to session so activations survive across turns.
                let snapshot = activated_deferred.clone();
                handle.process_table().with_mut(&session_key, |s| {
                    s.activated_deferred = snapshot;
                });
            }
            // Check tool output hints for SuggestFold.
            for ((success, output, _err, _dur), (_id, name, _args)) in
                results.iter().zip(valid_tool_calls.iter())
            {
                if !success {
                    continue;
                }
                // Primary: read hints from ToolOutput (tools that bypass ToolDef
                // or manually construct ToolOutput can set hints directly).
                let has_suggest_fold = output
                    .hints
                    .iter()
                    .any(|h| matches!(h, crate::tool::ToolHint::SuggestFold { .. }));
                // Fallback: the ToolDef macro calls from_serialize() which always
                // returns empty hints, so known heavy-context tools are matched
                // by name until the macro supports hint propagation.
                let is_known_heavy = name == "marketplace-install";
                if has_suggest_fold || is_known_heavy {
                    force_fold_next_iteration = true;
                    info!(
                        tool = %name,
                        via_hint = has_suggest_fold,
                        "setting force_fold_next_iteration"
                    );
                }
            }

            // Reset session-length counter when the agent creates an anchor.
            // Detected via result payload rather than hardcoded tool names so
            // that new anchor-creating tools are automatically covered.
            if did_create_anchor(&results_json) {
                user_turns_since_anchor = 0;
                session_length_warned = false;
            }

            // Record tool calls for loop detection.
            for (_id, name, args) in &valid_tool_calls {
                loop_breaker.record(name, &args.to_string());
            }
            let intervention = loop_breaker.check();
            match intervention {
                loop_breaker::LoopIntervention::None => {}
                loop_breaker::LoopIntervention::Warn { pattern, message } => {
                    warn!(
                        tool_calls_made,
                        pattern,
                        %message,
                        "loop breaker: injecting strategy-change warning"
                    );
                    stream_handle.emit(StreamEvent::LoopBreakerTriggered {
                        tools: vec![],
                        pattern: pattern.to_owned(),
                        tool_calls_made,
                    });
                    loop_breaker_warning = Some(message);
                }
                loop_breaker::LoopIntervention::DisableTools {
                    pattern,
                    tools,
                    message,
                } => {
                    warn!(
                        tool_calls_made,
                        pattern,
                        ?tools,
                        %message,
                        "loop breaker: disabling tools and injecting warning"
                    );
                    stream_handle.emit(StreamEvent::LoopBreakerTriggered {
                        tools: tools.clone(),
                        pattern: pattern.to_owned(),
                        tool_calls_made,
                    });
                    tool_defs.retain(|td| !tools.contains(&td.name));
                    loop_breaker_warning = Some(message);
                }
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

            crate::metrics::record_tool_duration(&manifest.name, name, *duration_ms);

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

        // ── Tool call limit circuit breaker ──────────────────────────────
        // When cumulative tool calls reach the ceiling, pause execution and
        // wait for the user to decide whether to continue or stop.
        //
        // Flow:
        //   1. Emit ToolCallLimit → adapter shows inline buttons to the user.
        //   2. Register a oneshot channel on the session (keyed by limit_id).
        //   3. Await the oneshot with a 120s hard timeout.
        //   4a. Continue → advance next_limit_at by limit_interval.
        //   4b. Stop / Timeout / channel closed → set stopped_by_limit, break.
        //
        // The 120s timeout prevents the agent loop from hanging indefinitely
        // if the user walks away or the adapter lacks decision UI.
        if limit_interval > 0 && tool_calls_made >= next_limit_at {
            limit_id_counter += 1;
            let current_limit_id = limit_id_counter;
            let elapsed_secs = turn_start.elapsed().as_secs();
            stream_handle.emit(StreamEvent::ToolCallLimit {
                session_key: session_key.to_string(),
                limit_id: current_limit_id,
                tool_calls_made,
                elapsed_secs,
            });

            // Register the oneshot sender on the session so that the adapter
            // (e.g. Telegram callback handler) can deliver the user's decision.
            // Uses the same oneshot pattern as the guard approval system.
            let (tx, rx) = tokio::sync::oneshot::channel();
            handle.register_tool_call_limit(session_key, current_limit_id, tx);

            info!(
                tool_calls_made,
                next_limit_at,
                limit_id = current_limit_id,
                elapsed_secs,
                "agent loop paused at tool call limit, awaiting user decision"
            );

            // 120s hard timeout — treats expiry the same as an explicit Stop.
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
                Ok(Ok(crate::io::ToolCallLimitDecision::Continue)) => {
                    info!(tool_calls_made, "user chose to continue agent loop");
                    // Additive: next limit fires after another full interval.
                    next_limit_at = tool_calls_made + limit_interval;
                    stream_handle.emit(StreamEvent::ToolCallLimitResolved {
                        session_key: session_key.to_string(),
                        limit_id:    current_limit_id,
                        continued:   true,
                    });
                }
                _ => {
                    // Explicit Stop, 120s timeout, or oneshot dropped — all
                    // treated as a graceful stop. NOT max-iteration exhaustion.
                    warn!(tool_calls_made, "agent loop stopped by user or timeout");
                    stream_handle.emit(StreamEvent::ToolCallLimitResolved {
                        session_key: session_key.to_string(),
                        limit_id:    current_limit_id,
                        continued:   false,
                    });
                    stopped_by_limit = true;
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
                         window capacity {} tokens. You MUST immediately call tape-anchor with \
                         summary and next_steps.",
                        tape_info.estimated_context_tokens,
                        usage_ratio * 100.0,
                        capabilities.context_window_tokens,
                    ));
                }
                ContextPressure::Warning { usage_ratio, .. } => {
                    context_pressure_warning = Some(format!(
                        "[Context Usage Warning] Current context ~{} tokens ({:.0}%), context \
                         window capacity {} tokens. You SHOULD consider calling tape-anchor.",
                        tape_info.estimated_context_tokens,
                        usage_ratio * 100.0,
                        capabilities.context_window_tokens,
                    ));
                }
                ContextPressure::Normal => {}
            }
        }

        // ── Session length reminder ──────────────────────────────────
        // Inject a warning when the session has many user turns without an
        // anchor, independent of token-based context pressure.  Uses an
        // in-memory counter instead of querying tape each iteration.
        if !session_length_warned
            && context_pressure_warning.is_none()
            && user_turns_since_anchor >= TURN_REMINDER_THRESHOLD
        {
            context_pressure_warning = Some(format!(
                "[Session Length Warning] This session has had {user_turns_since_anchor} user \
                 turns since the last anchor. If the topic has shifted, you MUST call tape-anchor \
                 now with summary and next_steps.",
            ));
            session_length_warned = true;
        }

        // Emit a progress event on silent (tool-only, no text) iterations so
        // the user sees the agent is still working. Throttled to at most once
        // every 5 seconds to avoid flooding the UI.
        if accumulated_text.len() == last_accumulated_text.len()
            && last_progress_at.elapsed() >= std::time::Duration::from_secs(5)
        {
            stream_handle.emit(StreamEvent::Progress {
                stage: format!("Processing... ({tool_calls_made} steps completed)"),
            });
            last_progress_at = Instant::now();
        }
    }

    // Determine exit reason and build appropriate error/message.
    let exhaustion_error = if stopped_by_limit {
        // User clicked "stop" or tool call limit timed out — not an exhaustion error.
        let msg = format!("agent stopped by user/timeout after {tool_calls_made} tool calls");
        warn!(
            tool_calls_made,
            "agent loop stopped by tool call limit decision"
        );
        stream_handle.emit(StreamEvent::Progress {
            stage: format!("[已停止] 已执行 {tool_calls_made} 次工具调用。"),
        });
        if last_accumulated_text.is_empty() {
            last_accumulated_text = format!("[已停止，已执行 {tool_calls_made} 次工具调用。]");
        }
        msg
    } else {
        // Max iterations exhausted — return partial results with failure markers
        warn!(
            max_iterations,
            tool_calls_made,
            "inline agent loop hit max iterations limit, returning partial results"
        );
        let msg = format!(
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
        msg
    };
    let actual_iterations = iteration_traces.len();
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

    let cascade = cascade_asm.finish();
    let _ = tape
        .append_event(
            tape_name,
            "cascade.trace",
            serde_json::to_value(&cascade).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "failed to serialize cascade trace");
                serde_json::Value::Null
            }),
        )
        .await;

    Ok(AgentTurnResult {
        text: last_accumulated_text,
        iterations: actual_iterations,
        tool_calls: tool_calls_made,
        model: model.clone(),
        trace,
        cascade,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ContextPressure, build_runtime_contract_prompt, classify_context_pressure,
        did_create_anchor, resolve_soul_prompt, should_remind_tape_anchor,
        should_remind_tape_search,
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
    fn runtime_contract_prompt_includes_tape_and_discover_tools() {
        let prompt = build_runtime_contract_prompt("base", &[], "");
        assert!(prompt.contains("<context_contract>"));
        assert!(prompt.contains("`tape-anchor` (checkpoint + trim)"));
        assert!(prompt.contains("`tape-search` (recall old context)"));
        assert!(prompt.contains("`discover-tools`"));
        assert!(prompt.contains("exact details from earlier"));
        assert!(prompt.contains("`summary` and `next_steps` in anchors"));
    }

    #[test]
    fn runtime_contract_lists_deferred_tool_catalog() {
        let catalog = vec![
            (
                "http-fetch".to_string(),
                "Fetch HTTP resources.".to_string(),
            ),
            (
                "system-paths".to_string(),
                "Show system paths. Extra detail here.".to_string(),
            ),
        ];
        let prompt = build_runtime_contract_prompt("base", &catalog, "");
        assert!(prompt.contains("http-fetch: Fetch HTTP resources."));
        assert!(prompt.contains("system-paths: Show system paths."));
        assert!(prompt.contains("Discoverable tools"));
        assert!(prompt.contains("NOT callable tool names"));
        assert!(!prompt.contains("`http-fetch`"));
    }

    #[test]
    fn runtime_contract_omits_discoverable_section_when_no_deferred_tools() {
        let prompt = build_runtime_contract_prompt("base", &[], "");
        assert!(!prompt.contains("Discoverable tools"));
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

    #[test]
    fn runtime_contract_includes_topic_switch_in_must_anchor() {
        let prompt = build_runtime_contract_prompt("base", &[], "");
        assert!(prompt.contains("switches topic"));
    }

    #[test]
    fn runtime_contract_includes_system_paths() {
        let paths = "\n**System Paths** (use these instead of guessing):\n- Home: /test/home\n- \
                     Config: /test/config\n- Data: /test/data\n- Workspace: /test/workspace";
        let prompt = build_runtime_contract_prompt("base", &[], paths);
        assert!(prompt.contains("**System Paths**"));
        assert!(prompt.contains("Home: /test/home"));
        assert!(prompt.contains("Config: /test/config"));
        assert!(prompt.contains("Data: /test/data"));
        assert!(prompt.contains("Workspace: /test/workspace"));
    }

    #[test]
    fn did_create_anchor_detects_tape_anchor() {
        let results = vec![json!({"anchor_name": "topic/foo", "entries_after_anchor": 5})];
        assert!(did_create_anchor(&results));
    }

    #[test]
    fn did_create_anchor_detects_tape_handoff() {
        let results = vec![json!({"output": "handoff created: my-handoff"})];
        assert!(did_create_anchor(&results));
    }

    #[test]
    fn did_create_anchor_ignores_unrelated_tools() {
        let results = vec![json!({"output": "search results: 3 found"})];
        assert!(!did_create_anchor(&results));
    }
}
