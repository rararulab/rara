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

//! ProcessHandle — the thin "syscall" interface for agents.
//!
//! Each agent process receives a [`ProcessHandle`] that routes all
//! interactions through `KernelEvent::Syscall` variants via the unified
//! event queue. The kernel event loop handles all business logic.

pub mod process_handle;
pub mod spawn_tool;

use tokio::sync::oneshot;

use crate::process::{AgentId, AgentResult};

/// Handle returned from spawn — allows waiting for agent completion.
///
/// Holds the spawned agent's ID and a oneshot receiver that resolves when
/// the agent finishes execution (successfully or with failure).
pub struct AgentHandle {
    /// The ID of the spawned agent process.
    pub agent_id:  AgentId,
    /// Receiver for the agent's result. Resolves when the agent finishes.
    pub result_rx: oneshot::Receiver<AgentResult>,
}

impl std::fmt::Debug for AgentHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentHandle")
            .field("agent_id", &self.agent_id)
            .field("result_rx", &"<oneshot::Receiver>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_handle_creation() {
        let (result_tx, result_rx) = oneshot::channel();
        let id = AgentId::new();
        let handle = AgentHandle {
            agent_id: id,
            result_rx,
        };
        assert_eq!(handle.agent_id, id);

        // Send a result through the channel
        let result = AgentResult {
            output:     "done".to_string(),
            iterations: 3,
            tool_calls: 1,
        };
        result_tx.send(result).unwrap();
    }
}
