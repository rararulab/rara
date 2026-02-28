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
//! Kernel (top-level)
//!   ├── ProcessTable  (all running agents)
//!   ├── global_semaphore (max total concurrent agents)
//!   ├── ManifestLoader  (named agent definitions)
//!   └── KernelInner (shared state via Arc)
//!         ├── LlmProviderLoader
//!         ├── ToolRegistry
//!         ├── Memory
//!         ├── EventBus
//!         ├── Guard
//!         └── shared_kv (cross-agent KV)
//! ```
//!
//! Each spawned agent receives a [`ScopedKernelHandle`] providing syscall-like
//! access to kernel capabilities (ProcessOps, MemoryOps, EventOps, GuardOps).

use std::sync::Arc;

use dashmap::DashMap;
use jiff::Timestamp;
use tokio::sync::{Semaphore, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    error::{KernelError, Result},
    event::EventBus,
    guard::Guard,
    handle::{AgentHandle, scoped::ScopedKernelHandle},
    io::{bus::OutboundBus, stream::StreamHub, types::InboundMessage},
    memory::Memory,
    process::{
        AgentEnv, AgentId, AgentManifest, AgentProcess, AgentResult, ProcessState, ProcessTable,
        SessionId, manifest_loader::ManifestLoader, principal::Principal, user::UserStore,
    },
    provider::LlmProviderLoaderRef,
    session::SessionRepository,
    tool::ToolRegistry,
};

// ---------------------------------------------------------------------------
// KernelInner — shared kernel state
// ---------------------------------------------------------------------------

/// Shared kernel state accessed by all `ScopedKernelHandle` instances via `Arc`.
///
/// This is the "real" kernel data — process table, component registries,
/// I/O subsystems. `Kernel` wraps it with concurrency config and a public API.
pub(crate) struct KernelInner {
    /// The global process table tracking all running agents.
    pub process_table:          Arc<ProcessTable>,
    /// Global semaphore limiting total concurrent agent processes.
    pub global_semaphore:       Arc<Semaphore>,
    /// Default maximum number of children per agent.
    pub default_child_limit:    usize,
    /// Default max LLM iterations for spawned agents.
    pub default_max_iterations: usize,
    /// LLM provider loader for acquiring providers.
    pub llm_provider:           LlmProviderLoaderRef,
    /// Global tool registry (spawned agents get filtered subsets).
    pub tool_registry:          Arc<ToolRegistry>,
    /// 3-layer memory (not used for cross-agent KV — see shared_kv).
    pub memory:                 Arc<dyn Memory>,
    /// Event bus for publishing kernel events.
    pub event_bus:              Arc<dyn EventBus>,
    /// Guard for tool approval checks.
    pub guard:                  Arc<dyn Guard>,
    /// Manifest loader for looking up named agent definitions.
    pub manifest_loader:        ManifestLoader,
    /// Cross-agent shared key-value store (simple DashMap).
    pub shared_kv:              DashMap<String, serde_json::Value>,
    /// User store for user management and permission validation.
    pub user_store:             Arc<dyn UserStore>,
    /// Session repository for conversation history.
    pub session_repo:           Arc<dyn SessionRepository>,
    /// Stream hub for real-time streaming events.
    pub stream_hub:             Arc<StreamHub>,
    /// Outbound bus for publishing final responses.
    pub outbound_bus:           Arc<dyn OutboundBus>,
}

impl KernelInner {
    /// Validate that the principal's user exists, is enabled, and has Spawn
    /// permission.
    ///
    /// Called by both `Kernel::spawn()` and `ScopedKernelHandle::spawn()`.
    pub(crate) async fn validate_principal(&self, principal: &Principal) -> Result<()> {
        let user = self
            .user_store
            .get_by_name(&principal.user_id.0)
            .await?
            .ok_or(KernelError::UserNotFound {
                name: principal.user_id.0.clone(),
            })?;
        if !user.enabled {
            return Err(KernelError::UserDisabled { name: user.name });
        }
        if !user.has_permission(&crate::process::user::Permission::Spawn) {
            return Err(KernelError::PermissionDenied {
                reason: format!("user '{}' lacks Spawn permission", user.name),
            });
        }
        Ok(())
    }

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
    pub(crate) async fn load_session_messages(
        &self,
        session_id: &SessionId,
    ) -> Vec<crate::channel::types::ChatMessage> {
        match self.session_repo.read_messages(session_id, None, None).await {
            Ok(msgs) => msgs,
            Err(e) => {
                tracing::warn!(%e, "failed to load session messages");
                vec![]
            }
        }
    }

