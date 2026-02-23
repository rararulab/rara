use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use crate::{model::LlmProviderLoaderRef, tool_registry::ToolRegistry};

use super::{definition::AgentDefinitionRegistry, executor};

/// Maximum number of parallel sub-agent tasks.
const MAX_PARALLEL_TASKS: usize = 8;
/// Default concurrency for parallel mode.
const DEFAULT_CONCURRENCY: usize = 4;

/// A single sub-agent task step: agent name + task description.
#[derive(Debug, Clone, Deserialize)]
pub struct SubagentStep {
    pub agent: String,
    pub task:  String,
}

/// Parameters for the subagent tool. Supports three modes.
/// NOTE: serde untagged tries variants in order. Chain and Parallel must come
/// before Single because Single's fields (agent, task) would also match
/// chain/parallel objects that happen to have extra fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SubagentParams {
    Chain {
        chain: Vec<SubagentStep>,
    },
    Parallel {
        parallel:        Vec<SubagentStep>,
        #[serde(default)]
        max_concurrency: Option<usize>,
    },
    Single {
        agent: String,
        task:  String,
    },
}

/// Tool that spawns sub-agents with isolated contexts.
pub struct SubagentTool {
    llm_provider:  LlmProviderLoaderRef,
    definitions:   Arc<AgentDefinitionRegistry>,
    parent_tools:  Arc<ToolRegistry>,
    default_model: String,
}

impl SubagentTool {
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

#[async_trait]
impl tool_core::AgentTool for SubagentTool {
    fn name(&self) -> &str {
        "subagent"
    }

    fn description(&self) -> &str {
        "Run sub-agents to handle complex tasks. Supports three modes:\n\
         1. Single: {\"agent\": \"<name>\", \"task\": \"<description>\"}\n\
         2. Chain: {\"chain\": [{\"agent\": \"<name>\", \"task\": \"...\"}]} \
         — sequential, use {previous} to reference prior output\n\
         3. Parallel: {\"parallel\": [{\"agent\": \"<name>\", \"task\": \"...\"}]} \
         — concurrent execution"
    }

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

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let params: SubagentParams = serde_json::from_value(params)?;

        match params {
            SubagentParams::Single { agent, task } => {
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
                if chain.is_empty() {
                    anyhow::bail!("chain must have at least one step");
                }
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
                if parallel.is_empty() {
                    anyhow::bail!("parallel must have at least one task");
                }
                if parallel.len() > MAX_PARALLEL_TASKS {
                    anyhow::bail!("too many parallel tasks (max {MAX_PARALLEL_TASKS})");
                }
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
