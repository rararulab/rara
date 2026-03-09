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
    handle::KernelHandle,
    identity::Role,
    io::{StreamEvent, StreamHandle},
    llm,
    llm::ModelCapabilities,
    session::SessionKey,
};

/// Estimated chars-per-token ratio for context size estimation.
const CHARS_PER_TOKEN: usize = 4;
/// Context usage threshold (fraction) at which a SHOULD-handoff hint is injected.
const CONTEXT_WARN_THRESHOLD: f64 = 0.70;
/// Context usage threshold (fraction) at which a MUST-handoff hint is injected.
const CONTEXT_CRITICAL_THRESHOLD: f64 = 0.85;

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

/// Agent "binary" — static definition, loadable from YAML or constructed
/// dynamically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Unique name identifying this agent definition.
    pub name:               String,
    /// Agent's functional role (chat, scout, planner, worker).
    #[serde(default)]
    pub role:               AgentRole,
    /// Human-readable description.
    pub description:        String,
    /// LLM model identifier.
    #[serde(default)]
    pub model:              Option<String>,
    /// System prompt defining agent behavior.
    pub system_prompt:      String,
    /// Optional personality/mood/voice prompt.
    #[serde(default)]
    pub soul_prompt:        Option<String>,
    /// Optional hint for provider selection.
    #[serde(default)]
    pub provider_hint:      Option<String>,
    /// Maximum LLM iterations before forced completion.
    #[serde(default)]
    pub max_iterations:     Option<usize>,
    /// Tool names this agent is allowed to use (empty = inherit parent's
    /// tools).
    #[serde(default)]
    pub tools:              Vec<String>,
    /// Maximum number of concurrent child agents this agent can spawn.
    #[serde(default)]
    pub max_children:       Option<usize>,
    /// Maximum context window size in tokens.
    #[serde(default)]
    pub max_context_tokens: Option<usize>,
    /// Dispatch priority for scheduling.
    #[serde(default)]
    pub priority:           Priority,
    /// Arbitrary metadata for extension.
    #[serde(default)]
    pub metadata:           serde_json::Value,
    /// Optional sandbox configuration for file access control.
    #[serde(default)]
    pub sandbox:            Option<SandboxConfig>,
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