    /// Unified spawn — creates an agent process with a mailbox-driven
    /// process_loop.
    ///
    /// Shared by `Kernel::spawn()` (top-level) and
    /// `ScopedKernelHandle::spawn()` (child). The caller is responsible for
    /// acquiring semaphore permits and computing the effective `allowed_tools`.
    pub(crate) async fn spawn_process(
        self_ref: Arc<KernelInner>,
        manifest: AgentManifest,
        inbound: InboundMessage,
        principal: Principal,
        session_id: SessionId,
        parent_id: Option<AgentId>,
        child_limit: usize,
        allowed_tools: Vec<String>,
        permits: SpawnPermits,
    ) -> Result<AgentHandle> {
        // Kernel setup: ensure session exists and load initial history.
        // This happens at spawn time before process creation.
        self_ref.ensure_session(&session_id).await;
        let initial_messages = self_ref.load_session_messages(&session_id).await;

        let agent_id = AgentId::new();
        let (mailbox_tx, mailbox_rx) =
            tokio::sync::mpsc::channel::<crate::process::ProcessMessage>(64);
        let (result_tx, result_rx) = oneshot::channel();

        // Register process in table
        let process = AgentProcess {
            agent_id,
            parent_id,
            session_id: session_id.clone(),
            manifest: manifest.clone(),
            principal: principal.clone(),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
        };
        self_ref.process_table.insert(process);
        self_ref
            .process_table
            .set_mailbox(agent_id, mailbox_tx.clone());

        // Deliver first message
        mailbox_tx
            .try_send(crate::process::ProcessMessage::UserMessage(inbound))
            .map_err(|_| KernelError::SpawnFailed {
                message: "failed to deliver initial message".to_string(),
            })?;

        // Compute effective tool registry
        let effective_registry = if allowed_tools.is_empty() {
            Arc::clone(&self_ref.tool_registry)
        } else {
            Arc::new(self_ref.tool_registry.filtered(&allowed_tools))
        };

        // Build ScopedKernelHandle
        let handle = Arc::new(ScopedKernelHandle {
            agent_id,
            session_id: session_id.clone(),
            principal,
            manifest,
            allowed_tools,
            tool_registry: effective_registry,
            child_semaphore: Arc::new(Semaphore::new(child_limit)),
            inner: Arc::clone(&self_ref),
        });

        // Create cancellation token (child of parent's token if applicable).
        let token = if let Some(pid) = parent_id {
            self_ref
                .process_table
                .get_cancellation_token(&pid)
                .map(|parent_token| parent_token.child_token())
                .unwrap_or_default()
        } else {
            CancellationToken::new()
        };
        self_ref
            .process_table
            .set_cancellation_token(agent_id, token.clone());

        // Kernel process management loop.
        //
        // This is the kernel side: it owns the mailbox, drives state
        // transitions, manages streams, persists messages, and publishes
        // outbound replies.  The actual agent execution is delegated to
        // `run_agent_turn` which only knows how to run the LLM.
        let inner = Arc::clone(&self_ref);
        tokio::spawn(async move {
            let _permits = permits;
            let mut mailbox_rx = mailbox_rx;
            let mut conversation = initial_messages;
            let mut last_result: Option<AgentResult> = None;

            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        info!(agent_id = %agent_id, "cancellation token triggered");
                        break;
                    }
                    msg = mailbox_rx.recv() => {
                        let Some(msg) = msg else { break };
                        match msg {
                            crate::process::ProcessMessage::UserMessage(inbound) => {
                                let _ = inner.process_table.set_state(
                                    agent_id,
                                    ProcessState::Running,
                                );

                                // Convert in-memory history to LLM format.
                                let history =
                                    match crate::runner::build_history_messages(&conversation) {
                                        Ok(msgs) if !msgs.is_empty() => Some(msgs),
                                        Ok(_) => None,
                                        Err(e) => {
                                            tracing::warn!(%e, "failed to convert history");
                                            None
                                        }
                                    };

                                // Append user message to conversation + persist.
                                let user_text = inbound.content.as_text();
                                let user_msg =
                                    crate::channel::types::ChatMessage::user(&user_text);
                                conversation.push(user_msg.clone());
                                if let Err(e) = inner
                                    .session_repo
                                    .append_message(&session_id, &user_msg)
                                    .await
                                {
                                    tracing::warn!(%e, "failed to persist user message");
                                }

                                // Open stream.
                                let stream_handle =
                                    inner.stream_hub.open(session_id.clone());

                                // === Agent execution ===
                                let turn_result =
                                    crate::process_loop::run_agent_turn(
                                        &handle,
                                        user_text,
                                        history,
                                        &stream_handle,
                                    )
                                    .await;

                                // Close stream.
                                inner.stream_hub.close(stream_handle.stream_id());

                                match turn_result {
                                    Ok(turn) if !turn.text.is_empty() => {
                                        // Persist assistant reply.
                                        let assistant_msg =
                                            crate::channel::types::ChatMessage::assistant(
                                                &turn.text,
                                            );
                                        conversation.push(assistant_msg.clone());
                                        if let Err(e) = inner
                                            .session_repo
                                            .append_message(
                                                &session_id,
                                                &assistant_msg,
                                            )
                                            .await
                                        {
                                            tracing::warn!(
                                                %e,
                                                "failed to persist assistant message"
                                            );
                                        }

                                        let result = AgentResult {
                                            output:     turn.text.clone(),
                                            iterations: turn.iterations,
                                            tool_calls: turn.tool_calls,
                                        };
                                        let _ = inner
                                            .process_table
                                            .set_result(agent_id, result.clone());

                                        // Publish reply via outbound bus.
                                        let envelope =
                                            crate::io::types::OutboundEnvelope {
                                                id:          crate::io::types::MessageId::new(),
                                                in_reply_to: inbound.id.clone(),
                                                user:        inbound.user.clone(),
                                                session_id:  session_id.clone(),
                                                routing:     crate::io::types::OutboundRouting::BroadcastAll,
                                                payload:     crate::io::types::OutboundPayload::Reply {
                                                    content:     crate::channel::types::MessageContent::Text(
                                                        turn.text,
                                                    ),
                                                    attachments: vec![],
                                                },
                                                timestamp:   jiff::Timestamp::now(),
                                            };
                                        if let Err(e) =
                                            inner.outbound_bus.publish(envelope).await
                                        {
                                            tracing::error!(
                                                %e,
                                                "failed to publish reply"
                                            );
                                        }

                                        info!(
                                            agent_id = %agent_id,
                                            iterations = result.iterations,
                                            tool_calls = result.tool_calls,
                                            "message processed"
                                        );
                                        last_result = Some(result);
                                    }
                                    Ok(_) => {
                                        // Empty result — nothing to publish.
                                    }
                                    Err(err_msg) => {
                                        let _ = inner.process_table.set_state(
                                            agent_id,
                                            ProcessState::Failed,
                                        );
                                        // Publish error via outbound bus.
                                        let envelope =
                                            crate::io::types::OutboundEnvelope {
                                                id:          crate::io::types::MessageId::new(),
                                                in_reply_to: inbound.id.clone(),
                                                user:        inbound.user.clone(),
                                                session_id:  session_id.clone(),
                                                routing:     crate::io::types::OutboundRouting::BroadcastAll,
                                                payload:     crate::io::types::OutboundPayload::Error {
                                                    code:    "agent_error".to_string(),
                                                    message: err_msg,
                                                },
                                                timestamp:   jiff::Timestamp::now(),
                                            };
                                        if let Err(e) =
                                            inner.outbound_bus.publish(envelope).await
                                        {
                                            tracing::error!(
                                                %e,
                                                "failed to publish error"
                                            );
                                        }
                                    }
                                }

                                let _ = inner.process_table.set_state(
                                    agent_id,
                                    ProcessState::Waiting,
                                );
                            }
                            crate::process::ProcessMessage::ChildResult {
                                child_id,
                                result,
                            } => {
                                info!(
                                    agent_id = %agent_id,
                                    child_id = %child_id,
                                    output_len = result.output.len(),
                                    "child result received"
                                );
                                // TODO: integrate child result into context
                            }
                            crate::process::ProcessMessage::Signal(
                                crate::process::Signal::Interrupt,
                            ) => {
                                tracing::warn!(agent_id = %agent_id, "interrupt received");
                                // TODO: cancel current LLM call
                            }
                        }
                    }
                }
            }

