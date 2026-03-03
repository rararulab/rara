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

//! Kernel — the unified OS-inspired orchestrator for agent lifecycle.
//!
//! The [`Kernel`] is the single entry point for all agent operations.
//! It manages a [`ProcessTable`] of running agents, enforces concurrency
//! limits via dual semaphores (global + per-agent), and provides
//! [`spawn`](Kernel::spawn) as the primary API for creating agent processes.
//!
//! # Architecture
//!
//! ```text
//! Kernel (top-level, behind Arc after start())
//!   ├── ProcessTable  (all running agents)
//!   ├── global_semaphore (max total concurrent agents)
//!   ├── AgentRegistry   (named agent definitions)
//!   ├── DriverRegistry  (multi-driver LLM)
//!   ├── ToolRegistry
//!   ├── Memory
//!   ├── EventBus
//!   ├── SecuritySubsystem (auth + authz + approval + guard)
//!   ├── shared_kv (cross-agent KV)
//!   ├── AuditSubsystem
//!   ├── StreamHub + IngressPipeline + EndpointRegistry
//!   └── EventQueue (sharded)
//! ```
//!
//! Each spawned agent receives a [`ProcessHandle`] — a thin event pusher that
//! sends [`Syscall`] variants through the unified event queue.

use std::{collections::HashMap, sync::Arc};

use jiff::Timestamp;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    audit::{AuditEvent, AuditFilter, AuditLog},
    audit_subsystem::AuditRef,
    channel::types::ChannelType,
    device_registry::{DeviceRegistry, DeviceRegistryRef},
    event::EventBusRef,
    event_queue::EventQueueRef,
    io::{
        egress::{EgressAdapterRef, EndpointRegistry, EndpointRegistryRef},
        ingress::{IdentityResolverRef, IngressPipeline, IngressPipelineRef, SessionResolverRef},
        pipe::PipeRegistry,
        stream::StreamHubRef,
    },
    kv::KvBackendRef,
    llm::DriverRegistryRef,
    memory::MemoryRef,
    process::{AgentId, ProcessState, ProcessTable, SessionId, agent_registry::AgentRegistryRef},
    security::SecurityRef,
    session::SessionRepoRef,
    sharded_event_queue::ShardedQueueRef,
    tool::ToolRegistryRef,
};

// ---------------------------------------------------------------------------
// KernelConfig
// ---------------------------------------------------------------------------

/// Kernel configuration.
#[derive(Debug, Clone, smart_default::SmartDefault)]
pub struct KernelConfig {
    /// Maximum number of concurrent agent processes globally.
    #[default = 16]
    pub max_concurrency:        usize,
    /// Default maximum number of children per agent.
    #[default = 8]
    pub default_child_limit:    usize,
    /// Default max LLM iterations for spawned agents.
    #[default = 25]
    pub default_max_iterations: usize,
    /// Maximum number of KV entries per agent (0 = unlimited).
    /// Applies to the agent-scoped namespace only.
    #[default = 1000]
    pub memory_quota_per_agent: usize,
}

/// Shared reference to a
/// [`SettingsProvider`](rara_domain_shared::settings::SettingsProvider).
pub type SettingsRef = Arc<dyn rara_domain_shared::settings::SettingsProvider>;

