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

//! OS process model — core types for the unified agent lifecycle.
//!
//! This module implements an OS-inspired process model where:
//! - [`AgentManifest`] = the "binary" (static definition, YAML-loadable)
//! - [`AgentProcess`] = a running instance in the [`ProcessTable`]
//! - [`AgentId`] = unique per-execution identifier (like a PID)
//! - [`SessionId`] = persistent conversation identifier (survives restarts)
//!
//! The [`ProcessTable`] is a concurrent in-memory table (backed by `DashMap`)
//! that tracks all running agent processes, supporting process tree queries
//! (parent/children) and state transitions.

pub mod manifest_loader;
pub mod principal;
pub mod user;

use std::collections::HashMap;

use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{error::Result, io::types::InboundMessage};

/// Unique identifier for a running agent process.
///
/// Each spawned agent receives a fresh `AgentId` (UUID v4). This is analogous
/// to a PID in operating systems — it identifies one specific execution, not
/// the agent definition itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    /// Generate a new random agent ID.
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

impl Default for AgentId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}

/// Persistent session identifier (survives across agent process restarts).
///
/// A session groups related conversations and memory. Multiple agent processes
/// can share the same session (e.g., a chat session that spawns sub-agents).
///
/// This is a type alias for [`SessionKey`](crate::session::SessionKey) to
/// minimize churn — existing code using `SessionId::new("...")` still
/// compiles because `SessionKey` provides the same constructor.
pub type SessionId = crate::session::SessionKey;

/// Agent "binary" — static definition, loadable from YAML or constructed
/// dynamically.
///
/// An `AgentManifest` defines *what* an agent is (its model, tools, prompt,
/// limits) but not *who* runs it or *when*. It is analogous to an executable
/// file on disk.
///
/// # YAML example
/// ```yaml
/// name: scout
/// description: "Fast codebase recon"
/// model: "deepseek/deepseek-chat"
/// system_prompt: "You are a scout agent..."
/// tools:
///   - read_file
///   - grep
/// max_iterations: 15
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Unique name identifying this agent definition.
    pub name:           String,
    /// Human-readable description.
    pub description:    String,
    /// LLM model identifier (e.g., "deepseek/deepseek-chat", "gpt-4").
    pub model:          String,
    /// System prompt defining agent behavior.
    pub system_prompt:  String,
    /// Optional hint for provider selection.
    #[serde(default)]
    pub provider_hint:  Option<String>,
    /// Maximum LLM iterations before forced completion.
    #[serde(default)]
    pub max_iterations: Option<usize>,
    /// Tool names this agent is allowed to use (empty = inherit parent's
    /// tools).
    #[serde(default)]
    pub tools:          Vec<String>,
    /// Maximum number of concurrent child agents this agent can spawn.
    #[serde(default)]
    pub max_children:   Option<usize>,
    /// Arbitrary metadata for extension.
    #[serde(default)]
    pub metadata:       serde_json::Value,
}

/// Runtime state of an agent process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProcessState {
    /// Agent is actively running (LLM loop in progress).
    Running,
    /// Agent is waiting for child agent results (mailbox still open).
    Waiting,
    /// Agent completed successfully.
    Completed,
    /// Agent failed with an error.
    Failed,
    /// Agent was cancelled (killed by parent or timeout).
    Cancelled,
}

/// Result of a completed agent process.
#[derive(Debug, Clone, Serialize)]
pub struct AgentResult {
    /// The agent's final output text.
    pub output:     String,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Number of tool calls made.
    pub tool_calls: usize,
}

/// Process environment — isolated per-agent context.
///
/// Provides workspace path and environment variables that are scoped to
/// a specific agent process.
#[derive(Debug, Clone, Serialize)]
pub struct AgentEnv {
    /// Optional workspace directory for file operations.
    pub workspace: Option<String>,
    /// Key-value environment variables.
    pub vars:      HashMap<String, String>,
}

