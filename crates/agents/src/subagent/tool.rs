//! Sub-agent tool — the LLM-callable tool that dispatches sub-agents.
//!
//! This module defines [`SubagentTool`], an implementation of [`tool_core::AgentTool`]
//! that the parent agent can invoke to spawn specialized child agents. It acts as the
//! bridge between the LLM's tool-calling interface and the executor functions in
//! [`super::executor`].
//!
//! # JSON Parameter Formats
//!
//! The tool accepts three mutually exclusive JSON shapes (parsed via
//! `#[serde(untagged)]` on [`SubagentParams`]):
//!
//! **Single mode** — run one agent:
//! ```json
//! {"agent": "scout", "task": "Find all authentication code"}
//! ```
//!
//! **Chain mode** — run agents sequentially, piping output:
//! ```json
//! {"chain": [
//!   {"agent": "scout",   "task": "Find relevant code"},
//!   {"agent": "planner", "task": "Create plan based on: {previous}"}
//! ]}
//! ```
//!
//! **Parallel mode** — run agents concurrently:
//! ```json
//! {"parallel": [
//!   {"agent": "scout", "task": "Find models"},
//!   {"agent": "scout", "task": "Find providers"}
//! ], "max_concurrency": 2}
//! ```
//!
//! # Concurrency Limits
//!
//! Parallel mode enforces two limits:
//! - **Task count**: at most [`MAX_PARALLEL_TASKS`] (8) tasks per invocation.
//! - **Concurrency**: at most [`DEFAULT_CONCURRENCY`] (4) tasks run simultaneously,
//!   even if the caller requests more via `max_concurrency`.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use crate::{model::LlmProviderLoaderRef, tool_registry::ToolRegistry};

use super::{definition::AgentDefinitionRegistry, executor};

/// Hard upper limit on the number of parallel sub-agent tasks in a single
/// invocation. Prevents the LLM from spawning an unbounded number of
/// concurrent requests that could overwhelm the LLM provider's API.
const MAX_PARALLEL_TASKS: usize = 8;

/// Default (and maximum effective) concurrency for parallel mode.
/// Even if the caller supplies a higher `max_concurrency`, it is clamped
/// to this value via `.min(DEFAULT_CONCURRENCY)`.
const DEFAULT_CONCURRENCY: usize = 4;

/// A single step in a chain or parallel execution: pairs an agent name
/// with a task description.
///
/// Used inside [`SubagentParams::Chain`] and [`SubagentParams::Parallel`]
/// to describe what each sub-agent should do.
///
/// # Fields
///
/// - `agent` — Name of the agent definition to use (must exist in the
///   [`AgentDefinitionRegistry`]).
/// - `task` — Natural-language task description. In chain mode, may contain
///   the `{previous}` placeholder which is replaced with the prior step's
///   output before execution.
#[derive(Debug, Clone, Deserialize)]
pub struct SubagentStep {
    /// Agent definition name (e.g. "scout", "planner", "worker").
    pub agent: String,
    /// Task description for this step; may contain `{previous}` in chain mode.
    pub task:  String,
}

/// Deserialized parameters for the `"subagent"` tool call.
///
/// Supports three execution modes as an untagged enum. **Variant order matters**:
/// serde's `#[serde(untagged)]` tries variants top-to-bottom, so `Chain` and
/// `Parallel` (which have distinctive top-level keys) must come before `Single`
/// (which only has `agent` + `task` — fields that also appear inside chain/parallel
/// step objects). If `Single` were first, JSON like `{"chain": [...]}` could
/// accidentally match `Single` with extra ignored fields.
///
/// # Variants
///
/// - [`Chain`](SubagentParams::Chain) — Sequential execution with output piping.
/// - [`Parallel`](SubagentParams::Parallel) — Concurrent execution with semaphore.
/// - [`Single`](SubagentParams::Single) — Run one agent with one task.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SubagentParams {
    /// Sequential chain: each step can reference `{previous}` to receive the
    /// prior step's output. Stops on first failure (fail-fast).
    Chain {
        chain: Vec<SubagentStep>,
    },
    /// Concurrent execution: all tasks run independently with a semaphore-based
    /// concurrency limit. Does NOT stop on individual failures.
    Parallel {
        parallel:        Vec<SubagentStep>,
        /// Optional concurrency limit (clamped to [`DEFAULT_CONCURRENCY`]).
        #[serde(default)]
        max_concurrency: Option<usize>,
    },
    /// Run a single sub-agent with one task. Simplest mode.
    Single {
        agent: String,
        task:  String,
    },
}

