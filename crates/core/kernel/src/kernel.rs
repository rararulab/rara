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

use std::{collections::HashMap, sync::{Arc, Mutex}};

use dashmap::DashMap;
use jiff::Timestamp;
use tokio::sync::{Semaphore, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    channel::types::ChannelType,
    error::{KernelError, Result},
    event::EventBus,
    guard::Guard,
    handle::{AgentHandle, scoped::ScopedKernelHandle},
    io::{
        bus::{InboundBus, OutboundBus},
        egress::{Egress, EgressAdapter, EndpointRegistry},
        ingress::{IdentityResolver, IngressPipeline, SessionResolver},
        stream::StreamHub,
        types::InboundMessage,
    },
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
    /// Maximum number of KV entries per agent (0 = unlimited).
    pub memory_quota_per_agent: usize,
    /// User store for user management and permission validation.
    pub user_store:             Arc<dyn UserStore>,
    /// Session repository for conversation history.
    pub session_repo:           Arc<dyn SessionRepository>,
    /// Model repository for runtime model resolution.
    pub model_repo:             Arc<dyn crate::model_repo::ModelRepo>,
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

            // Context compaction: resolve token budget and strategy.
            let max_context_tokens = handle
                .manifest()
                .max_context_tokens
                .unwrap_or(crate::memory::compaction::DEFAULT_MAX_CONTEXT_TOKENS);
            let compaction_strategy = crate::memory::compaction::SlidingWindowCompaction;

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

                                // Apply context compaction before building
                                // LLM history. This trims the in-memory
                                // conversation to fit within the token budget,
                                // preventing context-window overflow.
                                conversation = crate::memory::compaction::maybe_compact(
                                    conversation,
                                    max_context_tokens,
                                    &compaction_strategy,
                                )
                                .await;

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
                                    iterations = result.iterations,
                                    tool_calls = result.tool_calls,
                                    "child result received, resuming parent"
                                );

                                let _ = inner.process_table.set_state(
                                    agent_id,
                                    ProcessState::Running,
                                );

                                // Format child result as a system message injected
                                // into the parent's conversation context.
                                let child_result_text = format!(
                                    "[child_agent_result] child_id={child_id} \
                                     iterations={} tool_calls={}\n\n{}",
                                    result.iterations,
                                    result.tool_calls,
                                    result.output,
                                );
                                let child_msg =
                                    crate::channel::types::ChatMessage::system(
                                        &child_result_text,
                                    );
                                conversation.push(child_msg.clone());
                                if let Err(e) = inner
                                    .session_repo
                                    .append_message(&session_id, &child_msg)
                                    .await
                                {
                                    tracing::warn!(
                                        %e,
                                        "failed to persist child result message"
                                    );
                                }

                                // Build history and run a new LLM turn so the
                                // parent can reason about the child's output.
                                let history =
                                    match crate::runner::build_history_messages(
                                        &conversation,
                                    ) {
                                        Ok(msgs) if !msgs.is_empty() => {
                                            Some(msgs)
                                        }
                                        Ok(_) => None,
                                        Err(e) => {
                                            tracing::warn!(
                                                %e,
                                                "failed to convert history"
                                            );
                                            None
                                        }
                                    };

                                let resume_text = format!(
                                    "Child agent {child_id} completed. \
                                     Process the result above and continue."
                                );

                                let stream_handle =
                                    inner.stream_hub.open(session_id.clone());

                                let turn_result =
                                    crate::process_loop::run_agent_turn(
                                        &handle,
                                        resume_text,
                                        history,
                                        &stream_handle,
                                    )
                                    .await;

                                inner
                                    .stream_hub
                                    .close(stream_handle.stream_id());

                                match turn_result {
                                    Ok(turn) if !turn.text.is_empty() => {
                                        let assistant_msg =
                                            crate::channel::types::ChatMessage::assistant(
                                                &turn.text,
                                            );
                                        conversation
                                            .push(assistant_msg.clone());
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
                                                "failed to persist assistant \
                                                 message after child result"
                                            );
                                        }

                                        let agent_result = AgentResult {
                                            output: turn.text.clone(),
                                            iterations: turn.iterations,
                                            tool_calls: turn.tool_calls,
                                        };
                                        let _ = inner
                                            .process_table
                                            .set_result(
                                                agent_id,
                                                agent_result.clone(),
                                            );

                                        info!(
                                            agent_id = %agent_id,
                                            iterations = agent_result.iterations,
                                            tool_calls = agent_result.tool_calls,
                                            "child result processed"
                                        );
                                        last_result = Some(agent_result);
                                    }
                                    Ok(_) => {
                                        // Empty result — nothing to update.
                                    }
                                    Err(err_msg) => {
                                        tracing::error!(
                                            agent_id = %agent_id,
                                            error = %err_msg,
                                            "LLM turn failed after child \
                                             result"
                                        );
                                        let _ = inner
                                            .process_table
                                            .set_state(
                                                agent_id,
                                                ProcessState::Failed,
                                            );
                                    }
                                }

                                // Return to waiting for more messages.
                                let _ = inner.process_table.set_state(
                                    agent_id,
                                    ProcessState::Waiting,
                                );
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

            let final_result = last_result.unwrap_or(AgentResult {
                output:     "process ended".to_string(),
                iterations: 0,
                tool_calls: 0,
            });

            // Notify parent process via ChildResult message if this is a
            // child agent.  This allows the parent's process_loop to resume
            // LLM execution with the child's output.
            if let Some(pid) = parent_id {
                if let Some(parent_mailbox) = inner.process_table.get_mailbox(&pid) {
                    let child_result_msg =
                        crate::process::ProcessMessage::ChildResult {
                            child_id: agent_id,
                            result:   final_result.clone(),
                        };
                    if let Err(e) = parent_mailbox.send(child_result_msg).await {
                        tracing::warn!(
                            agent_id = %agent_id,
                            parent_id = %pid,
                            error = %e,
                            "failed to send ChildResult to parent mailbox"
                        );
                    }
                }
            }

            let _ = result_tx.send(final_result);
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
    /// Scheduler configuration (priority queue + token budgets).
    pub scheduler:              crate::scheduler::SchedulerConfig,
    /// Maximum number of KV entries per agent (0 = unlimited).
    /// Applies to the agent-scoped namespace only.
    pub memory_quota_per_agent: usize,
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            max_concurrency:        16,
            default_child_limit:    8,
            default_max_iterations: 25,
            scheduler:              crate::scheduler::SchedulerConfig::default(),
            memory_quota_per_agent: 1000,
        }
    }
}