impl Default for AgentEnv {
    fn default() -> Self {
        Self {
            workspace: None,
            vars:      HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ProcessMessage / Signal — mailbox message types
// ---------------------------------------------------------------------------

/// Messages delivered to a long-lived agent process via its mailbox.
#[derive(Debug)]
pub enum ProcessMessage {
    /// A new user message to process.
    UserMessage(InboundMessage),
    /// Result from a spawned child agent.
    ChildResult {
        child_id: AgentId,
        result:   AgentResult,
    },
    /// Control signal.
    Signal(Signal),
}

/// Control signals for agent processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Interrupt the current operation (cancel in-flight LLM call).
    Interrupt,
}

/// A running agent instance in the process table.
///
/// This is the runtime counterpart to [`AgentManifest`]. Each time an agent
/// is spawned, a new `AgentProcess` is created with a unique [`AgentId`],
/// the spawning principal, and the manifest that defines its behavior.
#[derive(Debug, Clone)]
pub struct AgentProcess {
    /// Unique identifier for this process.
    pub agent_id:    AgentId,
    /// Parent process (None for root-level agents).
    pub parent_id:   Option<AgentId>,
    /// Session this process belongs to.
    pub session_id:  SessionId,
    /// The agent definition driving this process.
    pub manifest:    AgentManifest,
    /// The identity under which this process runs.
    pub principal:   principal::Principal,
    /// Per-process environment.
    pub env:         AgentEnv,
    /// Current lifecycle state.
    pub state:       ProcessState,
    /// When this process was created.
    pub created_at:  Timestamp,
    /// When this process finished (if terminal).
    pub finished_at: Option<Timestamp>,
    /// Result of execution (set on completion/failure).
    pub result:      Option<AgentResult>,
}

/// Summary info for listing processes.
///
/// A lightweight view of an [`AgentProcess`] suitable for display in
/// process listings without exposing full internal state.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub agent_id:   AgentId,
    pub parent_id:  Option<AgentId>,
    pub name:       String,
    pub state:      ProcessState,
    pub created_at: Timestamp,
}

impl From<&AgentProcess> for ProcessInfo {
    fn from(p: &AgentProcess) -> Self {
        Self {
            agent_id:   p.agent_id,
            parent_id:  p.parent_id,
            name:       p.manifest.name.clone(),
            state:      p.state,
            created_at: p.created_at,
        }
    }
}

/// In-memory process table — the kernel's view of all running agents.
///
/// Thread-safe via `DashMap`. Supports concurrent reads and writes from
/// multiple tokio tasks (e.g., kernel spawn + agent tool calls).
///
/// Includes a session index for fast `SessionId -> AgentId` lookups and
/// a mailbox registry for sending messages to long-lived processes.
pub struct ProcessTable {
    processes:     DashMap<AgentId, AgentProcess>,
    /// Maps a session to its currently active agent process.
    session_index: DashMap<SessionId, AgentId>,
    /// Mailbox senders for long-lived processes (kept separate because
    /// `mpsc::Sender` doesn't derive `Clone` for `AgentProcess`'s derive).
    mailboxes:     DashMap<AgentId, mpsc::Sender<ProcessMessage>>,
    /// Cancellation tokens for graceful process termination.
    /// Parent token cancel → all child tokens cancel automatically.
    cancellation_tokens: DashMap<AgentId, CancellationToken>,
}

impl ProcessTable {
    /// Create an empty process table.
    pub fn new() -> Self {
        Self {
            processes:     DashMap::new(),
            session_index: DashMap::new(),
            mailboxes:     DashMap::new(),
            cancellation_tokens: DashMap::new(),
        }
    }

    /// Insert a process into the table.
    ///
    /// Automatically updates the session index so `find_by_session` can
    /// locate this process.
    pub fn insert(&self, process: AgentProcess) {
        self.session_index
            .insert(process.session_id.clone(), process.agent_id);
        self.processes.insert(process.agent_id, process);
    }

    /// Get a clone of a process by ID.
    pub fn get(&self, id: AgentId) -> Option<AgentProcess> {
        self.processes.get(&id).map(|p| p.value().clone())
    }

    /// Transition a process to a new state.
    ///
    /// Automatically sets `finished_at` when transitioning to a terminal state.
    pub fn set_state(&self, id: AgentId, state: ProcessState) -> Result<()> {
        let mut entry = self
            .processes
            .get_mut(&id)
            .ok_or(crate::error::KernelError::AgentNotFound { id: id.0 })?;
        entry.state = state;
        match state {
            ProcessState::Completed | ProcessState::Failed | ProcessState::Cancelled => {
                entry.finished_at = Some(Timestamp::now());
            }
            ProcessState::Running | ProcessState::Waiting => {
                // Non-terminal states: do not set finished_at.
            }
        }
        Ok(())
    }

    /// Set the result of a process.
    pub fn set_result(&self, id: AgentId, result: AgentResult) -> Result<()> {
        let mut entry = self
            .processes
            .get_mut(&id)
            .ok_or(crate::error::KernelError::AgentNotFound { id: id.0 })?;
        entry.result = Some(result);
        Ok(())
    }