/// The unified agent orchestrator.
///
/// Acts as an OS kernel for agents: manages the process table, enforces
/// concurrency limits, and provides the event loop as the single driver
/// for all kernel activity.
///
/// The Kernel owns its I/O subsystem: stream hub, endpoint registry, and
/// ingress pipeline. Call [`start()`](Self::start) to spawn the unified
/// event loop and egress delivery as background tasks.
///
/// After [`start()`](Self::start), the Kernel lives behind `Arc<Kernel>`.
/// Fields that were previously in a separate `KernelInner` are now
/// flattened directly into this struct.
pub struct Kernel {
    /// Kernel configuration.
    config: KernelConfig,
    // -- Core subsystems (previously in KernelInner) -----------------------
    /// The global process table tracking all running agents.
    process_table: Arc<ProcessTable>,
    /// Global semaphore limiting total concurrent agent processes.
    global_semaphore: Arc<Semaphore>,
    /// Multi-driver LLM registry with per-agent overrides.
    driver_registry: DriverRegistryRef,
    /// Global tool registry (spawned agents get filtered subsets).
    tool_registry: ToolRegistryRef,
    /// 3-layer memory (not used for cross-agent KV — see shared_kv).
    memory: MemoryRef,
    /// Event bus for publishing kernel events.
    event_bus: EventBusRef,
    /// Unified security subsystem (auth + authz + approval).
    security: SecurityRef,
    /// Agent registry for looking up named agent definitions.
    agent_registry: AgentRegistryRef,
    /// Cross-agent shared key-value store (trait-based, swappable backend).
    shared_kv: KvBackendRef,
    /// Unified audit subsystem (logging + tool call recording).
    audit: AuditRef,
    /// Session repository for conversation history.
    session_repo: SessionRepoRef,
    /// Flat KV settings provider for runtime configuration.
    settings: SettingsRef,
    /// Inter-agent pipe registry for streaming data between agents.
    pipe_registry: PipeRegistry,
    /// Device registry for hot-pluggable devices (MCP servers, APIs, etc.).
    device_registry: DeviceRegistryRef,
    // -- I/O subsystem -----------------------------------------------------
    /// Ephemeral stream hub for real-time token deltas.
    stream_hub: StreamHubRef,
    /// Ingress pipeline for adapters to push inbound messages.
    ingress_pipeline: IngressPipelineRef,
    /// Per-user endpoint registry (tracks connected channels).
    endpoint_registry: EndpointRegistryRef,
    /// Registered egress adapters (mutable before start, consumed by start).
    pub(crate) egress_adapters: HashMap<ChannelType, EgressAdapterRef>,
    /// Unified event queue for all kernel interactions.
    event_queue: EventQueueRef,
    /// Sharded event queue for multi-processor event loop.
    /// The `event_queue` field points to the same object (via `Arc<dyn
    /// EventQueue>`).
    sharded_queue: ShardedQueueRef,
    /// When this kernel was created (for uptime calculation).
    started_at: Timestamp,
}

