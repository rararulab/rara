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

//! KernelHandle — public entry point for interacting with the kernel.
//!
//! All mutations flow through the event queue — `KernelHandle` never
//! accesses internal kernel state directly. This is the external API
//! counterpart to [`ProcessHandle`](super::process_handle::ProcessHandle),
//! which is the per-process internal handle.

use crate::{
    error::{KernelError, Result},
    event_queue::EventQueueRef,
    io::types::InboundMessage,
    process::{
        AgentId, AgentManifest, Signal,
        agent_registry::AgentRegistryRef,
        principal::Principal,
    },
    unified_event::KernelEvent,
};

/// Public entry point for interacting with the kernel.
///
/// All mutations flow through the event queue — `KernelHandle` never
/// accesses internal kernel state directly. Cheap to clone (two `Arc`s).
///
/// # Usage
///
/// Obtain a `KernelHandle` via [`Kernel::handle()`](crate::kernel::Kernel::handle):
///
/// ```ignore
/// let handle = kernel.handle();
/// let agent_id = handle.spawn_with_input(manifest, "hello".into(), principal, None).await?;
/// handle.send_signal(agent_id, Signal::Pause)?;
/// handle.shutdown()?;
/// ```
#[derive(Clone)]
pub struct KernelHandle {
    /// Core: the unified event queue sender.
    event_queue: EventQueueRef,
    /// Agent registry for resolving named agents to manifests.
    agent_registry: AgentRegistryRef,
}

impl KernelHandle {
    /// Create a new `KernelHandle`.
    pub(crate) fn new(event_queue: EventQueueRef, agent_registry: AgentRegistryRef) -> Self {
        Self {
            event_queue,
            agent_registry,
        }
    }

    /// Spawn a new agent process via the unified event queue.
    ///
    /// Pushes a `KernelEvent::SpawnAgent` into the event queue and waits
    /// for the reply. The kernel generates a fresh isolated session for
    /// the new process.
    #[tracing::instrument(skip_all, fields(manifest_name = %manifest.name))]
    pub async fn spawn_with_input(
        &self,
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        parent_id: Option<AgentId>,
    ) -> Result<AgentId> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let event = KernelEvent::SpawnAgent {
            manifest,
            input,
            principal,
            parent_id,
            reply_tx,
        };
        self.event_queue
            .push(event)
            .await
            .map_err(|_| KernelError::SpawnFailed {
                message: "event queue full".to_string(),
            })?;

        reply_rx.await.map_err(|_| KernelError::SpawnFailed {
            message: "spawn reply channel closed".to_string(),
        })?
    }

    /// Spawn a named agent by looking up its manifest in the agent registry.
    #[tracing::instrument(skip(self, input, principal, parent_id))]
    pub async fn spawn_named(
        &self,
        agent_name: &str,
        input: String,
        principal: Principal,
        parent_id: Option<AgentId>,
    ) -> Result<AgentId> {
        let manifest =
            self.agent_registry
                .get(agent_name)
                .ok_or(KernelError::ManifestNotFound {
                    name: agent_name.to_string(),
                })?;

        self.spawn_with_input(manifest, input, principal, parent_id)
            .await
    }

    /// Send a control signal to an agent process (fire-and-forget).
    ///
    /// Uses `try_push` (non-async) so this can be called from synchronous
    /// contexts.
    pub fn send_signal(&self, target: AgentId, signal: Signal) -> Result<()> {
        self.event_queue
            .try_push(KernelEvent::SendSignal { target, signal })
            .map_err(|_| KernelError::Other {
                message: "event queue full for signal".into(),
            })
    }

    /// Submit an inbound user message (fire-and-forget).
    ///
    /// Uses `try_push` (non-async) so this can be called from synchronous
    /// contexts.
    pub fn submit_message(&self, msg: InboundMessage) -> Result<()> {
        self.event_queue
            .try_push(KernelEvent::UserMessage(msg))
            .map_err(|_| KernelError::Other {
                message: "event queue full for user message".into(),
            })
    }

    /// Request a graceful kernel shutdown (fire-and-forget).
    ///
    /// Uses `try_push` (non-async) so this can be called from synchronous
    /// contexts.
    pub fn shutdown(&self) -> Result<()> {
        self.event_queue
            .try_push(KernelEvent::Shutdown)
            .map_err(|_| KernelError::Other {
                message: "event queue full for shutdown".into(),
            })
    }
}

impl std::fmt::Debug for KernelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KernelHandle")
            .field("event_queue_pending", &self.event_queue.pending_count())
            .finish()
    }
}
