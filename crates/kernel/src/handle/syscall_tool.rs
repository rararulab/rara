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

//! SyscallTool — unified LLM-callable tool wrapping all agent-relevant
//! ProcessHandle operations.
//!
//! Instead of creating a separate `AgentTool` for each kernel capability,
//! this single tool dispatches on an `action` field. Adding new kernel
//! capabilities only requires adding a new [`SyscallParams`] variant and
//! a corresponding `exec_*` method — no new tool files needed.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tracing::{info, warn};

use super::{AgentHandle, process_handle::ProcessHandle};
use crate::{
    kv::KvScope,
    process::{AgentManifest, agent_registry::AgentRegistry},
    session::SessionKey,
};

/// Unified LLM-callable tool wrapping all agent-relevant ProcessHandle
/// syscall operations.
pub struct SyscallTool {
    handle:         Arc<ProcessHandle>,
    agent_registry: Arc<AgentRegistry>,
}

impl SyscallTool {
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

    // ========================================================================
    // Spawn
    // ========================================================================

    async fn exec_spawn(
        &self,
        agent_name: &str,
        task: &str,
    ) -> Result<serde_json::Value, anyhow::Error> {
        let manifest = self.resolve_manifest(agent_name)?;

        info!(
            agent = agent_name,
            task = task,
            "kernel: spawning single agent"
        );

        let handle = self
            .handle
            .spawn(manifest, task.to_string())
            .await
            .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?;

        let session_key = handle.session_key;

        let result = handle.result_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "agent {} was dropped without producing a result",
                session_key
            )
        })?;

        Ok(serde_json::json!({
            "agent_id": session_key.to_string(),
            "output": result.output,
            "iterations": result.iterations,
            "tool_calls": result.tool_calls,
        }))
    }

    async fn exec_spawn_parallel(
        &self,
        tasks: Vec<SpawnRequest>,
    ) -> Result<serde_json::Value, anyhow::Error> {
        info!(count = tasks.len(), "kernel: spawning agents in parallel");

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
                }
            }
        }

        let mut results = Vec::new();
        for (agent_name, handle) in handles {
            let agent_id = handle.session_key;
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

    // ========================================================================
    // Process queries & signals
    // ========================================================================

    async fn exec_status(&self, target: &str) -> anyhow::Result<serde_json::Value> {
        let agent_id = parse_session_key(target)?;
        let info = self
            .handle
            .status(agent_id)
            .await
            .map_err(|e| anyhow::anyhow!("status failed: {e}"))?;
        Ok(serde_json::json!({
            "agent_id": info.session_key.to_string(),
            "name": info.name,
            "state": info.state.to_string(),
            "parent_id": info.parent_id.map(|id| id.to_string()),
        }))
    }

    async fn exec_children(&self) -> anyhow::Result<serde_json::Value> {
        let children = self.handle.children().await;
        let list: Vec<serde_json::Value> = children
            .iter()
            .map(|c| {
                serde_json::json!({
                    "agent_id": c.session_key.to_string(),
                    "name": c.name,
                    "state": c.state.to_string(),
                })
            })
            .collect();
        Ok(serde_json::json!({ "children": list, "count": list.len() }))
    }

    async fn exec_signal(&self, target: &str, signal: &str) -> anyhow::Result<serde_json::Value> {
        let agent_id = parse_session_key(target)?;
        let result = match signal {
            "kill" => self.handle.kill(agent_id).await,
            "pause" => self.handle.pause(agent_id).await,
            "resume" => self.handle.resume(agent_id).await,
            _ => unreachable!(),
        };
        result.map_err(|e| anyhow::anyhow!("{signal} failed: {e}"))?;
        Ok(serde_json::json!({ "ok": true, "signal": signal, "target": target }))
    }

    // ========================================================================
    // Memory
    // ========================================================================

    async fn exec_mem_store(
        &self,
        key: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        self.handle
            .mem_store(key, value)
            .await
            .map_err(|e| anyhow::anyhow!("mem_store failed: {e}"))?;
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn exec_mem_recall(&self, key: &str) -> anyhow::Result<serde_json::Value> {
        let value = self
            .handle
            .mem_recall(key)
            .await
            .map_err(|e| anyhow::anyhow!("mem_recall failed: {e}"))?;
        Ok(serde_json::json!({ "key": key, "value": value }))
    }

    async fn exec_shared_store(
        &self,
        scope: &str,
        key: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let scope = parse_scope(scope)?;
        self.handle
            .shared_store(scope, key, value)
            .await
            .map_err(|e| anyhow::anyhow!("shared_store failed: {e}"))?;
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn exec_shared_recall(
        &self,
        scope: &str,
        key: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let scope = parse_scope(scope)?;
        let value = self
            .handle
            .shared_recall(scope, key)
            .await
            .map_err(|e| anyhow::anyhow!("shared_recall failed: {e}"))?;
        Ok(serde_json::json!({ "key": key, "value": value }))
    }

    // ========================================================================
    // Events
    // ========================================================================

    async fn exec_publish(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        self.handle
            .publish(event_type, payload)
            .await
            .map_err(|e| anyhow::anyhow!("publish failed: {e}"))?;
        Ok(serde_json::json!({ "ok": true }))
    }
}

// ============================================================================
// Parameter types
// ============================================================================

/// Top-level parameters: `action` selects the kernel operation.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SyscallParams {
    // -- Process --
    Spawn {
        agent: String,
        task:  String,
    },
    SpawnParallel {
        parallel:        Vec<SpawnRequest>,
        #[serde(default)]
        max_concurrency: Option<usize>,
    },
    Status {
        target: String,
    },
    Children,
    Kill {
        target: String,
    },
    Pause {
        target: String,
    },
    Resume {
        target: String,
    },
    // -- Memory --
    MemStore {
        key:   String,
        value: serde_json::Value,
    },
    MemRecall {
        key: String,
    },
    SharedStore {
        scope: String,
        key:   String,
        value: serde_json::Value,
    },
    SharedRecall {
        scope: String,
        key:   String,
    },
    // -- Events --
    Publish {
        event_type: String,
        payload:    serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct SpawnRequest {
    agent: String,
    task:  String,
}

// ============================================================================
// Helpers
// ============================================================================

fn parse_session_key(s: &str) -> anyhow::Result<SessionKey> {
    let uuid =
        uuid::Uuid::parse_str(s).map_err(|e| anyhow::anyhow!("invalid session key '{s}': {e}"))?;
    Ok(SessionKey(uuid))
}

fn parse_scope(scope: &str) -> anyhow::Result<KvScope> {
    match scope {
        "global" => Ok(KvScope::Global),
        s if s.starts_with("team:") => {
            Ok(KvScope::Team(s.strip_prefix("team:").unwrap().to_string()))
        }
        s if s.starts_with("agent:") => {
            let uuid_str = s.strip_prefix("agent:").unwrap();
            let uuid = uuid::Uuid::parse_str(uuid_str)
                .map_err(|e| anyhow::anyhow!("invalid agent UUID in scope: {e}"))?;
            Ok(KvScope::Agent(uuid))
        }
        _ => Err(anyhow::anyhow!(
            "invalid scope '{scope}'. Expected 'global', 'team:<name>', or 'agent:<uuid>'"
        )),
    }
}

// ============================================================================
// AgentTool impl
// ============================================================================

#[async_trait]
impl crate::tool::AgentTool for SyscallTool {
    fn name(&self) -> &str { "kernel" }

    fn description(&self) -> &str {
        "Interact with the kernel: spawn agents, query process status, send signals, manage memory \
         (private & shared), and publish events. Set the 'action' field to select the operation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        // FIXME: we should not expose all internal agent for syscall !.
        let agents = self.available_agents();
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "spawn", "spawn_parallel",
                        "status", "children", "kill", "pause", "resume",
                        "mem_store", "mem_recall",
                        "shared_store", "shared_recall",
                        "publish"
                    ],
                    "description": "The kernel operation to perform"
                },
                "agent": {
                    "type": "string",
                    "description": format!("Agent name for spawn. Available: {:?}", agents),
                    "enum": agents,
                },
                "task": {
                    "type": "string",
                    "description": "Task description for spawn"
                },
                "parallel": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": { "type": "string" },
                            "task":  { "type": "string" }
                        },
                        "required": ["agent", "task"]
                    },
                    "description": "Array of {agent, task} for spawn_parallel"
                },
                "max_concurrency": {
                    "type": "integer",
                    "description": "Max concurrent agents for spawn_parallel"
                },
                "target": {
                    "type": "string",
                    "description": "Target agent ID (UUID) for status/kill/pause/resume"
                },
                "key": {
                    "type": "string",
                    "description": "Memory key for mem_store/mem_recall/shared_store/shared_recall"
                },
                "value": {
                    "description": "Value to store (any JSON) for mem_store/shared_store"
                },
                "scope": {
                    "type": "string",
                    "description": "Scope for shared memory: 'global', 'team:<name>', or 'agent:<uuid>'"
                },
                "event_type": {
                    "type": "string",
                    "description": "Event type string for publish"
                },
                "payload": {
                    "description": "Event payload (any JSON) for publish"
                }
            }
        })
    }

    // FIXME: don't write this like match.
    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let action: SyscallParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid kernel tool params: {e}"))?;

        match action {
            SyscallParams::Spawn { agent, task } => self.exec_spawn(&agent, &task).await,
            SyscallParams::SpawnParallel {
                parallel,
                max_concurrency: _,
            } => self.exec_spawn_parallel(parallel).await,
            SyscallParams::Status { target } => self.exec_status(&target).await,
            SyscallParams::Children => self.exec_children().await,
            SyscallParams::Kill { target } => self.exec_signal(&target, "kill").await,
            SyscallParams::Pause { target } => self.exec_signal(&target, "pause").await,
            SyscallParams::Resume { target } => self.exec_signal(&target, "resume").await,
            SyscallParams::MemStore { key, value } => self.exec_mem_store(&key, value).await,
            SyscallParams::MemRecall { key } => self.exec_mem_recall(&key).await,
            SyscallParams::SharedStore { scope, key, value } => {
                self.exec_shared_store(&scope, &key, value).await
            }
            SyscallParams::SharedRecall { scope, key } => {
                self.exec_shared_recall(&scope, &key).await
            }
            SyscallParams::Publish {
                event_type,
                payload,
            } => self.exec_publish(&event_type, payload).await,
        }
    }
}