/// The unified agent orchestrator.
///
/// Acts as an OS kernel for agents: manages the process table, enforces
/// concurrency limits, and provides `spawn()` as the primary entry point.
///
/// The Kernel owns its I/O subsystem: inbound bus, outbound bus, stream hub,
/// endpoint registry, and ingress pipeline. Call [`start()`](Self::start) to
/// spawn TickLoop and Egress as background tasks.
pub struct Kernel {
    /// Shared kernel internals (process table, components, etc.).
    inner:  Arc<KernelInner>,
    /// Kernel configuration.
    config: KernelConfig,
    /// Inbound message bus (shared with IngressPipeline and TickLoop).
    inbound_bus:       Arc<dyn InboundBus>,
    /// Outbound message bus (shared with process_loop and Egress).
    outbound_bus:      Arc<dyn OutboundBus>,
    /// Ephemeral stream hub for real-time token deltas.
    stream_hub:        Arc<StreamHub>,
    /// Ingress pipeline (implements InboundSink for adapters).
    ingress_pipeline:  Arc<IngressPipeline>,
    /// Per-user endpoint registry (tracks connected channels).
    endpoint_registry: Arc<EndpointRegistry>,
    /// Registered egress adapters (mutable before start, consumed by start).
    egress_adapters:   HashMap<ChannelType, Arc<dyn EgressAdapter>>,
    /// Shared priority scheduler for LLM call rate limiting.
    scheduler:         Arc<Mutex<crate::scheduler::PriorityScheduler>>,
}