            // Set terminal state.
            let final_state = if token.is_cancelled() {
                ProcessState::Cancelled
            } else {
                ProcessState::Completed
            };
            let _ = inner.process_table.set_state(agent_id, final_state);
            info!(agent_id = %agent_id, "process ended");

            let _ = result_tx.send(last_result.unwrap_or(AgentResult {
                output:     "process ended".to_string(),
                iterations: 0,
                tool_calls: 0,
            }));
            inner.process_table.clear_cancellation_token(&agent_id);
        });

        Ok(AgentHandle {
            agent_id,
            mailbox: mailbox_tx,
            result_rx,
        })
    }
}

// ---------------------------------------------------------------------------
// SpawnPermits
// ---------------------------------------------------------------------------

/// Semaphore permits that must be held for the lifetime of the spawned task.
///
/// This enum accommodates both top-level spawns (global permit only) and
/// child spawns (global + child permit).
pub(crate) enum SpawnPermits {
    /// Top-level spawn — only a global permit.
    TopLevel {
        _global: tokio::sync::OwnedSemaphorePermit,
    },
    /// Child spawn — both a child and global permit.
    Child {
        _child:  tokio::sync::OwnedSemaphorePermit,
        _global: tokio::sync::OwnedSemaphorePermit,
    },
}