    /// Remove a process from the table, returning it if it existed.
    ///
    /// Also cleans up the session index and mailbox entries.
    pub fn remove(&self, id: AgentId) -> Option<AgentProcess> {
        let removed = self.processes.remove(&id).map(|(_, p)| p);
        if let Some(ref process) = removed {
            self.session_index
                .remove_if(&process.session_id, |_, agent_id| *agent_id == id);
            self.mailboxes.remove(&id);
            self.cancellation_tokens.remove(&id);
        }
        removed
    }

    /// List all children of a given parent.
    pub fn children_of(&self, parent_id: AgentId) -> Vec<ProcessInfo> {
        self.processes
            .iter()
            .filter(|p| p.parent_id == Some(parent_id))
            .map(|p| ProcessInfo::from(p.value()))
            .collect()
    }

    /// List all processes.
    pub fn list(&self) -> Vec<ProcessInfo> {
        self.processes
            .iter()
            .map(|p| ProcessInfo::from(p.value()))
            .collect()
    }

    /// Count running processes.
    pub fn running_count(&self) -> usize {
        self.processes
            .iter()
            .filter(|p| p.state == ProcessState::Running)
            .count()
    }

    // ----- Session index methods -----

    /// Find the active agent process for a session.
    pub fn find_by_session(&self, session_id: &SessionId) -> Option<AgentProcess> {
        let agent_id = self.session_index.get(session_id)?;
        self.get(*agent_id)
    }

    /// Bind a session to a specific agent process (overwrites any existing
    /// binding).
    pub fn bind_session(&self, session_id: SessionId, agent_id: AgentId) {
        self.session_index.insert(session_id, agent_id);
    }

    // ----- Mailbox methods -----

    /// Register a mailbox sender for a process.
    pub fn set_mailbox(&self, id: AgentId, tx: mpsc::Sender<ProcessMessage>) {
        self.mailboxes.insert(id, tx);
    }

    /// Get a clone of the mailbox sender for a process.
    pub fn get_mailbox(&self, id: &AgentId) -> Option<mpsc::Sender<ProcessMessage>> {
        self.mailboxes.get(id).map(|tx| tx.value().clone())
    }

    /// Register a cancellation token for a process.
    pub fn set_cancellation_token(&self, id: AgentId, token: CancellationToken) {
        self.cancellation_tokens.insert(id, token);
    }

    /// Get a clone of the cancellation token for a process.
    pub fn get_cancellation_token(&self, id: &AgentId) -> Option<CancellationToken> {
        self.cancellation_tokens.get(id).map(|t| t.value().clone())
    }

    /// Remove the cancellation token for a process that ended naturally.
    pub fn clear_cancellation_token(&self, id: &AgentId) {
        self.cancellation_tokens.remove(id);
    }

    /// Send a message to the agent process handling a given session.
    ///
    /// Returns `Ok(())` if the message was sent, or an error if no agent
    /// is bound to the session or the mailbox is closed.
    pub async fn send_to_session(&self, session_id: &SessionId, msg: ProcessMessage) -> Result<()> {
        let agent_id = self
            .session_index
            .get(session_id)
            .map(|r| *r)
            .ok_or_else(|| crate::error::KernelError::ProcessNotFound {
                id: format!("no agent for session {session_id}"),
            })?;
        let tx = self.get_mailbox(&agent_id).ok_or_else(|| {
            crate::error::KernelError::ProcessNotFound {
                id: format!("no mailbox for agent {agent_id}"),
            }
        })?;
        tx.send(msg)
            .await
            .map_err(|_| crate::error::KernelError::ProcessNotFound {
                id: format!("mailbox closed for agent {agent_id}"),
            })
    }
}

impl Default for ProcessTable {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::principal::Principal;

