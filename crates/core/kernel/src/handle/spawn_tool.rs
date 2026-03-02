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

//! SpawnTool — LLM-callable tool for spawning child agents via ProcessHandle.
//!
//! Supports two modes:
//! - **Single**: `{"agent": "scout", "task": "Find auth code"}` — spawn a named
//!   manifest and wait for its result.
//! - **Parallel**: `{"parallel": [{"agent": "...", "task": "..."}],
//!   "max_concurrency": 4}` — spawn multiple agents concurrently and collect
//!   all results.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tracing::{info, warn};

use super::{AgentHandle, process_handle::ProcessHandle};
use crate::process::{AgentManifest, agent_registry::AgentRegistry};

pub struct SpawnTool {
    handle:         Arc<ProcessHandle>,
    agent_registry: Arc<AgentRegistry>,
}

impl SpawnTool {
    pub fn new(handle: Arc<ProcessHandle>, agent_registry: Arc<AgentRegistry>) -> Self {
        Self {
            handle,
            agent_registry,
        }
    }

    fn available_agents(&self) -> Vec<String> {
        self.agent_registry
            .list()
            .iter()
            .map(|m| m.name.clone())
            .collect()
    }

    fn resolve_manifest(&self, name: &str) -> Result<AgentManifest, anyhow::Error> {
        self.agent_registry.get(name).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown agent: '{}'. Available agents: {:?}",
                name,
                self.available_agents()
            )
        })
    }

    /// Spawn a single agent and wait for its result.
    async fn spawn_single(
        &self,
        agent_name: &str,
        task: &str,
    ) -> Result<serde_json::Value, anyhow::Error> {
        let manifest = self.resolve_manifest(agent_name)?;

        info!(
            agent = agent_name,
            task = task,
            "SpawnTool: spawning single agent"
        );

        let handle = self
            .handle
            .spawn(manifest, task.to_string())
            .await
            .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?;

        let agent_id = handle.agent_id;

        // Wait for completion
        let result = handle.result_rx.await.map_err(|_| {
            anyhow::anyhow!("agent {} was dropped without producing a result", agent_id)
        })?;

        Ok(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "output": result.output,
            "iterations": result.iterations,
            "tool_calls": result.tool_calls,
        }))
    }

    /// Spawn multiple agents in parallel and collect all results.
    async fn spawn_parallel(
        &self,
        tasks: Vec<SpawnRequest>,
    ) -> Result<serde_json::Value, anyhow::Error> {
        info!(
            count = tasks.len(),
            "SpawnTool: spawning agents in parallel"
        );

        // Spawn all agents
        let mut handles: Vec<(String, AgentHandle)> = Vec::new();
        for task_req in &tasks {
            let manifest = self.resolve_manifest(&task_req.agent)?;
            match self.handle.spawn(manifest, task_req.task.clone()).await {
                Ok(handle) => handles.push((task_req.agent.clone(), handle)),
                Err(e) => {
                    warn!(
                        agent = %task_req.agent,
                        error = %e,
                        "failed to spawn parallel agent"
                    );
                    // Continue spawning others
                }
            }
        }

        // Wait for all results
        let mut results = Vec::new();
        for (agent_name, handle) in handles {
            let agent_id = handle.agent_id;
            match handle.result_rx.await {
                Ok(result) => {
                    results.push(serde_json::json!({
                        "agent": agent_name,
                        "agent_id": agent_id.to_string(),
                        "output": result.output,
                        "iterations": result.iterations,
                        "tool_calls": result.tool_calls,
                    }));
                }
                Err(_) => {
                    results.push(serde_json::json!({
                        "agent": agent_name,
                        "agent_id": agent_id.to_string(),
                        "error": "agent was dropped without producing a result",
                    }));
                }
            }
        }

        Ok(serde_json::json!({
            "results": results,
            "total": results.len(),
        }))
    }
}

/// Parameters for a single spawn request within a parallel batch.
#[derive(Debug, Deserialize)]
struct SpawnRequest {
    agent: String,
    task:  String,
}

/// Top-level parameters for the spawn_agent tool.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SpawnParams {
    /// Parallel mode: spawn multiple agents concurrently.
    Parallel {
        parallel:        Vec<SpawnRequest>,
        #[serde(default)]
        max_concurrency: Option<usize>,
    },
    /// Single mode: spawn one named agent.
    Single { agent: String, task: String },
}