/// The `"subagent"` tool — registered in the parent agent's [`ToolRegistry`]
/// to allow the LLM to dispatch specialized child agents.
///
/// When the parent LLM calls this tool, it:
/// 1. Deserializes the JSON parameters into [`SubagentParams`].
/// 2. Looks up agent definitions by name from [`AgentDefinitionRegistry`].
/// 3. Delegates to the appropriate executor function ([`executor::run_single`],
///    [`executor::run_chain`], or [`executor::run_parallel`]).
/// 4. Returns the result(s) as serialized JSON back to the parent LLM.
///
/// # Recursion Prevention
///
/// The `parent_tools` snapshot passed to this struct should be taken **before**
/// the `SubagentTool` itself is registered, ensuring sub-agents never have
/// access to the `"subagent"` tool. Additionally, [`executor::build_subagent_tools`]
/// explicitly filters out `"subagent"` as a safety net.
pub struct SubagentTool {
    /// Reference to the LLM provider loader, shared with sub-agents via `Arc`.
    llm_provider:  LlmProviderLoaderRef,
    /// Registry of all available agent definitions (loaded from `agents/` dirs).
    definitions:   Arc<AgentDefinitionRegistry>,
    /// Snapshot of the parent agent's tools, taken before this tool was registered.
    /// Sub-agents receive a filtered subset of these tools.
    parent_tools:  Arc<ToolRegistry>,
    /// Fallback model name used when an agent definition doesn't specify one.
    default_model: String,
}

impl SubagentTool {
    /// Create a new `SubagentTool`.
    ///
    /// # Arguments
    ///
    /// - `llm_provider` — Shared LLM provider loader for creating sub-agent runners.
    /// - `definitions` — Registry of agent definitions (loaded from markdown files).
    /// - `parent_tools` — Snapshot of the parent's tool registry. **Must be captured
    ///   before this tool is registered** to prevent recursive sub-agent spawning.
    /// - `default_model` — Fallback model name (e.g. `"openai/gpt-4o"`) used when
    ///   an agent definition's `model` field is `None`.
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
}

/// Implementation of the [`tool_core::AgentTool`] trait, making `SubagentTool`
/// callable by the parent agent's LLM through the standard tool-calling protocol.
///
/// - [`name()`](tool_core::AgentTool::name) returns `"subagent"`.
/// - [`description()`](tool_core::AgentTool::description) returns a human-readable
///   summary of the three execution modes, shown to the LLM in the tool list.
/// - [`parameters_schema()`](tool_core::AgentTool::parameters_schema) returns a
///   JSON Schema describing the accepted parameter shapes, dynamically including
///   the names and descriptions of all registered agent definitions.
/// - [`execute()`](tool_core::AgentTool::execute) deserializes parameters, dispatches
///   to the appropriate executor, and returns serialized results.
#[async_trait]
impl tool_core::AgentTool for SubagentTool {
    /// Tool name used by the LLM to invoke this tool in a tool_call.
    fn name(&self) -> &str {
        "subagent"
    }

    /// Human-readable description shown to the LLM, explaining the three
    /// available execution modes (single, chain, parallel) and their JSON formats.
    fn description(&self) -> &str {
        "Run sub-agents to handle complex tasks. Supports three modes:\n\
         1. Single: {\"agent\": \"<name>\", \"task\": \"<description>\"}\n\
         2. Chain: {\"chain\": [{\"agent\": \"<name>\", \"task\": \"...\"}]} \
         — sequential, use {previous} to reference prior output\n\
         3. Parallel: {\"parallel\": [{\"agent\": \"<name>\", \"task\": \"...\"}]} \
         — concurrent execution"
    }