/// Execute a single agent turn inline: build messages, stream LLM responses,
/// execute tool calls, and emit [`StreamEvent`]s directly.
///
/// Uses the new [`LlmDriver`] abstraction with first-class `reasoning_content`
/// (thinking tokens) support. The driver sends [`StreamDelta`] events through
/// an `mpsc` channel, which this function consumes.
///
/// # Cancellation
///
/// Respects `turn_cancel` at every `tokio::select!` point — both before the
/// stream starts and during delta consumption.
#[tracing::instrument(
    skip(handle, history, stream_handle, turn_cancel, tape, tape_name),
    fields(
        session_key = %session_key,
    )
)]
pub(crate) async fn run_agent_loop(
    handle: &KernelHandle,
    session_key: SessionKey,
    user_text: String,
    history: Option<Vec<llm::Message>>,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
    tape: crate::memory::TapeService,
    tape_name: &str,
    tool_context: crate::tool::ToolContext,
    milestone_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
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

    // Filter tools by user permissions — users can only see tools they are
    // authorized to use.  This prevents the LLM from even attempting to call
    // tools the user lacks permission for.
    let tools = if let Some(ref user_id) = tool_context.user_id {
        match handle.security().user_store().get_by_name(user_id).await {
            Ok(Some(user)) => {
                let filtered = manifest_filtered.filtered_by_user(&user);
                if filtered.len() < manifest_filtered.len() {
                    let denied: Vec<String> = manifest_filtered
                        .iter()
                        .filter(|(name, _)| !user.can_use_tool(name))
                        .map(|(name, _)| name.to_string())
                        .collect();
                    info!(user_id, ?denied, "filtered tools by user permissions");
                }
                Arc::new(filtered)
            }
            _ => Arc::new(manifest_filtered),
        }
    } else {
        Arc::new(manifest_filtered)
    };

    let max_iterations = manifest.max_iterations.unwrap_or(25);
    let effective_prompt = match &manifest.soul_prompt {
        Some(soul) => format!("{soul}\n\n---\n\n{}", manifest.system_prompt),
        None => manifest.system_prompt.clone(),
    };
    let effective_prompt = format!(
        "{effective_prompt}\n\n\
         <context_contract>\n\
         You have access to `tape-handoff` — a tool that creates a checkpoint and truncates conversation history.\n\
         \n\
         ## When you MUST use tape-handoff:\n\
         - Before your context becomes too long to complete the task\n\
         - After receiving a very large tool result (>2000 chars of output)\n\
         - When performing iterative tasks (screenshots, OCR, web scraping, file listing) that accumulate large outputs\n\
         - When the system injects a [Context Usage Warning]\n\
         \n\
         ## When you SHOULD use tape-handoff:\n\
         - After completing a logical phase of work (discovery → implementation → verification)\n\
         - When switching between unrelated subtasks\n\
         - After processing multiple tool results in sequence\n\
         \n\
         ## How to use it effectively:\n\
         1. Always provide a detailed `summary` of what happened so far\n\
         2. Always provide `next_steps` with concrete actionable items\n\
         3. A good handoff preserves your progress — a missing summary means lost context\n\
         \n\
         Failing to handoff when needed will cause context window overflow and task failure.\n\
         </context_contract>"
    );
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
    let input_text = user_text.clone();

    // Build initial messages: system + optional history + user
    let mut messages: Vec<llm::Message> = {
        let mut msgs = vec![llm::Message::system(&effective_prompt)];
        if let Some(hist) = history {
            msgs.extend(hist);
        }
        msgs.push(llm::Message::user(user_text));
        msgs
    };

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

    for iteration in 0..max_iterations {
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
        };

        // Start streaming via LlmDriver
        let (tx, mut rx) = mpsc::channel::<llm::StreamDelta>(128);
        let driver_clone = Arc::clone(&driver);
        let request_clone = request;

        // Spawn driver.stream() — it sends deltas to tx and returns when done.
        let stream_task = tokio::spawn(async move { driver_clone.stream(request_clone, tx).await });

        // Consume streaming deltas
        let stream_start = Instant::now();
        let mut first_token_at: Option<Instant> = None;
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut pending_tool_calls: HashMap<u32, PendingToolCall> = HashMap::new();
        let mut has_tool_calls = false;

        loop {
            let delta = tokio::select! {
                delta = rx.recv() => delta,
                _ = turn_cancel.cancelled() => {
                    stream_task.abort();
                    info!("LLM turn cancelled during streaming");
                    return Err(KernelError::AgentExecution {
                        message: "interrupted by user".into(),
                    });
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
                        accumulated_text.push_str(&text);
                        stream_handle.emit(StreamEvent::TextDelta { text });
                    }
                }
                llm::StreamDelta::ReasoningDelta { text } => {
                    if !text.is_empty() {
                        if first_token_at.is_none() {
                            first_token_at = Some(Instant::now());
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
                    break;
                }
            }
        }

        // Wait for the stream task to complete (the driver accumulates the
        // full response internally).
        let driver_result = match stream_task.await {
            Ok(result) => result,
            Err(join_err) if join_err.is_cancelled() => {
                return Err(KernelError::AgentExecution {
                    message: "interrupted by user".into(),
                });
            }
            Err(join_err) => {
                return Err(KernelError::AgentExecution {
                    message: format!("driver stream task panicked: {join_err}"),
                });
            }
        };

        if let Err(ref e) = driver_result {
            // Auto-handoff on context window overflow — truncate and retry.
            if !context_window_recovery_used && matches!(e, KernelError::ContextWindow) {
                warn!(
                    iteration,
                    model = model.as_str(),
                    "context window exceeded, performing auto-handoff and retry"
                );
                context_window_recovery_used = true;

                // Create an automatic handoff anchor to truncate context.
                let state = crate::memory::HandoffState {
                    phase:      None,
                    summary:    Some(
                        "Context window exceeded — automatic handoff to truncate history."
                            .to_owned(),
                    ),
                    next_steps: None,
                    source_ids: vec![],
                    owner:      Some("system".to_owned()),
                    extra:      None,
                };
                if let Err(he) = tape.handoff(tape_name, "auto-compact", state).await {
                    warn!(error = %he, "auto-handoff failed, cannot recover from context window error");
                    return Err(KernelError::AgentExecution {
                        message: format!("Context window exceeded and auto-handoff failed: {he}"),
                    });
                }

                // Rebuild messages from the truncated context.
                let rebuilt = tape.build_llm_context(tape_name).await.map_err(|e| {
                    KernelError::AgentExecution {
                        message: format!("failed to rebuild context after handoff: {e}"),
                    }
                })?;
                messages = rebuilt;
                // Re-add the user's current message that triggered this turn.
                messages.push(llm::Message::user(&input_text));
                continue;
            }

            if !llm_error_recovery_used && crate::error::is_retryable_provider_error(e) {
                warn!(
                    iteration,
                    model = model.as_str(),
                    error = %e,
                    "LLM stream error, attempting recovery without tools"
                );
                llm_error_recovery_used = true;
                messages.push(llm::Message::user(format!(
                    "[系统提示] 上一次请求遇到了服务端错误（{e}），请直接回复用户的问题，\
                     不要使用工具。"
                )));
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

        // Terminal response (no tool calls, or recovery iteration must exit)
        if !has_tool_calls || llm_error_recovery_used {
            // Persist final assistant message to tape.
            let _ = tape
                .append_message(
                    tape_name,
                    serde_json::json!({
                        "role": "assistant",
                        "content": &accumulated_text,
                    }),
                    None,
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
                duration_ms:      turn_start.elapsed().as_millis() as u64,
                model:            model.clone(),
                input_text:       Some(input_text.clone()),
                iterations:       iteration_traces,
                final_text_len:   accumulated_text.len(),
                total_tool_calls: tool_calls_made,
                success:          true,
                error:            None,
            };
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
            .collect();

        // Parse and validate tool calls
        let mut valid_tool_calls = Vec::new();
        let mut assistant_tool_calls = Vec::new();
        for tool_call in tool_call_list {
            tool_calls_made += 1;
            let args = match parse_tool_call_arguments(&tool_call.arguments_buf) {
                Ok(args) => args,
                Err(error_message) => {
                    messages.push(llm::Message::tool_result(
                        &tool_call.id,
                        serde_json::json!({ "error": error_message }).to_string(),
                    ));
                    continue;
                }
            };

            assistant_tool_calls.push(llm::ToolCallRequest {
                id:        tool_call.id.clone(),
                name:      tool_call.name.clone(),
                arguments: tool_call.arguments_buf.clone(),
            });

            stream_handle.emit(StreamEvent::ToolCallStart {
                name:      tool_call.name.clone(),
                id:        tool_call.id.clone(),
                arguments: args.clone(),
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

        if assistant_tool_calls.is_empty() {
            messages.push(llm::Message::assistant(accumulated_text.clone()));
        } else {
            messages.push(llm::Message::assistant_with_tool_calls(
                accumulated_text.clone(),
                assistant_tool_calls.clone(),
            ));
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
            let _ = tape
                .append_tool_call(tape_name, serde_json::json!({ "calls": calls_json }), None)
                .await;
        }

        iter_span.record("tool_count", valid_tool_calls.len());

        // Resolve user for runtime permission guard (defense in depth).
        let runtime_user = if let Some(ref uid) = tool_context.user_id {
            handle
                .security()
                .user_store()
                .get_by_name(uid)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        // Execute all tool calls concurrently (with timing for traces)
        let tool_futures: Vec<_> = valid_tool_calls
            .iter()
            .map(|(_id, name, args)| {
                let tool = tools.get(name);
                let args = args.clone();
                let name = name.clone();
                let tc = tool_context.clone();
                let user_ref = runtime_user.clone();
                let tool_span = info_span!(
                    "tool_exec",
                    tool_name = name.as_str(),
                    success = tracing::field::Empty,
                );
                async move {
                    let _guard = tool_span.enter();
                    let tool_start = Instant::now();

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
                            return (false, serde_json::json!({ "error": &err }), Some(err), dur);
                        }
                    }

                    if let Some(tool) = tool {
                        match tool.execute(args, &tc).await {
                            Ok(result) => {
                                tool_span.record("success", true);
                                let dur = tool_start.elapsed().as_millis() as u64;
                                (true, result, None::<String>, dur)
                            }
                            Err(e) => {
                                tool_span.record("success", false);
                                let dur = tool_start.elapsed().as_millis() as u64;
                                (
                                    false,
                                    serde_json::json!({ "error": e.to_string() }),
                                    Some(e.to_string()),
                                    dur,
                                )
                            }
                        }
                    } else {
                        tool_span.record("success", false);
                        let err = format!("tool not found: {name}");
                        let dur = tool_start.elapsed().as_millis() as u64;
                        (false, serde_json::json!({ "error": &err }), Some(err), dur)
                    }
                }
            })
            .collect();

        let results = futures::future::join_all(tool_futures).await;

        // Persist tool results to tape.
        if !results.is_empty() {
            let results_json: Vec<serde_json::Value> = results
                .iter()
                .map(|(_success, result, _err, _dur)| result.clone())
                .collect();
            let _ = tape
                .append_tool_result(
                    tape_name,
                    serde_json::json!({ "results": results_json }),
                    None,
                )
                .await;
        }

        // Build tool call traces from results
        let mut tool_call_traces: Vec<ToolCallTrace> = Vec::with_capacity(results.len());

        // Emit ToolCallEnd events and append tool response messages
        for ((id, name, args), (success, result, err, duration_ms)) in
            valid_tool_calls.iter().zip(results.iter())
        {
            let result_str = result.to_string();
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

            messages.push(llm::Message::tool_result(id, result_str));
        }

        // ── Runtime context guard ──────────────────────────────────────
        // Estimate current context size and inject a warning if it's
        // approaching the model's context window limit.
        {
            let estimated_chars: usize =
                messages.iter().map(|m| m.estimated_char_len()).sum();
            let estimated_tokens = estimated_chars / CHARS_PER_TOKEN;
            let usage_ratio =
                estimated_tokens as f64 / capabilities.context_window_tokens as f64;

            if usage_ratio >= CONTEXT_CRITICAL_THRESHOLD {
                let warning = format!(
                    "[Context Usage Critical] 当前上下文约 {estimated_tokens} tokens ({:.0}%)。\
                     你 MUST 立即使用 tape-handoff 工具：提供详细 summary 和 next_steps，然后继续工作。\
                     不 handoff 将导致下一轮调用失败。",
                    usage_ratio * 100.0
                );
                messages.push(llm::Message::user(warning));
            } else if usage_ratio >= CONTEXT_WARN_THRESHOLD {
                let warning = format!(
                    "[Context Usage Warning] 当前上下文约 {estimated_tokens} tokens ({:.0}%)。\
                     你 SHOULD 考虑使用 tape-handoff 工具保存进度并截断上下文。",
                    usage_ratio * 100.0
                );
                messages.push(llm::Message::user(warning));
            }
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

    // Max iterations exhausted — return partial results
    warn!(
        max_iterations,
        tool_calls_made, "inline agent loop hit max iterations limit, returning partial results"
    );
    let trace = TurnTrace {
        duration_ms:      turn_start.elapsed().as_millis() as u64,
        model:            model.clone(),
        input_text:       Some(input_text.clone()),
        iterations:       iteration_traces,
        final_text_len:   last_accumulated_text.len(),
        total_tool_calls: tool_calls_made,
        success:          true,
        error:            None,
    };
    Ok(AgentTurnResult {
        text: last_accumulated_text,
        iterations: max_iterations,
        tool_calls: tool_calls_made,
        model: model.clone(),
        trace,
    })
}
