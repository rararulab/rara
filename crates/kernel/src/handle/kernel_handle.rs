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

use std::sync::Arc;

use jiff::Timestamp;
use tokio::sync::Semaphore;

use crate::{
    audit::{AuditEvent, AuditFilter, AuditRef},
    device::DeviceRegistryRef,
    error::{KernelError, Result},
    event::KernelEvent,
    io::{
        egress::EndpointRegistryRef,
        ingress::{IngressPipelineRef, RawPlatformMessage},
        stream::StreamHubRef,
        types::{InboundMessage, IngestError},
    },
    kernel::{KernelConfig, SettingsRef},
    process::{
        AgentId, AgentManifest, ProcessState, ProcessTable, Signal,
        agent_registry::AgentRegistryRef, principal::Principal,
    },
    queue::EventQueueRef,
    security::SecurityRef,
    tool::ToolRegistryRef,
};

// FIXME: why kernel this complicated ?
/// Public entry point for interacting with the kernel.
///
/// Provides both mutation methods (spawn, signal, shutdown) that flow through
/// the event queue, and read-only accessors for kernel subsystems.
///
/// Cheap to clone (all fields are `Arc`s). External callers should prefer
/// `KernelHandle` over `Arc<Kernel>`.
///
/// # Usage
///
/// Obtain a `KernelHandle` via
/// [`Kernel::handle()`](crate::kernel::Kernel::handle):
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
    event_queue:       EventQueueRef,
    /// Agent registry for resolving named agents to manifests.
    agent_registry:    AgentRegistryRef,
    /// The global process table tracking all running agents.
    process_table:     Arc<ProcessTable>,
    /// Ingress pipeline for adapters to push inbound messages.
    ingress_pipeline:  IngressPipelineRef,
    /// Ephemeral stream hub for real-time token deltas.
    stream_hub:        StreamHubRef,
    /// Per-user endpoint registry (tracks connected channels).
    endpoint_registry: EndpointRegistryRef,
    /// Unified audit subsystem (logging + tool call recording).
    audit:             AuditRef,
    /// Flat KV settings provider for runtime configuration.
    settings:          SettingsRef,
    /// Unified security subsystem (auth + authz + approval + guard).
    security:          SecurityRef,
    /// Kernel configuration.
    config:            KernelConfig,
    /// Global tool registry.
    tool_registry:     ToolRegistryRef,
    /// Device registry for hot-pluggable devices.
    device_registry:   DeviceRegistryRef,
    /// Global semaphore limiting total concurrent agent processes.
    global_semaphore:  Arc<Semaphore>,
    /// When the kernel was created (for uptime calculation).
    started_at:        Timestamp,
}