// ---------------------------------------------------------------------------
// KernelConfig
// ---------------------------------------------------------------------------

/// Kernel configuration.
#[derive(Debug, Clone)]
pub struct KernelConfig {
    /// Maximum number of concurrent agent processes globally.
    pub max_concurrency:        usize,
    /// Default maximum number of children per agent.
    pub default_child_limit:    usize,
    /// Default max LLM iterations for spawned agents.
    pub default_max_iterations: usize,
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            max_concurrency:        16,
            default_child_limit:    8,
            default_max_iterations: 25,
        }
    }
}

/// The unified agent orchestrator.
///
/// Acts as an OS kernel for agents: manages the process table, enforces
/// concurrency limits, and provides `spawn()` as the primary entry point.
pub struct Kernel {
    /// Shared kernel internals (process table, components, etc.).
    inner:  Arc<KernelInner>,
    /// Kernel configuration.
    config: KernelConfig,
}

impl Kernel {
    /// Create a new Kernel with the given configuration and components.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: KernelConfig,
        llm_provider: LlmProviderLoaderRef,
        tool_registry: Arc<ToolRegistry>,
        memory: Arc<dyn Memory>,
        event_bus: Arc<dyn EventBus>,
        guard: Arc<dyn Guard>,
        manifest_loader: ManifestLoader,
        user_store: Arc<dyn UserStore>,
    ) -> Self {
        info!(
            max_concurrency = config.max_concurrency,
            default_child_limit = config.default_child_limit,
            default_max_iterations = config.default_max_iterations,
            "booting kernel"
        );

        let inner = Arc::new(KernelInner {
            process_table: Arc::new(ProcessTable::new()),
            global_semaphore: Arc::new(Semaphore::new(config.max_concurrency)),
            default_child_limit: config.default_child_limit,
            default_max_iterations: config.default_max_iterations,
            llm_provider,
            tool_registry,
            memory,
            event_bus,
            guard,
            manifest_loader,
            shared_kv: DashMap::new(),
            user_store,
            session_repo: Arc::new(crate::defaults::noop::NoopSessionRepository)
                as Arc<dyn SessionRepository>,
            stream_hub: Arc::new(StreamHub::new(1)),
            outbound_bus: Arc::new(crate::io::memory_bus::InMemoryOutboundBus::new(1))
                as Arc<dyn OutboundBus>,
        });

        Self { inner, config }
    }

    /// Set the I/O pipeline components for process loops.
    ///
    /// Must be called before any Arc clones are taken (i.e., before
    /// `Arc::new(kernel)` or any `spawn()` calls).
    pub fn set_io_context(
        &mut self,
        session_repo: Arc<dyn SessionRepository>,
        stream_hub: Arc<StreamHub>,
        outbound_bus: Arc<dyn OutboundBus>,
    ) {
        let inner = Arc::get_mut(&mut self.inner)
            .expect("set_io_context must be called before any Arc clones");
        inner.session_repo = session_repo;
        inner.stream_hub = stream_hub;
        inner.outbound_bus = outbound_bus;
    }

    /// Spawn a long-lived agent process for a session.
    ///
    /// Spawns a process_loop that receives messages via a mailbox.
    /// The first message (from `inbound`) is automatically delivered.
    ///
    /// # Arguments
    /// - `manifest` — the agent definition to run
    /// - `inbound` — the first inbound message to process
    /// - `principal` — the identity under which the agent runs
    /// - `session_id` — the session this agent belongs to
    /// - `parent_id` — optional parent agent ID (for process tree)
    pub async fn spawn(
        &self,
        manifest: AgentManifest,
        inbound: InboundMessage,
        principal: Principal,
        session_id: SessionId,
        parent_id: Option<AgentId>,
    ) -> Result<AgentHandle> {
        // Validate user exists, is enabled, and has Spawn permission
        self.inner.validate_principal(&principal).await?;

        // Acquire global semaphore
        let global_permit = self
            .inner
            .global_semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::SpawnLimitReached {
                message: "global concurrency limit reached".to_string(),
            })?;

        let child_limit = manifest
            .max_children
            .unwrap_or(self.config.default_child_limit);

        KernelInner::spawn_process(
            Arc::clone(&self.inner),
            manifest,
            inbound,
            principal,
            session_id,
            parent_id,
            child_limit,
            vec![], // top-level: no tool restriction
            SpawnPermits::TopLevel {
                _global: global_permit,
            },
        )
        .await
    }

    /// Convenience spawn with string input (for workers and backward
    /// compatibility).
    ///
    /// Wraps the input string as a synthetic [`InboundMessage`] and delegates
    /// to [`spawn()`](Self::spawn).
    pub async fn spawn_with_input(
        &self,
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        session_id: SessionId,
        parent_id: Option<AgentId>,
    ) -> Result<AgentHandle> {
        let user_id = principal.user_id.clone();
        let inbound =
            InboundMessage::synthetic(input, user_id, session_id.clone());
        self.spawn(manifest, inbound, principal, session_id, parent_id)
            .await
    }

    /// Spawn a named agent by looking up its manifest (legacy string input).
    ///
    /// Uses the short-lived execution model (`spawn_with_input`).
    pub async fn spawn_named(
        &self,
        agent_name: &str,
        input: String,
        principal: Principal,
        session_id: SessionId,
        parent_id: Option<AgentId>,
    ) -> Result<AgentHandle> {
        let manifest = self
            .inner
            .manifest_loader
            .get(agent_name)
            .ok_or(KernelError::ManifestNotFound {
                name: agent_name.to_string(),
            })?
            .clone();

        self.spawn_with_input(manifest, input, principal, session_id, parent_id)
            .await
    }

    /// Access the process table for querying.
    pub fn process_table(&self) -> &ProcessTable { &self.inner.process_table }

    /// Access the manifest loader for looking up named manifests.
    pub fn manifest_loader(&self) -> &ManifestLoader { &self.inner.manifest_loader }

    /// Access the tool registry.
    pub fn tool_registry(&self) -> &Arc<ToolRegistry> { &self.inner.tool_registry }

    /// Access the event bus.
    pub fn event_bus(&self) -> &Arc<dyn EventBus> { &self.inner.event_bus }

    /// Access the memory subsystem.
    pub fn memory(&self) -> &Arc<dyn Memory> { &self.inner.memory }

    /// Access the kernel config.
    pub fn config(&self) -> &KernelConfig { &self.config }

    /// Access the shared KernelInner (for constructing ScopedKernelHandles
    /// externally).
    pub(crate) fn inner(&self) -> &Arc<KernelInner> { &self.inner }

    /// Construct a `Kernel` from a pre-built `KernelInner` and config.
    ///
    /// Used by [`crate::testing::TestKernelBuilder`] to assemble kernels in
    /// tests without going through the public `new()` constructor.
    pub(crate) fn from_inner(inner: Arc<KernelInner>, config: KernelConfig) -> Self {
        Self { inner, config }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        defaults::{
            noop::{NoopEventBus, NoopGuard, NoopMemory},
            noop_user_store::NoopUserStore,
        },
        process::principal::Principal,
        provider::EnvLlmProviderLoader,
    };

    fn make_test_kernel(max_concurrency: usize, child_limit: usize) -> Kernel {
        let config = KernelConfig {
            max_concurrency,
            default_child_limit: child_limit,
            default_max_iterations: 5,
        };

        let mut loader = ManifestLoader::new();
        loader.load_bundled();

        Kernel::new(
            config,
            Arc::new(EnvLlmProviderLoader::default()) as LlmProviderLoaderRef,
            Arc::new(ToolRegistry::new()),
            Arc::new(NoopMemory),
            Arc::new(NoopEventBus),
            Arc::new(NoopGuard),
            loader,
            Arc::new(NoopUserStore),
        )
    }

    fn test_manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name:           name.to_string(),
            description:    format!("Test agent: {name}"),
            model:          "test-model".to_string(),
            system_prompt:  "You are a test agent.".to_string(),
            provider_hint:  None,
            max_iterations: Some(5),
            tools:          vec![],
            max_children:   None,
            metadata:       serde_json::Value::Null,
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
    fn test_kernel_manifest_loader() {
        let kernel = make_test_kernel(10, 5);
        assert!(kernel.manifest_loader().get("scout").is_some());
        assert!(kernel.manifest_loader().get("planner").is_some());
        assert!(kernel.manifest_loader().get("worker").is_some());
        assert!(kernel.manifest_loader().get("nonexistent").is_none());
    }

    #[test]
    fn test_kernel_default_config() {
        let config = KernelConfig::default();
        assert_eq!(config.max_concurrency, 16);
        assert_eq!(config.default_child_limit, 8);
        assert_eq!(config.default_max_iterations, 25);
    }

    #[tokio::test]
    async fn test_kernel_spawn_creates_process() {
        let kernel = make_test_kernel(10, 5);
        let manifest = test_manifest("test-agent");
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        // spawn_with_input will fail because there's no real LLM provider,
        // but the process should be created in the table
        let handle = kernel
            .spawn_with_input(manifest, "hello".to_string(), principal, session_id, None)
            .await;

        // The spawn itself should succeed (it just launches a task)
        assert!(handle.is_ok());
        let handle = handle.unwrap();

        // Process should appear in the table
        let process = kernel.process_table().get(handle.agent_id);
        assert!(process.is_some());
        let process = process.unwrap();
        assert_eq!(process.manifest.name, "test-agent");
    }

    #[tokio::test]
    async fn test_kernel_spawn_global_limit() {
        let kernel = make_test_kernel(2, 5);
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        // Spawn 2 agents (global limit is 2)
        let h1 = kernel
            .spawn_with_input(
                test_manifest("a1"),
                "task 1".to_string(),
                principal.clone(),
                session_id.clone(),
                None,
            )
            .await;
        assert!(h1.is_ok());

        let h2 = kernel
            .spawn_with_input(
                test_manifest("a2"),
                "task 2".to_string(),
                principal.clone(),
                session_id.clone(),
                None,
            )
            .await;
        assert!(h2.is_ok());

        // Third spawn should fail (global limit reached)
        let h3 = kernel
            .spawn_with_input(
                test_manifest("a3"),
                "task 3".to_string(),
                principal,
                session_id,
                None,
            )
            .await;
        assert!(h3.is_err());
        let err = h3.unwrap_err();
        assert!(
            err.to_string().contains("global concurrency limit"),
            "expected global limit error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_kernel_spawn_named_success() {
        let kernel = make_test_kernel(10, 5);
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        let handle = kernel
            .spawn_named(
                "scout",
                "find something".to_string(),
                principal,
                session_id,
                None,
            )
            .await;
        assert!(handle.is_ok());
    }

    #[tokio::test]
    async fn test_kernel_spawn_named_not_found() {
        let kernel = make_test_kernel(10, 5);
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        let result = kernel
            .spawn_named(
                "nonexistent",
                "task".to_string(),
                principal,
                session_id,
                None,
            )
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("manifest not found")
        );
    }

    #[tokio::test]
    async fn test_kernel_spawn_with_parent() {
        let kernel = make_test_kernel(10, 5);
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        let parent_handle = kernel
            .spawn_with_input(
                test_manifest("parent"),
                "parent task".to_string(),
                principal.clone(),
                session_id.clone(),
                None,
            )
            .await
            .unwrap();

        let child_handle = kernel
            .spawn_with_input(
                test_manifest("child"),
                "child task".to_string(),
                principal,
                session_id,
                Some(parent_handle.agent_id),
            )
            .await
            .unwrap();

        let child_process = kernel.process_table().get(child_handle.agent_id).unwrap();
        assert_eq!(child_process.parent_id, Some(parent_handle.agent_id));

        let children = kernel.process_table().children_of(parent_handle.agent_id);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].agent_id, child_handle.agent_id);
    }
}