    /// Helper to create a test manifest.
    fn test_manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name:           name.to_string(),
            description:    format!("Test agent: {name}"),
            model:          "test-model".to_string(),
            system_prompt:  "You are a test agent.".to_string(),
            provider_hint:  None,
            max_iterations: Some(10),
            tools:          vec!["read_file".to_string()],
            max_children:   None,
            metadata:       serde_json::Value::Null,
        }
    }

    /// Helper to create a test process.
    fn test_process(name: &str, parent_id: Option<AgentId>) -> AgentProcess {
        AgentProcess {
            agent_id: AgentId::new(),
            parent_id,
            session_id: SessionId::new("test-session"),
            manifest: test_manifest(name),
            principal: Principal::user("test-user"),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
        }
    }

    #[test]
    fn test_agent_id_display() {
        let id = AgentId::new();
        let display = id.to_string();
        // UUID format: 8-4-4-4-12
        assert_eq!(display.len(), 36);
        assert!(display.contains('-'));
    }

    #[test]
    fn test_session_id() {
        let sid = SessionId::new("my-session");
        assert_eq!(sid.to_string(), "my-session");
        assert_eq!(sid.as_str(), "my-session");
    }

    #[test]
    fn test_process_table_insert_get() {
        let table = ProcessTable::new();
        let process = test_process("scout", None);
        let id = process.agent_id;

        table.insert(process);

        let retrieved = table.get(id).unwrap();
        assert_eq!(retrieved.agent_id, id);
        assert_eq!(retrieved.manifest.name, "scout");
        assert_eq!(retrieved.state, ProcessState::Running);
    }

    #[test]
    fn test_process_table_get_nonexistent() {
        let table = ProcessTable::new();
        assert!(table.get(AgentId::new()).is_none());
    }

    #[test]
    fn test_process_table_set_state() {
        let table = ProcessTable::new();
        let process = test_process("scout", None);
        let id = process.agent_id;
        table.insert(process);

        // Transition to Completed
        table.set_state(id, ProcessState::Completed).unwrap();

        let p = table.get(id).unwrap();
        assert_eq!(p.state, ProcessState::Completed);
        assert!(p.finished_at.is_some());
    }

    #[test]
    fn test_process_table_set_state_nonexistent() {
        let table = ProcessTable::new();
        let result = table.set_state(AgentId::new(), ProcessState::Failed);
        assert!(result.is_err());
    }

    #[test]
    fn test_process_table_set_result() {
        let table = ProcessTable::new();
        let process = test_process("worker", None);
        let id = process.agent_id;
        table.insert(process);

        let result = AgentResult {
            output:     "done".to_string(),
            iterations: 5,
            tool_calls: 3,
        };
        table.set_result(id, result).unwrap();

        let p = table.get(id).unwrap();
        let r = p.result.unwrap();
        assert_eq!(r.output, "done");
        assert_eq!(r.iterations, 5);
        assert_eq!(r.tool_calls, 3);
    }

    #[test]
    fn test_process_table_remove() {
        let table = ProcessTable::new();
        let process = test_process("scout", None);
        let id = process.agent_id;
        table.insert(process);

        let removed = table.remove(id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().agent_id, id);

        // Should be gone now
        assert!(table.get(id).is_none());
    }

    #[test]
    fn test_process_table_remove_nonexistent() {
        let table = ProcessTable::new();
        assert!(table.remove(AgentId::new()).is_none());
    }

    #[test]
    fn test_process_table_children_of() {
        let table = ProcessTable::new();

        let parent = test_process("planner", None);
        let parent_id = parent.agent_id;
        table.insert(parent);

        let child1 = test_process("worker-1", Some(parent_id));
        let child1_id = child1.agent_id;
        table.insert(child1);

        let child2 = test_process("worker-2", Some(parent_id));
        let child2_id = child2.agent_id;
        table.insert(child2);

        // Unrelated process (no parent)
        let unrelated = test_process("scout", None);
        table.insert(unrelated);

        let children = table.children_of(parent_id);
        assert_eq!(children.len(), 2);

        let child_ids: Vec<AgentId> = children.iter().map(|c| c.agent_id).collect();
        assert!(child_ids.contains(&child1_id));
        assert!(child_ids.contains(&child2_id));
    }

    #[test]
    fn test_process_table_list() {
        let table = ProcessTable::new();
        assert!(table.list().is_empty());

        table.insert(test_process("a", None));
        table.insert(test_process("b", None));
        table.insert(test_process("c", None));

        assert_eq!(table.list().len(), 3);
    }

    #[test]
    fn test_process_table_running_count() {
        let table = ProcessTable::new();
        assert_eq!(table.running_count(), 0);

        let p1 = test_process("a", None);
        let p1_id = p1.agent_id;
        table.insert(p1);

        let p2 = test_process("b", None);
        table.insert(p2);

        assert_eq!(table.running_count(), 2);

        table.set_state(p1_id, ProcessState::Completed).unwrap();
        assert_eq!(table.running_count(), 1);
    }

    #[test]
    fn test_process_info_from_agent_process() {
        let process = test_process("scout", None);
        let info = ProcessInfo::from(&process);

        assert_eq!(info.agent_id, process.agent_id);
        assert_eq!(info.parent_id, None);
        assert_eq!(info.name, "scout");
        assert_eq!(info.state, ProcessState::Running);
    }

    #[test]
    fn test_agent_manifest_yaml_roundtrip() {
        let manifest = test_manifest("roundtrip");
        let yaml = serde_yaml::to_string(&manifest).unwrap();
        let deserialized: AgentManifest = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(deserialized.name, "roundtrip");
        assert_eq!(deserialized.model, "test-model");
        assert_eq!(deserialized.max_iterations, Some(10));
        assert_eq!(deserialized.tools, vec!["read_file"]);
    }

    #[test]
    fn test_agent_manifest_yaml_minimal() {
        let yaml = r#"
name: minimal
description: "Minimal agent"
model: "gpt-4"
system_prompt: "Hello"
"#;
        let m: AgentManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.name, "minimal");
        assert!(m.tools.is_empty());
        assert!(m.max_iterations.is_none());
        assert!(m.provider_hint.is_none());
        assert!(m.max_children.is_none());
    }

    #[test]
    fn test_agent_env_default() {
        let env = AgentEnv::default();
        assert!(env.workspace.is_none());
        assert!(env.vars.is_empty());
    }

    #[test]
    fn test_process_state_terminal_sets_finished_at() {
        let table = ProcessTable::new();

        // Test Failed
        let p = test_process("a", None);
        let id = p.agent_id;
        table.insert(p);
        table.set_state(id, ProcessState::Failed).unwrap();
        assert!(table.get(id).unwrap().finished_at.is_some());

        // Test Cancelled
        let p = test_process("b", None);
        let id = p.agent_id;
        table.insert(p);
        table.set_state(id, ProcessState::Cancelled).unwrap();
        assert!(table.get(id).unwrap().finished_at.is_some());
    }

    #[test]
    fn test_process_state_waiting_does_not_set_finished_at() {
        let table = ProcessTable::new();
        let p = test_process("waiter", None);
        let id = p.agent_id;
        table.insert(p);

        table.set_state(id, ProcessState::Waiting).unwrap();

        let process = table.get(id).unwrap();
        assert_eq!(process.state, ProcessState::Waiting);
        assert!(
            process.finished_at.is_none(),
            "Waiting state should not set finished_at"
        );
    }

    #[test]
    fn test_process_table_session_index() {
        let table = ProcessTable::new();

        // Insert creates a session index entry
        let p = test_process("agent-a", None);
        let agent_id = p.agent_id;
        let session_id = p.session_id.clone();
        table.insert(p);

        // find_by_session should return the process
        let found = table.find_by_session(&session_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().agent_id, agent_id);

        // bind_session overwrites
        let new_id = AgentId::new();
        let new_process = AgentProcess {
            agent_id:    new_id,
            parent_id:   None,
            session_id:  session_id.clone(),
            manifest:    test_manifest("agent-b"),
            principal:   Principal::user("test-user"),
            env:         AgentEnv::default(),
            state:       ProcessState::Running,
            created_at:  Timestamp::now(),
            finished_at: None,
            result:      None,
        };
        table.insert(new_process);
        table.bind_session(session_id.clone(), new_id);

        let found = table.find_by_session(&session_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().agent_id, new_id);
    }

    #[test]
    fn test_process_table_remove_clears_session_index() {
        let table = ProcessTable::new();
        let p = test_process("removable", None);
        let agent_id = p.agent_id;
        let session_id = p.session_id.clone();
        table.insert(p);

        // Session index should have an entry
        assert!(table.find_by_session(&session_id).is_some());

        // Remove should clear the session index
        table.remove(agent_id);
        assert!(table.find_by_session(&session_id).is_none());
    }

    #[test]
    fn test_process_table_mailbox() {
        let table = ProcessTable::new();
        let p = test_process("mailbox-test", None);
        let agent_id = p.agent_id;
        table.insert(p);

        // Initially no mailbox
        assert!(table.get_mailbox(&agent_id).is_none());

        // Set mailbox
        let (tx, _rx) = mpsc::channel(16);
        table.set_mailbox(agent_id, tx);

        // Now we should get it
        assert!(table.get_mailbox(&agent_id).is_some());

        // Remove clears mailbox too
        table.remove(agent_id);
        assert!(table.get_mailbox(&agent_id).is_none());
    }
}