impl KernelHandle {
    /// Create a new `KernelHandle`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        event_queue: EventQueueRef,
        agent_registry: AgentRegistryRef,
        process_table: Arc<ProcessTable>,
        ingress_pipeline: IngressPipelineRef,
        stream_hub: StreamHubRef,
        endpoint_registry: EndpointRegistryRef,
        audit: AuditRef,
        settings: SettingsRef,
        security: SecurityRef,
        config: KernelConfig,
        tool_registry: ToolRegistryRef,
        device_registry: DeviceRegistryRef,
        global_semaphore: Arc<Semaphore>,
        started_at: Timestamp,
    ) -> Self {
        Self {
            event_queue,
            agent_registry,
            process_table,
            ingress_pipeline,
            stream_hub,
            endpoint_registry,
            audit,
            settings,
            security,
            config,
            tool_registry,
            device_registry,
            global_semaphore,
            started_at,
        }
    }

    // -- Mutation methods (flow through event queue) -------------------------

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

    /// Ingest a raw platform message: resolve identity + session, then push
    /// the resulting [`InboundMessage`] into the event queue.
    ///
    /// This is the primary entry point for channel adapters.
    pub async fn ingest(&self, raw: RawPlatformMessage) -> std::result::Result<(), IngestError> {
        let msg = self.ingress_pipeline.resolve(raw).await?;
        let channel_label = format!("{:?}", msg.source.channel_type);

        self.submit_message(msg)
            .map_err(|_| IngestError::SystemBusy)?;

        crate::metrics::MESSAGE_INBOUND
            .with_label_values(&[&channel_label])
            .inc();

        Ok(())
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

    // -- Read-only accessors ------------------------------------------------

    /// Access the process table for querying.
    pub fn process_table(&self) -> &Arc<ProcessTable> { &self.process_table }

    /// Access the ingress pipeline (resolution layer).
    pub fn ingress_pipeline(&self) -> &IngressPipelineRef { &self.ingress_pipeline }

    /// Access the ephemeral stream hub (WebAdapter needs this for token
    /// deltas).
    pub fn stream_hub(&self) -> &StreamHubRef { &self.stream_hub }

    /// Access the endpoint registry (WebAdapter needs this for connection
    /// tracking).
    pub fn endpoint_registry(&self) -> &EndpointRegistryRef { &self.endpoint_registry }

    /// Access the agent registry for looking up named manifests.
    pub fn agent_registry(&self) -> &AgentRegistryRef { &self.agent_registry }

    /// Access the tool registry.
    pub fn tool_registry(&self) -> &ToolRegistryRef { &self.tool_registry }

    /// Access the unified audit subsystem.
    pub fn audit(&self) -> &AuditRef { &self.audit }

    /// Access the flat KV settings provider.
    pub fn settings(&self) -> &SettingsRef { &self.settings }

    /// Access the unified security subsystem.
    pub fn security(&self) -> &SecurityRef { &self.security }

    /// Access the kernel config.
    pub fn config(&self) -> &KernelConfig { &self.config }

    /// Access the unified event queue.
    pub fn event_queue(&self) -> &EventQueueRef { &self.event_queue }

    /// Access the device registry (for hot-plugging devices).
    pub fn device_registry(&self) -> &DeviceRegistryRef { &self.device_registry }

    // -- Query methods ------------------------------------------------------

    /// Get detailed runtime statistics for a single process.
    ///
    /// Returns `None` if the process does not exist.
    pub async fn process_stats(&self, agent_id: &AgentId) -> Option<crate::process::ProcessStats> {
        self.process_table.process_stats(*agent_id).await
    }

    /// List detailed runtime statistics for all processes.
    pub async fn list_processes(&self) -> Vec<crate::process::ProcessStats> {
        self.process_table.all_process_stats().await
    }

    /// Get kernel-wide aggregate statistics.
    pub fn system_stats(&self) -> crate::process::SystemStats {
        let pt = &self.process_table;
        let active = pt
            .list()
            .iter()
            .filter(|p| {
                matches!(
                    p.state,
                    ProcessState::Running | ProcessState::Idle | ProcessState::Waiting
                )
            })
            .count();

        let uptime_ms = Timestamp::now()
            .since(self.started_at)
            .ok()
            .map(|span| span.get_milliseconds().unsigned_abs())
            .unwrap_or(0);

        crate::process::SystemStats {
            active_processes: active,
            total_spawned: pt.total_spawned(),
            total_completed: pt.total_completed(),
            total_failed: pt.total_failed(),
            global_semaphore_available: self.global_semaphore.available_permits(),
            total_tokens_consumed: pt.total_tokens_consumed(),
            uptime_ms,
        }
    }

    /// Get the detailed turn traces for a specific agent process.
    pub fn get_process_turns(&self, agent_id: AgentId) -> Vec<crate::agent_turn::TurnTrace> {
        self.process_table.get_turn_traces(agent_id)
    }

    /// Query the audit log for events matching the given filter.
    pub async fn audit_query(&self, filter: AuditFilter) -> Vec<AuditEvent> {
        self.audit.query(filter).await
    }
}

impl std::fmt::Debug for KernelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KernelHandle")
            .field("event_queue_pending", &self.event_queue.pending_count())
            .finish()
    }
}