impl Kernel {
    /// Create a new Kernel with the given configuration, components, and I/O
    /// subsystem.
    ///
    /// The I/O subsystem is fully assembled at construction time -- no
    /// `set_io_context()` needed. Call [`start()`](Self::start) to spawn
    /// background tasks (TickLoop, Egress).
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
        session_repo: Arc<dyn SessionRepository>,
        model_repo: Arc<dyn crate::model_repo::ModelRepo>,
        inbound_bus: Arc<dyn InboundBus>,
        outbound_bus: Arc<dyn OutboundBus>,
        stream_hub: Arc<StreamHub>,
        identity_resolver: Arc<dyn IdentityResolver>,
        session_resolver: Arc<dyn SessionResolver>,
    ) -> Self {
        info!(
            max_concurrency = config.max_concurrency,
            default_child_limit = config.default_child_limit,
            default_max_iterations = config.default_max_iterations,
            "booting kernel"
        );

        let endpoint_registry = Arc::new(EndpointRegistry::new());

        let ingress_pipeline = Arc::new(IngressPipeline::new(
            identity_resolver,
            session_resolver,
            inbound_bus.clone(),
        ));

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
            memory_quota_per_agent: config.memory_quota_per_agent,
            user_store,
            session_repo,
            model_repo,
            stream_hub: stream_hub.clone(),
            outbound_bus: outbound_bus.clone(),
        });

        let scheduler = Arc::new(Mutex::new(
            crate::scheduler::PriorityScheduler::new(config.scheduler.clone()),
        ));

        Self {
            inner,
            config,
            inbound_bus,
            outbound_bus,
            stream_hub,
            ingress_pipeline,
            endpoint_registry,
            egress_adapters: HashMap::new(),
            scheduler,
        }
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

    /// Access the model repository for runtime model resolution.
    pub fn model_repo(&self) -> &Arc<dyn crate::model_repo::ModelRepo> { &self.inner.model_repo }

    /// Access the shared priority scheduler.
    ///
    /// Used by the process loop to record token usage after LLM calls, and
    /// by the tick loop to enqueue/drain messages.
    pub fn scheduler(&self) -> &Arc<Mutex<crate::scheduler::PriorityScheduler>> { &self.scheduler }

    /// Access the shared KernelInner (for constructing ScopedKernelHandles
    /// externally).
    pub(crate) fn inner(&self) -> &Arc<KernelInner> { &self.inner }

    /// Construct a `Kernel` from a pre-built `KernelInner` and config.
    ///
    /// Used by [`crate::testing::TestKernelBuilder`] to assemble kernels in
    /// tests without going through the public `new()` constructor.
    ///
    /// Creates minimal I/O subsystem components (InboundBus, IngressPipeline,
    /// EndpointRegistry) with Noop resolvers. The OutboundBus and StreamHub
    /// are cloned from `KernelInner`.
    pub(crate) fn from_inner(inner: Arc<KernelInner>, config: KernelConfig) -> Self {
        use crate::io::memory_bus::InMemoryInboundBus;

        let inbound_bus: Arc<dyn InboundBus> = Arc::new(InMemoryInboundBus::new(128));
        let identity_resolver: Arc<dyn IdentityResolver> =
            Arc::new(crate::defaults::noop::NoopIdentityResolver);
        let session_resolver: Arc<dyn SessionResolver> =
            Arc::new(crate::defaults::noop::NoopSessionResolver);
        let ingress_pipeline = Arc::new(IngressPipeline::new(
            identity_resolver,
            session_resolver,
            inbound_bus.clone(),
        ));
        let endpoint_registry = Arc::new(EndpointRegistry::new());

        let scheduler = Arc::new(Mutex::new(
            crate::scheduler::PriorityScheduler::new(config.scheduler.clone()),
        ));

        Self {
            outbound_bus: inner.outbound_bus.clone(),
            stream_hub: inner.stream_hub.clone(),
            inner,
            config,
            inbound_bus,
            ingress_pipeline,
            endpoint_registry,
            egress_adapters: HashMap::new(),
            scheduler,
        }
    }

    // -- I/O subsystem accessors -----------------------------------------

    /// Access the ingress pipeline (adapters need this to push messages).
    pub fn ingress_pipeline(&self) -> &Arc<IngressPipeline> { &self.ingress_pipeline }

    /// Access the ephemeral stream hub (WebAdapter needs this for token
    /// deltas).
    pub fn stream_hub(&self) -> &Arc<StreamHub> { &self.stream_hub }

    /// Access the endpoint registry (WebAdapter needs this for connection
    /// tracking).
    pub fn endpoint_registry(&self) -> &Arc<EndpointRegistry> { &self.endpoint_registry }

    /// Access the inbound bus (for monitoring / pending count).
    pub fn inbound_bus(&self) -> &Arc<dyn InboundBus> { &self.inbound_bus }

    /// Register an egress adapter for a channel type.
    ///
    /// Must be called **before** [`start()`](Self::start).
    pub fn register_adapter(&mut self, channel_type: ChannelType, adapter: Arc<dyn EgressAdapter>) {
        self.egress_adapters.insert(channel_type, adapter);
    }

    /// Spawn TickLoop and Egress as background tasks.
    ///
    /// Consumes `self` by value, wraps it in `Arc`, spawns background tasks,
    /// and returns the shared `Arc<Kernel>` for callers to use.
    ///
    /// The returned `Arc<Kernel>` can be used to access the ingress pipeline,
    /// stream hub, endpoint registry, etc. The background tasks run until the
    /// `cancel_token` is cancelled.
    pub fn start(mut self, cancel_token: CancellationToken) -> Arc<Self> {
        let adapters = std::mem::take(&mut self.egress_adapters);
        let kernel = Arc::new(self);

        // TickLoop
        let tick_loop = crate::tick::TickLoop::new(
            kernel.inbound_bus.clone(),
            kernel.clone(),
        );
        tokio::spawn({
            let token = cancel_token.clone();
            async move {
                tick_loop.run(token).await;
            }
        });

        // Egress
        let outbound_sub = kernel.outbound_bus.subscribe();
        let mut egress = Egress::new(
            adapters,
            kernel.endpoint_registry.clone(),
            outbound_sub,
        );
        tokio::spawn({
            let token = cancel_token;
            async move {
                tokio::select! {
                    _ = egress.run() => {}
                    _ = token.cancelled() => {}
                }
            }
        });

        info!("kernel I/O subsystem started (tick_loop + egress)");
        kernel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        defaults::{
            noop::{NoopEventBus, NoopGuard, NoopMemory, NoopModelRepo, NoopSessionRepository},
            noop_user_store::NoopUserStore,
        },
        io::memory_bus::{InMemoryInboundBus, InMemoryOutboundBus},
        process::principal::Principal,
        provider::EnvLlmProviderLoader,
    };

    fn make_test_kernel(max_concurrency: usize, child_limit: usize) -> Kernel {
        let config = KernelConfig {
            max_concurrency,
            default_child_limit: child_limit,
            default_max_iterations: 5,
            memory_quota_per_agent: 1000,
            ..Default::default()
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
            Arc::new(NoopSessionRepository) as Arc<dyn SessionRepository>,
            Arc::new(NoopModelRepo) as Arc<dyn crate::model_repo::ModelRepo>,
            Arc::new(InMemoryInboundBus::new(128)) as Arc<dyn InboundBus>,
            Arc::new(InMemoryOutboundBus::new(64)) as Arc<dyn OutboundBus>,
            Arc::new(StreamHub::new(16)),
            Arc::new(crate::defaults::noop::NoopIdentityResolver) as Arc<dyn IdentityResolver>,
            Arc::new(crate::defaults::noop::NoopSessionResolver) as Arc<dyn SessionResolver>,
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
            max_children:        None,
            max_context_tokens:  None,
            priority:            crate::process::Priority::default(),
            metadata:            serde_json::Value::Null,
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
        assert_eq!(config.memory_quota_per_agent, 1000);
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

    // =======================================================================
    // ChildResult integration tests
    // =======================================================================

    /// Stub LLM provider that records calls and returns a canned response.
    mod child_result_helpers {
        use std::sync::{Arc, Mutex};

        use async_openai::types::chat::{
            ChatChoiceStream, ChatCompletionResponseStream, ChatCompletionStreamResponseDelta,
            CreateChatCompletionRequest, CreateChatCompletionStreamResponse, FinishReason,
        };
        use async_trait::async_trait;
        use futures::stream;

        use crate::{
            error::KernelError,
            provider::{LlmProvider, LlmProviderLoader, LlmProviderLoaderRef},
        };

        /// Records the number of messages in each LLM call so tests can assert
        /// the conversation length.
        #[derive(Default)]
        pub struct StubLlm {
            pub call_counts: Mutex<Vec<usize>>,
        }

        #[async_trait]
        impl LlmProvider for StubLlm {
            async fn chat_completion(
                &self,
                _req: CreateChatCompletionRequest,
            ) -> crate::error::Result<
                async_openai::types::chat::CreateChatCompletionResponse,
            > {
                Err(KernelError::Other {
                    message: "not supported".into(),
                })
            }

            #[allow(deprecated)]
            async fn chat_completion_stream(
                &self,
                req: CreateChatCompletionRequest,
            ) -> crate::error::Result<ChatCompletionResponseStream> {
                self.call_counts
                    .lock()
                    .unwrap()
                    .push(req.messages.len());

                let chunk = CreateChatCompletionStreamResponse {
                    id:                 "resp_child".to_string(),
                    choices:            vec![ChatChoiceStream {
                        index:         0,
                        delta:         ChatCompletionStreamResponseDelta {
                            content:       Some(
                                "processed child result".to_string(),
                            ),
                            function_call: None,
                            tool_calls:    None,
                            role:          None,
                            refusal:       None,
                        },
                        finish_reason: Some(FinishReason::Stop),
                        logprobs:      None,
                    }],
                    created:            0,
                    model:              "test-model".to_string(),
                    service_tier:       None,
                    system_fingerprint: None,
                    object:             "chat.completion.chunk".to_string(),
                    usage:              None,
                };
                Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
            }
        }

        #[derive(Clone)]
        pub struct StubLoader {
            pub provider: Arc<dyn LlmProvider>,
        }

        #[async_trait]
        impl LlmProviderLoader for StubLoader {
            async fn acquire_provider(
                &self,
            ) -> crate::error::Result<Arc<dyn LlmProvider>> {
                Ok(Arc::clone(&self.provider))
            }
        }

        pub fn make_llm() -> (Arc<StubLlm>, LlmProviderLoaderRef) {
            let llm = Arc::new(StubLlm::default());
            let loader = Arc::new(StubLoader {
                provider: llm.clone() as Arc<dyn LlmProvider>,
            }) as LlmProviderLoaderRef;
            (llm, loader)
        }
    }

    /// Helper: build a kernel with a stub LLM that records calls.
    fn make_kernel_with_stub_llm(
    ) -> (Kernel, Arc<child_result_helpers::StubLlm>) {
        let (llm, loader) = child_result_helpers::make_llm();
        let config = KernelConfig {
            max_concurrency:        16,
            default_child_limit:    5,
            default_max_iterations: 5,
            ..Default::default()
        };
        let mut manifest_loader = ManifestLoader::new();
        manifest_loader.load_bundled();

        let kernel = Kernel::new(
            config,
            loader,
            Arc::new(ToolRegistry::new()),
            Arc::new(NoopMemory),
            Arc::new(NoopEventBus),
            Arc::new(NoopGuard),
            manifest_loader,
            Arc::new(NoopUserStore),
            Arc::new(NoopSessionRepository)
                as Arc<dyn SessionRepository>,
            Arc::new(NoopModelRepo)
                as Arc<dyn crate::model_repo::ModelRepo>,
            Arc::new(InMemoryInboundBus::new(128))
                as Arc<dyn InboundBus>,
            Arc::new(InMemoryOutboundBus::new(64))
                as Arc<dyn OutboundBus>,
            Arc::new(StreamHub::new(16)),
            Arc::new(crate::defaults::noop::NoopIdentityResolver)
                as Arc<dyn IdentityResolver>,
            Arc::new(crate::defaults::noop::NoopSessionResolver)
                as Arc<dyn SessionResolver>,
        );
        (kernel, llm)
    }

    #[tokio::test]
    async fn test_child_result_delivered_to_parent_mailbox() {
        // Spawn a parent process, then a child.  When the child's process
        // loop ends (after mailbox is closed) it should deliver a
        // ChildResult to the parent's mailbox.
        let (kernel, llm) = make_kernel_with_stub_llm();

        let principal = Principal::user("test-user");
        let session_id = SessionId::new("cr-test-session");

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

        let child_agent_id = child_handle.agent_id;

        // Wait for the child's initial turn to complete (enters Waiting).
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let calls_before = llm.call_counts.lock().unwrap().len();

        // Force the child to terminate by removing it from the process
        // table (which drops the mailbox sender in the table), then
        // dropping the handle's mailbox sender.  This closes the mailbox
        // channel, causing the child's process_loop to exit and send
        // ChildResult to the parent.
        kernel.process_table().remove(child_agent_id);
        drop(child_handle);

        // Wait for the child's cleanup to run and the parent to process
        // the ChildResult.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // The parent should have made an additional LLM call from
        // processing the ChildResult.
        let calls_after = llm.call_counts.lock().unwrap().len();
        assert!(
            calls_after > calls_before,
            "parent should have made additional LLM call after child \
             result delivery: before={calls_before}, after={calls_after}"
        );

        // Parent should still be alive.
        let parent_process =
            kernel.process_table().get(parent_handle.agent_id);
        assert!(
            parent_process.is_some(),
            "parent process should still exist"
        );
    }

    #[tokio::test]
    async fn test_child_result_triggers_parent_llm_turn() {
        // Verify that receiving a ChildResult causes the parent to make
        // an additional LLM call (beyond the initial UserMessage turn).
        let (kernel, llm) = make_kernel_with_stub_llm();

        let principal = Principal::user("test-user");
        let session_id = SessionId::new("cr-llm-test");

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

        // Wait a bit for the parent's initial turn to complete.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let initial_calls = llm.call_counts.lock().unwrap().len();

        // Directly send a ChildResult to the parent's mailbox.
        let child_id = crate::process::AgentId::new();
        let child_result = crate::process::AgentResult {
            output:     "child output text".to_string(),
            iterations: 2,
            tool_calls: 1,
        };
        kernel
            .process_table()
            .get_mailbox(&parent_handle.agent_id)
            .unwrap()
            .send(crate::process::ProcessMessage::ChildResult {
                child_id,
                result: child_result,
            })
            .await
            .unwrap();

        // Give the parent time to process the ChildResult and make the
        // follow-up LLM call.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let total_calls = llm.call_counts.lock().unwrap().len();
        assert!(
            total_calls > initial_calls,
            "parent should have made additional LLM call(s) after \
             ChildResult: initial={initial_calls}, total={total_calls}"
        );
    }

    #[tokio::test]
    async fn test_child_result_parent_state_transitions() {
        // Verify parent state transitions:
        //   Running → (initial turn) → Waiting → (ChildResult) → Running
        //   → (LLM turn) → Waiting
        let (kernel, _llm) = make_kernel_with_stub_llm();

        let principal = Principal::user("test-user");
        let session_id = SessionId::new("cr-state-test");

        let parent_handle = kernel
            .spawn_with_input(
                test_manifest("parent"),
                "parent task".to_string(),
                principal,
                session_id,
                None,
            )
            .await
            .unwrap();

        // Wait for initial turn to complete → Waiting.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let state = kernel
            .process_table()
            .get(parent_handle.agent_id)
            .unwrap()
            .state;
        assert_eq!(
            state,
            ProcessState::Waiting,
            "parent should be Waiting after initial turn"
        );

        // Send a ChildResult.
        let child_id = crate::process::AgentId::new();
        kernel
            .process_table()
            .get_mailbox(&parent_handle.agent_id)
            .unwrap()
            .send(crate::process::ProcessMessage::ChildResult {
                child_id,
                result: crate::process::AgentResult {
                    output:     "done".to_string(),
                    iterations: 1,
                    tool_calls: 0,
                },
            })
            .await
            .unwrap();

        // After processing the ChildResult + follow-up LLM turn, the
        // parent should be back to Waiting.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let state = kernel
            .process_table()
            .get(parent_handle.agent_id)
            .unwrap()
            .state;
        assert_eq!(
            state,
            ProcessState::Waiting,
            "parent should return to Waiting after processing ChildResult"
        );
    }

    #[tokio::test]
    async fn test_child_result_error_handling() {
        // When a child's output is empty (e.g., failure case), the parent
        // should still process it gracefully.
        let (kernel, _llm) = make_kernel_with_stub_llm();

        let principal = Principal::user("test-user");
        let session_id = SessionId::new("cr-error-test");

        let parent_handle = kernel
            .spawn_with_input(
                test_manifest("parent"),
                "parent task".to_string(),
                principal,
                session_id,
                None,
            )
            .await
            .unwrap();

        // Wait for initial turn to complete.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Send a ChildResult with empty/error output.
        let child_id = crate::process::AgentId::new();
        kernel
            .process_table()
            .get_mailbox(&parent_handle.agent_id)
            .unwrap()
            .send(crate::process::ProcessMessage::ChildResult {
                child_id,
                result: crate::process::AgentResult {
                    output:     String::new(),
                    iterations: 0,
                    tool_calls: 0,
                },
            })
            .await
            .unwrap();

        // Parent should handle it gracefully and return to Waiting.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let parent_process =
            kernel.process_table().get(parent_handle.agent_id);
        assert!(
            parent_process.is_some(),
            "parent should still exist after empty child result"
        );
        let state = parent_process.unwrap().state;
        assert_eq!(
            state,
            ProcessState::Waiting,
            "parent should be Waiting after processing empty child result"
        );
    }

    #[tokio::test]
    async fn test_child_result_multiple_children() {
        // Parent receives ChildResult from multiple children sequentially.
        let (kernel, llm) = make_kernel_with_stub_llm();

        let principal = Principal::user("test-user");
        let session_id = SessionId::new("cr-multi-test");

        let parent_handle = kernel
            .spawn_with_input(
                test_manifest("parent"),
                "parent task".to_string(),
                principal,
                session_id,
                None,
            )
            .await
            .unwrap();

        // Wait for initial turn.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let calls_after_init = llm.call_counts.lock().unwrap().len();

        // Send two ChildResult messages.
        for i in 0..2 {
            let child_id = crate::process::AgentId::new();
            kernel
                .process_table()
                .get_mailbox(&parent_handle.agent_id)
                .unwrap()
                .send(crate::process::ProcessMessage::ChildResult {
                    child_id,
                    result: crate::process::AgentResult {
                        output:     format!("child {i} output"),
                        iterations: 1,
                        tool_calls: 0,
                    },
                })
                .await
                .unwrap();
            // Give time for each to be processed sequentially.
            tokio::time::sleep(std::time::Duration::from_millis(300))
                .await;
        }

        let total_calls = llm.call_counts.lock().unwrap().len();
        assert!(
            total_calls >= calls_after_init + 2,
            "parent should have made at least 2 additional LLM calls \
             for 2 child results: init={calls_after_init}, total={total_calls}"
        );

        let state = kernel
            .process_table()
            .get(parent_handle.agent_id)
            .unwrap()
            .state;
        assert_eq!(
            state,
            ProcessState::Waiting,
            "parent should be Waiting after processing all child results"
        );
    }
}
