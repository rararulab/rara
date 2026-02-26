//! Sub-agent execution engine.
//!
//! The core type is [`SubagentExecutor`], which holds the shared state needed
//! to run sub-agents (LLM provider, agent definitions, parent tools, default
//! model) and exposes three execution methods:
//!
//! - [`run_single`](SubagentExecutor::run_single) — Run one sub-agent.
//! - [`run_chain`](SubagentExecutor::run_chain) — Run sequentially, piping
//!   output via `{previous}`.
//! - [`run_parallel`](SubagentExecutor::run_parallel) — Run concurrently with a
//!   semaphore limit.
//!
//! # Tool Isolation
//!
//! Each sub-agent receives a filtered copy of the parent's [`ToolRegistry`].
//! The `"subagent"` tool is always excluded to prevent infinite recursion
//! (a sub-agent spawning another sub-agent spawning another...).

use std::sync::Arc;

use tracing::{info, warn};

use super::definition::{AgentDefinition, AgentDefinitionRegistry};
use crate::{
    model::LlmProviderLoaderRef,
    runner::{AgentRunner, UserContent},
    tool_registry::ToolRegistry,
};

/// Structured result from running a single sub-agent.
///
/// Serialized to JSON and returned to the parent agent as the tool call
/// result. The parent LLM can inspect `success`, `output`, and `error`
/// to decide what to do next.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubagentResult {
    /// Name of the agent that was executed (matches the definition name).
    pub agent_name: String,
    /// Final text output from the sub-agent's LLM response.
    /// Empty string on failure.
    pub output:     String,
    /// Number of LLM round-trips the sub-agent consumed.
    pub iterations: usize,
    /// Total number of tool calls the sub-agent made across all iterations.
    pub tool_calls: usize,
    /// Whether the sub-agent completed without errors.
    pub success:    bool,
    /// Error message if the sub-agent failed (`success == false`).
    pub error:      Option<String>,
}

/// Sub-agent executor — holds shared state and provides execution methods.
///
/// Instead of passing `llm_provider`, `parent_tools`, `default_model`, and
/// `definitions` as separate arguments to every function, they live here as
/// fields. The three execution methods (`run_single`, `run_chain`,
/// `run_parallel`) only take task-specific parameters.
///
/// # Clone
///
/// `SubagentExecutor` is cheaply cloneable (all fields are `Arc` or `String`).
/// This allows `run_parallel` to clone the executor into each spawned tokio
/// task without deep-copying the underlying data.
#[derive(Clone)]
pub struct SubagentExecutor {
    /// Shared LLM provider loader for creating sub-agent runners.
    llm_provider:  LlmProviderLoaderRef,
    /// Registry of all available agent definitions (loaded from `agents/`
    /// dirs).
    definitions:   Arc<AgentDefinitionRegistry>,
    /// Snapshot of the parent agent's tools, taken before `SubagentTool` was
    /// registered. Sub-agents receive a filtered subset of these tools.
    parent_tools:  Arc<ToolRegistry>,
    /// Fallback model name used when an agent definition doesn't specify one
    /// (e.g. `"openai/gpt-4o"`).
    default_model: String,
}

