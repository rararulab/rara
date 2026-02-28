// Copyright 2025 Crrow
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

//! SpawnTool — LLM-callable tool for spawning child agents via KernelHandle.
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

use super::{AgentHandle, ProcessOps};
use crate::process::{AgentManifest, manifest_loader::ManifestLoader};

/// An `AgentTool` implementation that allows LLMs to spawn child agents.
///
/// The tool wraps a `ScopedKernelHandle` (via `Arc<dyn ProcessOps>`) and
/// the `ManifestLoader` for resolving agent names to manifests.
pub struct SpawnTool {
    /// Process operations handle for spawning children.
    process_ops:     Arc<dyn ProcessOps>,
    /// Manifest loader for looking up named agent definitions.
    manifest_loader: Arc<ManifestLoader>,
}

impl SpawnTool {
    /// Create a new SpawnTool.
    pub fn new(process_ops: Arc<dyn ProcessOps>, manifest_loader: Arc<ManifestLoader>) -> Self {
        Self {
            process_ops,
            manifest_loader,
        }
    }

    /// Available agent names for the JSON schema description.
    fn available_agents(&self) -> Vec<String> {
        self.manifest_loader
            .list()
            .iter()
            .map(|m| m.name.clone())
            .collect()
    }

    /// Resolve a manifest by name from the loader.
    fn resolve_manifest(&self, name: &str) -> Result<AgentManifest, anyhow::Error> {
        self.manifest_loader.get(name).cloned().ok_or_else(|| {
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
            .process_ops
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
            match self
                .process_ops
                .spawn(manifest, task_req.task.clone())
                .await
            {
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
    use crate::error::KernelError;
    use crate::handle::AgentHandle;
    use crate::process::{AgentId, AgentManifest, AgentResult, ProcessInfo};
    use crate::tool::AgentTool;
    use tokio::sync::oneshot;

    /// Mock ProcessOps that spawns agents and immediately returns results.
    struct MockProcessOps {
        /// Whether to fail spawns.
        fail: bool,
    }

    #[async_trait]
    impl ProcessOps for MockProcessOps {
        async fn spawn(
            &self,
            manifest: AgentManifest,
            input: String,
        ) -> crate::error::Result<AgentHandle> {
            if self.fail {
                return Err(KernelError::SpawnLimitReached {
                    message: "mock limit reached".to_string(),
                });
            }

            let agent_id = AgentId::new();
            let (tx, rx) = oneshot::channel();
            let (mailbox_tx, _mailbox_rx) = tokio::sync::mpsc::channel(16);

            // Immediately send a result
            let _ = tx.send(AgentResult {
                output:     format!("Result from {}: processed '{}'", manifest.name, input),
                iterations: 1,
                tool_calls: 0,
            });

            Ok(AgentHandle {
                agent_id,
                mailbox: mailbox_tx,
                result_rx: rx,
            })
        }

        async fn send(&self, _agent_id: AgentId, _message: String) -> crate::error::Result<String> {
            Err(KernelError::Other {
                message: "not implemented".into(),
            })
        }

        fn status(&self, _agent_id: AgentId) -> crate::error::Result<ProcessInfo> {
            Err(KernelError::ProcessNotFound {
                id: "mock".to_string(),
            })
        }

        fn kill(&self, _agent_id: AgentId) -> crate::error::Result<()> { Ok(()) }

        fn children(&self) -> Vec<ProcessInfo> { vec![] }
    }

    fn make_test_manifest_loader() -> Arc<ManifestLoader> {
        let mut loader = ManifestLoader::new();
        loader.load_bundled();
        Arc::new(loader)
    }

    #[tokio::test]
    async fn test_spawn_tool_single_mode() {
        let process_ops = Arc::new(MockProcessOps { fail: false }) as Arc<dyn ProcessOps>;
        let loader = make_test_manifest_loader();
        let tool = SpawnTool::new(process_ops, loader);

        let params = serde_json::json!({
            "agent": "scout",
            "task": "Find auth code"
        });

        let result = tool.execute(params).await.unwrap();
        assert!(
            result["output"]
                .as_str()
                .unwrap()
                .contains("Result from scout")
        );
        assert!(result["agent_id"].as_str().is_some());
        assert_eq!(result["iterations"], 1);
        assert_eq!(result["tool_calls"], 0);
    }

    #[tokio::test]
    async fn test_spawn_tool_parallel_mode() {
        let process_ops = Arc::new(MockProcessOps { fail: false }) as Arc<dyn ProcessOps>;
        let loader = make_test_manifest_loader();
        let tool = SpawnTool::new(process_ops, loader);

        let params = serde_json::json!({
            "parallel": [
                {"agent": "scout", "task": "Find files"},
                {"agent": "planner", "task": "Create plan"}
            ]
        });

        let result = tool.execute(params).await.unwrap();
        assert_eq!(result["total"], 2);
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);

        // Check both results have output
        for r in results {
            assert!(r["output"].as_str().is_some());
        }
    }

    #[tokio::test]
    async fn test_spawn_tool_unknown_agent() {
        let process_ops = Arc::new(MockProcessOps { fail: false }) as Arc<dyn ProcessOps>;
        let loader = make_test_manifest_loader();
        let tool = SpawnTool::new(process_ops, loader);

        let params = serde_json::json!({
            "agent": "nonexistent_agent",
            "task": "do something"
        });

        let result = tool.execute(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[tokio::test]
    async fn test_spawn_tool_spawn_failure() {
        let process_ops = Arc::new(MockProcessOps { fail: true }) as Arc<dyn ProcessOps>;
        let loader = make_test_manifest_loader();
        let tool = SpawnTool::new(process_ops, loader);

        let params = serde_json::json!({
            "agent": "scout",
            "task": "Find something"
        });

        let result = tool.execute(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("spawn failed"));
    }

    #[test]
    fn test_spawn_tool_metadata() {
        let process_ops = Arc::new(MockProcessOps { fail: false }) as Arc<dyn ProcessOps>;
        let loader = make_test_manifest_loader();
        let tool = SpawnTool::new(process_ops, loader);

        assert_eq!(tool.name(), "spawn_agent");
        assert!(tool.description().contains("Spawn"));
        assert!(tool.description().contains("parallel"));

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["oneOf"].is_array());
    }

    #[test]
    fn test_spawn_tool_available_agents() {
        let process_ops = Arc::new(MockProcessOps { fail: false }) as Arc<dyn ProcessOps>;
        let loader = make_test_manifest_loader();
        let tool = SpawnTool::new(process_ops, loader);

        let agents = tool.available_agents();
        assert!(agents.contains(&"scout".to_string()));
        assert!(agents.contains(&"planner".to_string()));
        assert!(agents.contains(&"worker".to_string()));
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