    /// Generate the JSON Schema for this tool's parameters.
    ///
    /// The schema is built dynamically at runtime so that it includes the
    /// current list of registered agent definitions (name + description) in
    /// the `agent` field's description. This helps the LLM know which agents
    /// are available without needing a separate lookup step.
    fn parameters_schema(&self) -> serde_json::Value {
        let agents: Vec<String> = self
            .definitions
            .list()
            .iter()
            .map(|d| format!("{}: {}", d.name, d.description))
            .collect();
        let agents_desc = if agents.is_empty() {
            "No agents defined".to_string()
        } else {
            agents.join(", ")
        };

        serde_json::json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": format!("Agent name for single mode. Available: {agents_desc}")
                },
                "task": {
                    "type": "string",
                    "description": "Task description for the agent"
                },
                "chain": {
                    "type": "array",
                    "description": "Sequential chain of agent tasks. Use {previous} in task to reference prior output.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": { "type": "string" },
                            "task": { "type": "string" }
                        },
                        "required": ["agent", "task"]
                    }
                },
                "parallel": {
                    "type": "array",
                    "description": "Parallel agent tasks. Results are aggregated.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": { "type": "string" },
                            "task": { "type": "string" }
                        },
                        "required": ["agent", "task"]
                    }
                },
                "max_concurrency": {
                    "type": "integer",
                    "description": "Max concurrent tasks for parallel mode (default: 4, max: 8)"
                }
            }
        })
    }

    /// Execute the sub-agent tool with the given JSON parameters.
    ///
    /// Deserializes `params` into [`SubagentParams`] and dispatches to the
    /// appropriate executor function based on the detected mode:
    ///
    /// - **Single**: Looks up the agent definition by name, then calls
    ///   [`executor::run_single`]. Returns an error if the agent name is not
    ///   found (with a helpful message listing available agents).
    ///
    /// - **Chain**: Validates the chain is non-empty, converts steps to
    ///   `(name, task)` tuples, then calls [`executor::run_chain`].
    ///
    /// - **Parallel**: Validates the task list is non-empty and within
    ///   [`MAX_PARALLEL_TASKS`], clamps concurrency to [`DEFAULT_CONCURRENCY`],
    ///   then calls [`executor::run_parallel`].
    ///
    /// The return value is the executor's result serialized to JSON, which the
    /// parent agent's LLM can inspect to decide its next action.
    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let params: SubagentParams = serde_json::from_value(params)?;

        match params {
            SubagentParams::Single { agent, task } => {
                // Look up the agent definition; fail with available names if not found.
                let def = self.definitions.get(&agent).ok_or_else(|| {
                    anyhow::anyhow!(
                        "agent '{}' not found. Available: {}",
                        agent,
                        self.definitions
                            .list()
                            .iter()
                            .map(|d| d.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
                let result = executor::run_single(
                    def,
                    &task,
                    &self.llm_provider,
                    &self.parent_tools,
                    &self.default_model,
                )
                .await;
                Ok(serde_json::to_value(&result)?)
            }

            SubagentParams::Chain { chain } => {
                // Validate at least one step exists.
                if chain.is_empty() {
                    anyhow::bail!("chain must have at least one step");
                }
                // Convert SubagentStep structs to (name, task) tuples for the executor.
                let steps: Vec<(String, String)> =
                    chain.into_iter().map(|s| (s.agent, s.task)).collect();
                let results = executor::run_chain(
                    &steps,
                    &self.definitions,
                    &self.llm_provider,
                    &self.parent_tools,
                    &self.default_model,
                )
                .await;
                Ok(serde_json::to_value(&results)?)
            }

            SubagentParams::Parallel {
                parallel,
                max_concurrency,
            } => {
                // Validate task count constraints.
                if parallel.is_empty() {
                    anyhow::bail!("parallel must have at least one task");
                }
                if parallel.len() > MAX_PARALLEL_TASKS {
                    anyhow::bail!("too many parallel tasks (max {MAX_PARALLEL_TASKS})");
                }
                // Clamp concurrency to the default maximum to prevent abuse.
                let concurrency = max_concurrency
                    .unwrap_or(DEFAULT_CONCURRENCY)
                    .min(DEFAULT_CONCURRENCY);
                let tasks: Vec<(String, String)> =
                    parallel.into_iter().map(|s| (s.agent, s.task)).collect();
                let results = executor::run_parallel(
                    &tasks,
                    &self.definitions,
                    &self.llm_provider,
                    &self.parent_tools,
                    &self.default_model,
                    concurrency,
                )
                .await;
                Ok(serde_json::to_value(&results)?)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_params() {
        let json = serde_json::json!({
            "agent": "scout",
            "task": "Find auth code"
        });
        let params: SubagentParams = serde_json::from_value(json).unwrap();
        assert!(matches!(params, SubagentParams::Single { .. }));
    }

    #[test]
    fn parse_chain_params() {
        let json = serde_json::json!({
            "chain": [
                { "agent": "scout", "task": "Find code" },
                { "agent": "planner", "task": "Plan based on {previous}" }
            ]
        });
        let params: SubagentParams = serde_json::from_value(json).unwrap();
        assert!(matches!(params, SubagentParams::Chain { .. }));
    }

    #[test]
    fn parse_parallel_params() {
        let json = serde_json::json!({
            "parallel": [
                { "agent": "scout", "task": "Find models" },
                { "agent": "scout", "task": "Find providers" }
            ],
            "max_concurrency": 2
        });
        let params: SubagentParams = serde_json::from_value(json).unwrap();
        assert!(matches!(params, SubagentParams::Parallel { .. }));
    }
}