impl SubagentExecutor {
    /// Create a new executor with the given shared state.
    ///
    /// # Arguments
    ///
    /// - `llm_provider` — Shared LLM provider loader.
    /// - `definitions` — Registry of agent definitions (loaded from markdown
    ///   files).
    /// - `parent_tools` — Snapshot of the parent's tool registry. **Must be
    ///   captured before `SubagentTool` is registered** to prevent recursion.
    /// - `default_model` — Fallback model name for agents that don't specify
    ///   one.
    pub fn new(
        llm_provider: LlmProviderLoaderRef,
        definitions: Arc<AgentDefinitionRegistry>,
        parent_tools: Arc<ToolRegistry>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            llm_provider,
            definitions,
            parent_tools,
            default_model: default_model.into(),
        }
    }

    /// Access the agent definition registry.
    pub fn definitions(&self) -> &AgentDefinitionRegistry { &self.definitions }

    /// Build a filtered [`ToolRegistry`] for a sub-agent.
    ///
    /// If the agent definition specifies a `tools` whitelist, only those tools
    /// are included. If the whitelist is empty, all parent tools are inherited.
    /// In both cases, the `"subagent"` tool is always excluded to prevent
    /// recursive sub-agent spawning.
    fn build_tools(&self, def: &AgentDefinition) -> ToolRegistry {
        if def.tools.is_empty() {
            // No whitelist — inherit all parent tools except "subagent".
            let all_names: Vec<String> = self
                .parent_tools
                .tool_names()
                .into_iter()
                .filter(|n| n != "subagent")
                .collect();
            self.parent_tools.filtered(&all_names)
        } else {
            // Whitelist specified — only include listed tools, still excluding "subagent".
            let names: Vec<String> = def
                .tools
                .iter()
                .filter(|n| n.as_str() != "subagent")
                .cloned()
                .collect();
            self.parent_tools.filtered(&names)
        }
    }

    /// Execute a single sub-agent with an isolated context.
    ///
    /// Creates a fresh [`AgentRunner`] configured with the agent definition's
    /// system prompt, model (or `default_model` fallback), and filtered tool
    /// set. The sub-agent runs its full tool-calling loop and returns the
    /// final assistant text as a [`SubagentResult`].
    ///
    /// Sub-agents do not receive streaming events (`on_event` is `None`) since
    /// their output is consumed by the parent agent, not streamed to a UI.
    pub async fn run_single(&self, def: &AgentDefinition, task: &str) -> SubagentResult {
        let model = def.model.as_deref().unwrap_or(&self.default_model);
        let max_iter = def.max_iterations.unwrap_or(15);
        let tools = self.build_tools(def);

        info!(
            agent = %def.name,
            model,
            max_iterations = max_iter,
            tool_count = tools.len(),
            "running sub-agent"
        );

        let runner = AgentRunner::builder()
            .llm_provider(Arc::clone(&self.llm_provider))
            .model_name(model.to_owned())
            .system_prompt(def.system_prompt.clone())
            .user_content(UserContent::Text(task.to_owned()))
            .max_iterations(max_iter)
            .build();

        match runner.run(&tools, None).await {
            Ok(response) => {
                let text = response
                    .provider_response
                    .choices
                    .first()
                    .and_then(|c| c.message.content.as_deref())
                    .unwrap_or("")
                    .to_owned();
                SubagentResult {
                    agent_name: def.name.clone(),
                    output:     text,
                    iterations: response.iterations,
                    tool_calls: response.tool_calls_made,
                    success:    true,
                    error:      None,
                }
            }
            Err(err) => {
                warn!(agent = %def.name, error = %err, "sub-agent failed");
                SubagentResult {
                    agent_name: def.name.clone(),
                    output:     String::new(),
                    iterations: 0,
                    tool_calls: 0,
                    success:    false,
                    error:      Some(err.to_string()),
                }
            }
        }
    }

    /// Execute a chain of sub-agents sequentially, piping output between steps.
    ///
    /// Each step is an `(agent_name, task_template)` pair. The `task_template`
    /// may contain the literal string `{previous}`, which is replaced with the
    /// output of the preceding step before execution. The first step receives
    /// an empty string for `{previous}`.
    ///
    /// # Error Handling
    ///
    /// The chain stops immediately on the first failing step (fail-fast). The
    /// returned `Vec` will contain results up to and including the failed step,
    /// but no further steps will be executed.
    pub async fn run_chain(&self, steps: &[(String, String)]) -> Vec<SubagentResult> {
        let mut results = Vec::with_capacity(steps.len());
        let mut previous_output = String::new();

        for (i, (agent_name, task_template)) in steps.iter().enumerate() {
            // Look up the agent definition by name.
            let Some(def) = self.definitions.get(agent_name) else {
                results.push(SubagentResult {
                    agent_name: agent_name.clone(),
                    output:     String::new(),
                    iterations: 0,
                    tool_calls: 0,
                    success:    false,
                    error:      Some(format!("agent definition not found: {agent_name}")),
                });
                break;
            };

            // Replace {previous} placeholder with the prior step's output.
            let task = task_template.replace("{previous}", &previous_output);

            info!(step = i + 1, total = steps.len(), agent = %agent_name, "running chain step");

            let result = self.run_single(def, &task).await;

            if !result.success {
                // Fail-fast: stop the chain on first error.
                results.push(result);
                break;
            }

            // Save output for the next step's {previous} substitution.
            previous_output = result.output.clone();
            results.push(result);
        }

        results
    }

    /// Execute multiple sub-agents concurrently with a concurrency limit.
    ///
    /// Each task is an `(agent_name, task)` pair. All tasks are spawned as
    /// independent tokio tasks, but a [`Semaphore`](tokio::sync::Semaphore)
    /// limits how many run simultaneously to avoid overwhelming the LLM
    /// provider with concurrent requests.
    ///
    /// Results are returned in the same order as the input `tasks` slice.
    /// Unlike [`run_chain`](Self::run_chain), parallel execution does NOT
    /// stop on errors — all tasks run to completion (or failure) independently.
    pub async fn run_parallel(
        &self,
        tasks: &[(String, String)],
        max_concurrency: usize,
    ) -> Vec<SubagentResult> {
        use tokio::sync::Semaphore;

        let semaphore = Arc::new(Semaphore::new(max_concurrency));
        let mut handles = Vec::with_capacity(tasks.len());

        for (agent_name, task) in tasks {
            let sem = Arc::clone(&semaphore);
            let executor = self.clone();
            let agent_name = agent_name.clone();
            let task = task.clone();

            handles.push(tokio::spawn(async move {
                // Acquire a semaphore permit to limit concurrency.
                let _permit = sem.acquire().await.expect("semaphore closed");

                let def = executor.definitions().get(&agent_name).cloned();
                match def {
                    Some(d) => executor.run_single(&d, &task).await,
                    None => SubagentResult {
                        agent_name,
                        output: String::new(),
                        iterations: 0,
                        tool_calls: 0,
                        success: false,
                        error: Some("agent definition not found".to_string()),
                    },
                }
            }));
        }

        // Collect results in order, handling panicked tasks gracefully.
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(err) => results.push(SubagentResult {
                    agent_name: "unknown".to_string(),
                    output:     String::new(),
                    iterations: 0,
                    tool_calls: 0,
                    success:    false,
                    error:      Some(format!("task panicked: {err}")),
                }),
            }
        }
        results
    }
}