impl Kernel {
    /// Create a new Kernel with the given configuration, components, and I/O
    /// subsystem.
    ///
    /// The I/O subsystem is fully assembled at construction time. Call
    /// [`start()`](Self::start) to spawn the unified event loop.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: KernelConfig,
        driver_registry: DriverRegistryRef,
        tool_registry: ToolRegistryRef,
        memory: MemoryRef,
        event_bus: EventBusRef,
        security: SecurityRef,
        agent_registry: AgentRegistryRef,
        session_repo: SessionRepoRef,
        settings: SettingsRef,
        stream_hub: StreamHubRef,
        identity_resolver: IdentityResolverRef,
        session_resolver: SessionResolverRef,
        audit: AuditRef,
        sharded_queue: Option<ShardedQueueRef>,
        kv_backend: Option<KvBackendRef>,
    ) -> Self {
        info!(
            max_concurrency = config.max_concurrency,
            default_child_limit = config.default_child_limit,
            default_max_iterations = config.default_max_iterations,
            "booting kernel"
        );

        let endpoint_registry = Arc::new(EndpointRegistry::new());

        // Always use a ShardedEventQueue — create a default one if not provided.
        let sharded_queue = sharded_queue.unwrap_or_else(|| {
            Arc::new(crate::sharded_event_queue::ShardedEventQueue::new(
                crate::sharded_event_queue::ShardedEventQueueConfig::default(),
            ))
        });
        let event_queue: EventQueueRef = sharded_queue.clone();

        let ingress_pipeline = Arc::new(IngressPipeline::with_event_queue(
            identity_resolver,
            session_resolver,
            event_queue.clone(),
        ));

        let global_semaphore = Arc::new(Semaphore::new(config.max_concurrency));
        Self {
            config,
            process_table: Arc::new(ProcessTable::new()),
            global_semaphore,
            driver_registry,
            tool_registry,
            memory,
            event_bus,
            security,
            agent_registry,
            shared_kv: kv_backend
                .unwrap_or_else(|| Arc::new(crate::defaults::dashmap_kv::DashMapKv::new())),
            audit,
            session_repo,
            settings,
            pipe_registry: PipeRegistry::new(),
            device_registry: Arc::new(DeviceRegistry::new()),
            stream_hub,
            ingress_pipeline,
            endpoint_registry,
            egress_adapters: HashMap::new(),
            event_queue,
            sharded_queue,
            started_at: Timestamp::now(),
        }
    }

    /// Access the process table for querying.
    pub fn process_table(&self) -> &ProcessTable { &self.process_table }

    /// Access the agent registry for looking up named manifests.
    pub fn agent_registry(&self) -> &AgentRegistryRef { &self.agent_registry }

    /// Access the tool registry.
    pub fn tool_registry(&self) -> &ToolRegistryRef { &self.tool_registry }

    /// Access the event bus.
    pub fn event_bus(&self) -> &EventBusRef { &self.event_bus }

    /// Access the memory subsystem.
    pub fn memory(&self) -> &MemoryRef { &self.memory }

    /// Access the kernel config.
    pub fn config(&self) -> &KernelConfig { &self.config }

    /// Access the flat KV settings provider.
    pub fn settings(&self) -> &SettingsRef { &self.settings }

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

    /// Access the device registry (for hot-plugging devices).
    pub fn device_registry(&self) -> &DeviceRegistryRef { &self.device_registry }

    /// Access the audit log.
    pub fn audit_log(&self) -> &Arc<dyn AuditLog> { self.audit.audit_log() }

    /// Access the unified audit subsystem.
    pub fn audit(&self) -> &AuditRef { &self.audit }

    /// Access the approval manager.
    pub fn approval(&self) -> &Arc<crate::approval::ApprovalManager> { self.security.approval() }

    /// Access the unified security subsystem.
    pub fn security(&self) -> &SecurityRef { &self.security }

    /// Query the audit log for events matching the given filter.
    pub async fn audit_query(&self, filter: AuditFilter) -> Vec<AuditEvent> {
        self.audit.query(filter).await
    }

    /// Access the global semaphore (used by event loop for spawn limits).
    pub(crate) fn global_semaphore(&self) -> &Arc<Semaphore> { &self.global_semaphore }

    /// Access the shared KV backend (used by event loop).
    pub(crate) fn shared_kv(&self) -> &KvBackendRef { &self.shared_kv }

    /// Access the session repository (used by event loop).
    pub(crate) fn session_repo(&self) -> &SessionRepoRef { &self.session_repo }

    /// Access the pipe registry (used by event loop).
    pub(crate) fn pipe_registry(&self) -> &PipeRegistry { &self.pipe_registry }

    /// Access the driver registry (used by event loop for ResolveDriver).
    pub(crate) fn driver_registry(&self) -> &DriverRegistryRef { &self.driver_registry }

    /// Ensure a session exists for the given ID, creating one if needed.
    ///
    /// Called at spawn time to set up the process's conversation environment.
    pub(crate) async fn ensure_session(&self, session_id: &SessionId) {
        use chrono::Utc;

        match self.session_repo.get_session(session_id).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                let now = Utc::now();
                let entry = crate::session::SessionEntry {
                    key:           session_id.clone(),
                    title:         None,
                    model:         None,
                    system_prompt: None,
                    message_count: 0,
                    preview:       None,
                    metadata:      None,
                    created_at:    now,
                    updated_at:    now,
                };
                if let Err(e) = self.session_repo.create_session(&entry).await {
                    tracing::warn!(%e, "failed to create session");
                }
            }
            Err(e) => {
                tracing::warn!(%e, "failed to check session");
            }
        }
    }

    /// Load raw conversation messages for a session.
    ///
    /// Called at spawn time to provide the process with its initial
    /// conversation state. Returns an empty vec on error.
    #[allow(dead_code)]
    pub(crate) async fn load_session_messages(
        &self,
        session_id: &SessionId,
    ) -> Vec<crate::channel::types::ChatMessage> {
        match self
            .session_repo
            .read_messages(session_id, None, None)
            .await
        {
            Ok(msgs) => msgs,
            Err(e) => {
                tracing::warn!(%e, "failed to load session messages");
                vec![]
            }
        }
    }

    /// Construct a `Kernel` for testing with minimal I/O subsystem.
    ///
    /// Used by [`crate::testing::TestKernelBuilder`] to assemble kernels in
    /// tests without going through the public `new()` constructor.
    ///
    /// Creates minimal I/O subsystem components (IngressPipeline,
    /// EndpointRegistry) with Noop resolvers.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn for_testing(
        config: KernelConfig,
        process_table: Arc<ProcessTable>,
        global_semaphore: Arc<Semaphore>,
        driver_registry: DriverRegistryRef,
        tool_registry: ToolRegistryRef,
        memory: MemoryRef,
        event_bus: EventBusRef,
        security: SecurityRef,
        agent_registry: AgentRegistryRef,
        shared_kv: KvBackendRef,
        audit: AuditRef,
        session_repo: SessionRepoRef,
        settings: SettingsRef,
        stream_hub: StreamHubRef,
        pipe_registry: PipeRegistry,
        device_registry: DeviceRegistryRef,
    ) -> Self {
        let identity_resolver: IdentityResolverRef =
            Arc::new(crate::defaults::noop::NoopIdentityResolver);
        let session_resolver: SessionResolverRef =
            Arc::new(crate::defaults::noop::NoopSessionResolver);

        // Always create a ShardedEventQueue so the parallel event loop works.
        let sharded_queue = Arc::new(crate::sharded_event_queue::ShardedEventQueue::new(
            crate::sharded_event_queue::ShardedEventQueueConfig::default(),
        ));
        let event_queue: EventQueueRef = sharded_queue.clone();

        let ingress_pipeline = Arc::new(IngressPipeline::with_event_queue(
            identity_resolver,
            session_resolver,
            event_queue.clone(),
        ));
        let endpoint_registry = Arc::new(EndpointRegistry::new());

        Self {
            config,
            process_table,
            global_semaphore,
            driver_registry,
            tool_registry,
            memory,
            event_bus,
            security,
            agent_registry,
            shared_kv,
            audit,
            session_repo,
            settings,
            pipe_registry,
            device_registry,
            stream_hub,
            ingress_pipeline,
            endpoint_registry,
            egress_adapters: HashMap::new(),
            event_queue,
            sharded_queue,
            started_at: Timestamp::now(),
        }
    }

    // -- I/O subsystem accessors -----------------------------------------

    /// Access the ingress pipeline (adapters need this to push messages).
    pub fn ingress_pipeline(&self) -> &IngressPipelineRef { &self.ingress_pipeline }

    /// Access the ephemeral stream hub (WebAdapter needs this for token
    /// deltas).
    pub fn stream_hub(&self) -> &StreamHubRef { &self.stream_hub }

    /// Access the endpoint registry (WebAdapter needs this for connection
    /// tracking).
    pub fn endpoint_registry(&self) -> &EndpointRegistryRef { &self.endpoint_registry }

    /// Create a [`KernelHandle`] for external callers.
    ///
    /// The handle is cheap to clone (all `Arc`s) and routes all mutations
    /// through the event queue, while exposing read-only accessors for
    /// kernel subsystems.
    pub fn handle(&self) -> crate::handle::kernel_handle::KernelHandle {
        crate::handle::kernel_handle::KernelHandle::new(
            self.event_queue.clone(),
            Arc::clone(&self.agent_registry),
            Arc::clone(&self.process_table),
            Arc::clone(&self.ingress_pipeline),
            Arc::clone(&self.stream_hub),
            Arc::clone(&self.endpoint_registry),
            Arc::clone(&self.audit),
            Arc::clone(&self.settings),
            Arc::clone(&self.security),
            self.config.clone(),
            Arc::clone(&self.tool_registry),
            Arc::clone(&self.device_registry),
            Arc::clone(&self.global_semaphore),
            self.started_at,
        )
    }

    /// Access the unified event queue.
    pub fn event_queue(&self) -> &EventQueueRef { &self.event_queue }

    /// Access the sharded event queue.
    pub(crate) fn sharded_queue(&self) -> &ShardedQueueRef { &self.sharded_queue }

    /// Register an egress adapter for a channel type.
    ///
    /// Must be called **before** [`start()`](Self::start).
    pub fn register_adapter(&mut self, channel_type: ChannelType, adapter: EgressAdapterRef) {
        self.egress_adapters.insert(channel_type, adapter);
    }

    /// Start the unified event loop as a background task.
    ///
    /// Consumes `self` by value, wraps it in `Arc`, spawns the event loop,
    /// and returns `(Arc<Kernel>, KernelHandle)`.
    ///
    /// The `KernelHandle` is the preferred external API — it provides
    /// read-only accessors and mutation methods that flow through the event
    /// queue. The `Arc<Kernel>` is retained for internal use and backwards
    /// compatibility during the migration period.
    pub fn start(
        self,
        cancel_token: CancellationToken,
    ) -> (Arc<Self>, crate::handle::kernel_handle::KernelHandle) {
        let kernel = Arc::new(self);
        let handle = kernel.handle();

        // Unified event loop — parallel multi-processor mode via ShardedEventQueue.
        tokio::spawn({
            let k = kernel.clone();
            let token = cancel_token;
            async move {
                Kernel::run_event_loop_arc(k, token).await;
            }
        });

        info!("kernel event loop started");
        (kernel, handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        defaults::{
            noop::{NoopEventBus, NoopMemory, NoopSessionRepository, NoopSettingsProvider},
            noop_user_store::NoopUserStore,
        },
        handle::kernel_handle::KernelHandle,
        io::stream::StreamHub,
        llm::DriverRegistryBuilder,
        process::{AgentManifest, agent_registry::AgentRegistry, principal::Principal},
        tool::ToolRegistry,
    };

    fn make_test_kernel(max_concurrency: usize, child_limit: usize) -> Kernel {
        let config = KernelConfig {
            max_concurrency,
            default_child_limit: child_limit,
            default_max_iterations: 5,
            memory_quota_per_agent: 1000,
            ..Default::default()
        };

        let registry = Arc::new(AgentRegistry::new(
            crate::testing::test_manifests(),
            std::env::temp_dir().join("kernel_test_agents"),
        ));

        let driver_registry = Arc::new(DriverRegistryBuilder::new("test", "test-model").build());

        Kernel::new(
            config,
            driver_registry,
            Arc::new(ToolRegistry::new()),
            Arc::new(NoopMemory),
            Arc::new(NoopEventBus),
            Arc::new(crate::security::SecuritySubsystem::noop()),
            registry,
            Arc::new(NoopSessionRepository) as SessionRepoRef,
            Arc::new(NoopSettingsProvider) as SettingsRef,
            Arc::new(StreamHub::new(16)),
            Arc::new(crate::defaults::noop::NoopIdentityResolver) as IdentityResolverRef,
            Arc::new(crate::defaults::noop::NoopSessionResolver) as SessionResolverRef,
            Arc::new(crate::audit_subsystem::AuditSubsystem::noop()),
            None,
            None,
        )
    }

    /// Create a test kernel with its event loop running, returning an
    /// Arc<Kernel> and a CancellationToken to shut it down.
    fn start_test_kernel(
        max_concurrency: usize,
        child_limit: usize,
    ) -> (KernelHandle, CancellationToken) {
        let kernel = make_test_kernel(max_concurrency, child_limit);
        let cancel = CancellationToken::new();
        let (_arc, handle) = kernel.start(cancel.clone());
        (handle, cancel)
    }

    fn test_manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name:               name.to_string(),
            role:               None,
            description:        format!("Test agent: {name}"),
            model:              Some("test-model".to_string()),
            system_prompt:      "You are a test agent.".to_string(),
            soul_prompt:        None,
            provider_hint:      None,
            max_iterations:     Some(5),
            tools:              vec![],
            max_children:       None,
            max_context_tokens: None,
            priority:           crate::process::Priority::default(),
            metadata:           serde_json::Value::Null,
            sandbox:            None,
        }
    }

    #[test]
    fn test_kernel_creation() {
        let kernel = make_test_kernel(10, 5);
        assert_eq!(kernel.config().max_concurrency, 10);
        assert_eq!(kernel.config().default_child_limit, 5);
        assert_eq!(kernel.process_table().list().len(), 0);
    }

    #[test]
    fn test_kernel_agent_registry() {
        let kernel = make_test_kernel(10, 5);
        assert!(kernel.agent_registry().get("rara").is_some());
        assert!(kernel.agent_registry().get("scout").is_some());
        assert!(kernel.agent_registry().get("nonexistent").is_none());
    }

    #[test]
    fn test_kernel_default_config() {
        let config = KernelConfig::default();
        assert_eq!(config.max_concurrency, 16);
        assert_eq!(config.default_child_limit, 8);
        assert_eq!(config.default_max_iterations, 25);
        assert_eq!(config.memory_quota_per_agent, 1000);
    }

    #[tokio::test]
    async fn test_kernel_spawn_creates_process() {
        let (kernel, cancel) = start_test_kernel(10, 5);
        let manifest = test_manifest("test-agent");
        let principal = Principal::user("test-user");

        let result = kernel
            .spawn_with_input(manifest, "hello".to_string(), principal, None)
            .await;

        assert!(result.is_ok());
        let agent_id = result.unwrap();

        let process = kernel.process_table().get(agent_id);
        assert!(process.is_some());
        let process = process.unwrap();
        assert_eq!(process.manifest.name, "test-agent");
        // Each process gets its own agent-scoped session.
        assert!(process.session_id.as_str().starts_with("agent:"));

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_kernel_spawn_global_limit() {
        let (kernel, cancel) = start_test_kernel(2, 5);
        let principal = Principal::user("test-user");

        let h1 = kernel
            .spawn_with_input(
                test_manifest("a1"),
                "task 1".to_string(),
                principal.clone(),
                None,
            )
            .await;
        assert!(h1.is_ok());

        let h2 = kernel
            .spawn_with_input(
                test_manifest("a2"),
                "task 2".to_string(),
                principal.clone(),
                None,
            )
            .await;
        assert!(h2.is_ok());

        // Third spawn should fail (global limit reached)
        let h3 = kernel
            .spawn_with_input(test_manifest("a3"), "task 3".to_string(), principal, None)
            .await;
        assert!(h3.is_err());
        let err = h3.unwrap_err();
        assert!(
            err.to_string().contains("global concurrency limit"),
            "expected global limit error, got: {}",
            err
        );

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_kernel_spawn_named_success() {
        let (kernel, cancel) = start_test_kernel(10, 5);
        let principal = Principal::user("test-user");

        let result = kernel
            .spawn_named("scout", "find something".to_string(), principal, None)
            .await;
        assert!(result.is_ok());

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_kernel_spawn_named_not_found() {
        let (kernel, cancel) = start_test_kernel(10, 5);
        let principal = Principal::user("test-user");

        let result = kernel
            .spawn_named("nonexistent", "task".to_string(), principal, None)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("manifest not found")
        );

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_kernel_spawn_with_parent() {
        let (kernel, cancel) = start_test_kernel(10, 5);
        let principal = Principal::user("test-user");

        let parent_id = kernel
            .spawn_with_input(
                test_manifest("parent"),
                "parent task".to_string(),
                principal.clone(),
                None,
            )
            .await
            .unwrap();

        let child_id = kernel
            .spawn_with_input(
                test_manifest("child"),
                "child task".to_string(),
                principal,
                Some(parent_id),
            )
            .await
            .unwrap();

        let child_process = kernel.process_table().get(child_id).unwrap();
        assert_eq!(child_process.parent_id, Some(parent_id));

        let children = kernel.process_table().children_of(parent_id);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].agent_id, child_id);

        cancel.cancel();
    }

    // -----------------------------------------------------------------------
    // /proc API — Kernel-level introspection tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_kernel_process_stats_after_spawn() {
        let (kernel, cancel) = start_test_kernel(10, 5);
        let principal = Principal::user("test-user");

        let agent_id = kernel
            .spawn_with_input(
                test_manifest("stats-agent"),
                "hello".to_string(),
                principal,
                None,
            )
            .await
            .unwrap();

        let stats = kernel.process_stats(&agent_id).await;
        assert!(stats.is_some());

        let stats = stats.unwrap();
        assert_eq!(stats.agent_id, agent_id);
        assert_eq!(stats.manifest_name, "stats-agent");
        assert!(stats.parent_id.is_none());

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_kernel_list_processes_returns_all_spawned() {
        let (kernel, cancel) = start_test_kernel(10, 5);
        let principal = Principal::user("test-user");

        kernel
            .spawn_with_input(
                test_manifest("agent-1"),
                "task 1".to_string(),
                principal.clone(),
                None,
            )
            .await
            .unwrap();

        kernel
            .spawn_with_input(
                test_manifest("agent-2"),
                "task 2".to_string(),
                principal,
                None,
            )
            .await
            .unwrap();

        let list = kernel.list_processes().await;
        assert_eq!(list.len(), 2);

        let names: Vec<&str> = list.iter().map(|s| s.manifest_name.as_str()).collect();
        assert!(names.contains(&"agent-1"));
        assert!(names.contains(&"agent-2"));

        cancel.cancel();
    }

    #[test]
    fn test_kernel_system_stats_initial() {
        let kernel = make_test_kernel(10, 5);
        let stats = kernel.system_stats();
        assert_eq!(stats.active_processes, 0);
        assert_eq!(stats.total_spawned, 0);
        assert_eq!(stats.total_completed, 0);
        assert_eq!(stats.total_failed, 0);
        assert_eq!(stats.global_semaphore_available, 10);
        assert_eq!(stats.total_tokens_consumed, 0);
    }

    #[tokio::test]
    async fn test_kernel_system_stats_after_spawn() {
        let (kernel, cancel) = start_test_kernel(10, 5);
        let principal = Principal::user("test-user");

        kernel
            .spawn_with_input(
                test_manifest("sys-agent"),
                "work".to_string(),
                principal,
                None,
            )
            .await
            .unwrap();

        let stats = kernel.system_stats();
        assert_eq!(stats.total_spawned, 1);
        // The global semaphore permit is stored in ProcessRuntime, so the
        // available count decreases while the process is alive.
        assert_eq!(stats.global_semaphore_available, 9);

        cancel.cancel();
    }

    // =======================================================================
    // Signal system tests (via KernelEvent::SendSignal through EventQueue)
    // =======================================================================

    #[tokio::test]
    async fn test_signal_via_event_queue() {
        let (kernel, cancel) = start_test_kernel(10, 5);
        let principal = Principal::user("test-user");

        let agent_id = kernel
            .spawn_with_input(
                test_manifest("signalable"),
                "initial message".to_string(),
                principal,
                None,
            )
            .await
            .unwrap();

        // Verify process exists in the table.
        let process = kernel.process_table().get(agent_id);
        assert!(process.is_some(), "process should exist after spawn");

        // Push a Pause signal via event queue.
        let event = crate::unified_event::KernelEvent::SendSignal {
            target: agent_id,
            signal: crate::process::Signal::Pause,
        };
        kernel.event_queue().push(event).await.unwrap();

        // Allow time for the event loop to process.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Process should be Paused.
        let process = kernel.process_table().get(agent_id).unwrap();
        assert_eq!(
            process.state,
            ProcessState::Paused,
            "expected Paused after signal, got {:?}",
            process.state
        );

        // Push Resume signal.
        let event = crate::unified_event::KernelEvent::SendSignal {
            target: agent_id,
            signal: crate::process::Signal::Resume,
        };
        kernel.event_queue().push(event).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let process = kernel.process_table().get(agent_id).unwrap();
        assert_ne!(
            process.state,
            ProcessState::Paused,
            "process should not be Paused after Resume"
        );

        cancel.cancel();
    }

    #[test]
    fn test_kernel_system_stats_serializes_to_json() {
        let kernel = make_test_kernel(10, 5);
        let stats = kernel.system_stats();
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"active_processes\":0"));
        assert!(json.contains("\"global_semaphore_available\":10"));
    }

    // =======================================================================
    // Per-process syscall tool injection tests (#443)
    // =======================================================================

    #[tokio::test]
    async fn test_get_tool_registry_includes_kernel_tool() {
        let (kernel, cancel) = start_test_kernel(10, 5);
        let principal = Principal::user("test-user");

        let agent_id = kernel
            .spawn_with_input(
                test_manifest("tool-test-agent"),
                "hello".to_string(),
                principal,
                None,
            )
            .await
            .unwrap();

        // Allow time for the spawn to fully register the runtime.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Create a ProcessHandle pointing at this agent and query the
        // tool registry via the GetToolRegistry syscall.
        let handle = crate::handle::process_handle::ProcessHandle::new(
            agent_id,
            crate::process::SessionId::new("test"),
            Principal::user("test-user"),
            kernel.event_queue().clone(),
        );

        let registry = handle.tool_registry().await.unwrap();

        // The per-process SyscallTool should be injected.
        assert!(
            registry.get("kernel").is_some(),
            "tool registry should include kernel, got: {:?}",
            registry.tool_names(),
        );

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_tool_registry_filtered_by_manifest_tools() {
        // The "scout" manifest specifies tools: ["read_file", "grep"].
        // Even though the kernel tool is injected, `filtered()` should exclude
        // it when the manifest specifies a non-empty tool list.
        let (kernel, cancel) = start_test_kernel(10, 5);
        let principal = Principal::user("test-user");

        let agent_id = kernel
            .spawn_with_input(
                test_manifest("filter-agent"),
                "hello".to_string(),
                principal,
                None,
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let handle = crate::handle::process_handle::ProcessHandle::new(
            agent_id,
            crate::process::SessionId::new("test"),
            Principal::user("test-user"),
            kernel.event_queue().clone(),
        );

        let full_registry = handle.tool_registry().await.unwrap();
        assert!(full_registry.get("kernel").is_some());

        // Filter with an explicit tool whitelist that excludes kernel.
        let whitelist = vec!["read_file".to_string(), "grep".to_string()];
        let filtered = full_registry.filtered(&whitelist);
        assert!(
            filtered.get("kernel").is_none(),
            "filtered registry should NOT include kernel"
        );

        // Filter with empty list means include all.
        let unfiltered = full_registry.filtered(&[]);
        assert!(
            unfiltered.get("kernel").is_some(),
            "unfiltered registry should include kernel"
        );

        cancel.cancel();
    }

    // -----------------------------------------------------------------------
    // Guard integration tests
    // -----------------------------------------------------------------------

    /// A guard that denies calls to tools whose name contains "dangerous".
    struct DenyDangerousGuard;

    #[async_trait::async_trait]
    impl crate::guard::Guard for DenyDangerousGuard {
        async fn check_tool(
            &self,
            _ctx: &crate::guard::GuardContext,
            tool_name: &str,
            _args: &serde_json::Value,
        ) -> crate::guard::Verdict {
            if tool_name.contains("dangerous") {
                crate::guard::Verdict::Deny {
                    reason: format!("tool '{tool_name}' is forbidden"),
                }
            } else {
                crate::guard::Verdict::Allow
            }
        }

        async fn check_output(
            &self,
            _ctx: &crate::guard::GuardContext,
            _content: &str,
        ) -> crate::guard::Verdict {
            crate::guard::Verdict::Allow
        }
    }

    fn make_guarded_kernel() -> Kernel {
        let config = KernelConfig {
            max_concurrency: 10,
            default_child_limit: 5,
            default_max_iterations: 5,
            memory_quota_per_agent: 1000,
            ..Default::default()
        };

        let registry = Arc::new(AgentRegistry::new(
            crate::testing::test_manifests(),
            std::env::temp_dir().join("kernel_guard_test_agents"),
        ));

        let driver_registry = Arc::new(DriverRegistryBuilder::new("test", "test-model").build());

        let security = Arc::new(crate::security::SecuritySubsystem::new(
            Arc::new(NoopUserStore),
            Arc::new(DenyDangerousGuard),
            Arc::new(crate::approval::ApprovalManager::new(
                crate::approval::ApprovalPolicy::default(),
            )),
        ));

        Kernel::new(
            config,
            driver_registry,
            Arc::new(ToolRegistry::new()),
            Arc::new(NoopMemory),
            Arc::new(NoopEventBus),
            security,
            registry,
            Arc::new(NoopSessionRepository) as SessionRepoRef,
            Arc::new(NoopSettingsProvider) as SettingsRef,
            Arc::new(StreamHub::new(16)),
            Arc::new(crate::defaults::noop::NoopIdentityResolver) as IdentityResolverRef,
            Arc::new(crate::defaults::noop::NoopSessionResolver) as SessionResolverRef,
            Arc::new(crate::audit_subsystem::AuditSubsystem::noop()),
            None,
            None,
        )
    }

    #[tokio::test]
    async fn test_check_guard_batch_denies_dangerous_tools() {
        let kernel = make_guarded_kernel();
        let cancel = CancellationToken::new();
        let (_kernel, handle) = kernel.start(cancel.clone());

        let principal = Principal::user("test-user");
        let agent_id = handle
            .spawn_with_input(
                test_manifest("guard-test"),
                "hello".to_string(),
                principal,
                None,
            )
            .await
            .unwrap();

        // Allow process to be registered.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Push a CheckGuardBatch syscall via the event queue.
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let checks = vec![
            ("safe_tool".to_string(), serde_json::json!({"arg": "val"})),
            (
                "dangerous_delete".to_string(),
                serde_json::json!({"path": "/etc"}),
            ),
            ("another_safe".to_string(), serde_json::json!({})),
        ];

        let process = handle.process_table().get(agent_id).unwrap();
        let event = crate::unified_event::KernelEvent::Syscall(
            crate::unified_event::Syscall::CheckGuardBatch {
                agent_id,
                session_id: process.session_id.clone(),
                checks,
                reply_tx,
            },
        );
        handle.event_queue().push(event).await.unwrap();

        let verdicts = tokio::time::timeout(std::time::Duration::from_secs(2), reply_rx)
            .await
            .expect("timeout waiting for guard verdict")
            .expect("reply channel closed");

        assert_eq!(verdicts.len(), 3);
        assert!(verdicts[0].is_allow(), "safe_tool should be allowed");
        assert!(verdicts[1].is_deny(), "dangerous_delete should be denied");
        assert!(verdicts[2].is_allow(), "another_safe should be allowed");

        // Verify the deny reason contains the tool name.
        if let crate::guard::Verdict::Deny { reason } = &verdicts[1] {
            assert!(
                reason.contains("dangerous_delete"),
                "deny reason should mention the tool name, got: {reason}"
            );
        }

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_check_guard_batch_allows_all_safe_tools() {
        let kernel = make_guarded_kernel();
        let cancel = CancellationToken::new();
        let (_kernel, handle) = kernel.start(cancel.clone());

        let principal = Principal::user("test-user");
        let agent_id = handle
            .spawn_with_input(
                test_manifest("guard-safe-test"),
                "hi".to_string(),
                principal,
                None,
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let checks = vec![
            (
                "read_file".to_string(),
                serde_json::json!({"path": "/tmp/test"}),
            ),
            ("grep".to_string(), serde_json::json!({"pattern": "hello"})),
        ];

        let process = handle.process_table().get(agent_id).unwrap();
        let event = crate::unified_event::KernelEvent::Syscall(
            crate::unified_event::Syscall::CheckGuardBatch {
                agent_id,
                session_id: process.session_id.clone(),
                checks,
                reply_tx,
            },
        );
        handle.event_queue().push(event).await.unwrap();

        let verdicts = tokio::time::timeout(std::time::Duration::from_secs(2), reply_rx)
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(verdicts.len(), 2);
        assert!(verdicts[0].is_allow());
        assert!(verdicts[1].is_allow());

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_check_guard_batch_empty_checks() {
        let kernel = make_guarded_kernel();
        let cancel = CancellationToken::new();
        let (_kernel, handle) = kernel.start(cancel.clone());

        let principal = Principal::user("test-user");
        let agent_id = handle
            .spawn_with_input(
                test_manifest("guard-empty-test"),
                "hi".to_string(),
                principal,
                None,
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let checks = vec![];

        let process = handle.process_table().get(agent_id).unwrap();
        let event = crate::unified_event::KernelEvent::Syscall(
            crate::unified_event::Syscall::CheckGuardBatch {
                agent_id,
                session_id: process.session_id.clone(),
                checks,
                reply_tx,
            },
        );
        handle.event_queue().push(event).await.unwrap();

        let verdicts = tokio::time::timeout(std::time::Duration::from_secs(2), reply_rx)
            .await
            .expect("timeout")
            .expect("channel closed");

        assert!(
            verdicts.is_empty(),
            "empty checks should return empty verdicts"
        );

        cancel.cancel();
    }
}