#[async_trait]
impl crate::tool::AgentTool for SpawnTool {
    fn name(&self) -> &str { "spawn_agent" }

    fn description(&self) -> &str {
        "Spawn one or more child agents to perform tasks. Single mode: {\"agent\": \"<name>\", \
         \"task\": \"<description>\"}. Parallel mode: {\"parallel\": [{\"agent\": \"<name>\", \
         \"task\": \"...\"}], \"max_concurrency\": 4}. Returns the agent's output when complete."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agents = self.available_agents();
        serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "properties": {
                        "agent": {
                            "type": "string",
                            "description": format!("Agent name to spawn. Available: {:?}", agents),
                            "enum": agents,
                        },
                        "task": {
                            "type": "string",
                            "description": "Task description for the agent"
                        }
                    },
                    "required": ["agent", "task"]
                },
                {
                    "properties": {
                        "parallel": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "agent": {
                                        "type": "string",
                                        "description": "Agent name to spawn",
                                        "enum": agents,
                                    },
                                    "task": {
                                        "type": "string",
                                        "description": "Task description"
                                    }
                                },
                                "required": ["agent", "task"]
                            },
                            "description": "Array of agents to spawn in parallel"
                        },
                        "max_concurrency": {
                            "type": "integer",
                            "description": "Max concurrent agents (optional)",
                            "default": 4
                        }
                    },
                    "required": ["parallel"]
                }
            ]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let spawn_params: SpawnParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid spawn_agent params: {e}"))?;

        match spawn_params {
            SpawnParams::Single { agent, task } => self.spawn_single(&agent, &task).await,
            SpawnParams::Parallel {
                parallel,
                max_concurrency: _,
            } => self.spawn_parallel(parallel).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        event_queue::InMemoryEventQueue,
        process::{AgentId, SessionId, principal::Principal},
        tool::AgentTool,
    };

    fn make_test_handle() -> Arc<ProcessHandle> {
        Arc::new(ProcessHandle::new(
            AgentId::new(),
            SessionId::new("test-session"),
            Principal::user("test-user"),
            Arc::new(InMemoryEventQueue::new(4096)),
        ))
    }

    fn make_test_agent_registry() -> Arc<AgentRegistry> {
        Arc::new(AgentRegistry::new(
            crate::testing::test_manifests(),
            std::env::temp_dir().join("spawn_tool_test_agents"),
        ))
    }

    #[test]
    fn test_spawn_tool_metadata() {
        let handle = make_test_handle();
        let registry = make_test_agent_registry();
        let tool = SpawnTool::new(handle, registry);

        assert_eq!(tool.name(), "spawn_agent");
        assert!(tool.description().contains("Spawn"));
        assert!(tool.description().contains("parallel"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["oneOf"].is_array());
    }

    #[test]
    fn test_spawn_tool_available_agents() {
        let handle = make_test_handle();
        let registry = make_test_agent_registry();
        let tool = SpawnTool::new(handle, registry);

        let agents = tool.available_agents();
        assert!(agents.contains(&"rara".to_string()));
        assert!(agents.contains(&"scout".to_string()));
    }

    #[tokio::test]
    async fn test_spawn_tool_unknown_agent() {
        let handle = make_test_handle();
        let registry = make_test_agent_registry();
        let tool = SpawnTool::new(handle, registry);

        let params = serde_json::json!({
            "agent": "nonexistent_agent",
            "task": "do something"
        });

        let result = tool.execute(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[tokio::test]
    async fn test_spawn_params_parsing_single() {
        let params = serde_json::json!({
            "agent": "scout",
            "task": "Find auth"
        });
        let parsed: SpawnParams = serde_json::from_value(params).unwrap();
        assert!(matches!(parsed, SpawnParams::Single { .. }));
    }

    #[tokio::test]
    async fn test_spawn_params_parsing_parallel() {
        let params = serde_json::json!({
            "parallel": [
                {"agent": "scout", "task": "task1"},
                {"agent": "worker", "task": "task2"}
            ],
            "max_concurrency": 2
        });
        let parsed: SpawnParams = serde_json::from_value(params).unwrap();
        assert!(matches!(parsed, SpawnParams::Parallel { .. }));
    }
}
