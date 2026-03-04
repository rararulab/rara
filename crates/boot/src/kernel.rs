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

//! Kernel boot — assembles a fully-configured [`Kernel`] from external
//! dependencies.
//!
//! ```rust,ignore
//! let kernel = boot::kernel::boot(BootConfig { ... });
//! kernel.register_adapter(ChannelType::Web, web_adapter);
//! kernel.start(cancel_token);
//! ```

use std::sync::Arc;

use rara_kernel::{
    audit::{AuditLog, InMemoryAuditLog},
    io::{
        ingress::{IdentityResolver, SessionResolver},
        stream::StreamHub,
    },
    kernel::{Kernel, KernelConfig},
    llm::DriverRegistry,
    memory::{Memory, NoopMemory},
    process::{agent_registry::AgentRegistry, user::UserStore},
    security::{ApprovalManager, ApprovalPolicy},
    session::{SessionIndex, SessionRepository},
    tool::ToolRegistry,
};

use crate::resolvers::{DefaultIdentityResolver, DefaultSessionResolver};

// ---------------------------------------------------------------------------
// BootConfig
// ---------------------------------------------------------------------------

/// Configuration for [`boot()`] — everything the kernel needs from the
/// outside world.
///
/// Fields with sensible defaults are set via `Default`; callers only need to
/// supply the truly external deps (driver_registry, tool_registry, etc.).
pub struct BootConfig {
    // -- core kernel config --------------------------------------------------
    /// Kernel concurrency / iteration limits.
    pub kernel_config:   KernelConfig,
    /// Multi-driver LLM registry.
    pub driver_registry: Arc<DriverRegistry>,
    /// Global tool registry.
    pub tool_registry:   Arc<ToolRegistry>,
    /// Agent registry.
    pub agent_registry:  Arc<AgentRegistry>,
    /// User store for permission checks.
    pub user_store:      Arc<dyn UserStore>,
    /// Session repository for conversation history (legacy).
    pub session_repo:    Arc<dyn SessionRepository>,
    /// Lightweight session metadata index (tape-centric replacement).
    pub session_index:   Option<Arc<dyn SessionIndex>>,
    /// File-backed tape store (tape-centric session storage).
    pub tape_store:      Option<Arc<rara_memory::tape::FileTapeStore>>,
    /// Flat KV settings provider.
    pub settings:        Arc<dyn rara_domain_shared::settings::SettingsProvider>,

    // -- I/O capacities (optional, have sensible defaults) -------------------
    /// Per-stream broadcast capacity.
    pub stream_capacity: usize,

    // -- optional overrides for resolvers / components -----------------------
    /// Identity resolver (optional — defaults to `DefaultIdentityResolver`).
    pub identity_resolver:  Option<Arc<dyn IdentityResolver>>,
    /// Session resolver (optional — defaults to `DefaultSessionResolver`).
    pub session_resolver:   Option<Arc<dyn SessionResolver>>,
    /// Notification bus (optional — defaults to BroadcastNotificationBus).
    pub event_bus:          Option<Arc<dyn rara_kernel::notification::NotificationBus>>,
    /// Guard (optional — defaults to NoopGuard).
    pub guard:              Option<Arc<dyn rara_kernel::guard::Guard>>,
    /// Audit log (optional — defaults to InMemoryAuditLog).
    pub audit_log:          Option<Arc<dyn AuditLog>>,
    /// Approval manager (optional — defaults to ApprovalManager with default
    /// policy).
    pub approval:           Option<Arc<ApprovalManager>>,
    /// Event queue sharding configuration (optional — defaults to
    /// single-queue mode via `KernelConfig`).
    pub event_queue_config: Option<rara_kernel::queue::ShardedEventQueueConfig>,
    /// OpenDAL operator for the kernel shared KV store (optional — defaults
    /// to in-memory).
    pub kv_operator:        Option<opendal::Operator>,
    /// Tool call recorder (optional — defaults to NoopToolCallRecorder).
    pub tool_call_recorder: Option<Arc<dyn rara_kernel::audit::ToolCallRecorder>>,
}

impl Default for BootConfig {
    fn default() -> Self {
        use rara_kernel::{
            kernel::NoopSettingsProvider, llm::DriverRegistryBuilder,
            process::noop_user_store::NoopUserStore, session::NoopSessionRepository,
        };

        Self {
            kernel_config:      KernelConfig::default(),
            driver_registry:    Arc::new(
                DriverRegistryBuilder::new("default")
                    .provider_model("default", "openai/gpt-4o-mini", vec![])
                    .build(),
            ),
            tool_registry:      Arc::new(ToolRegistry::new()),
            agent_registry:     Arc::new(AgentRegistry::new(
                vec![],
                rara_paths::data_dir().join("agents"),
            )),
            user_store:         Arc::new(NoopUserStore) as Arc<dyn UserStore>,
            session_repo:       Arc::new(NoopSessionRepository) as Arc<dyn SessionRepository>,
            session_index:      None,
            tape_store:         None,
            settings:           Arc::new(NoopSettingsProvider)
                as Arc<dyn rara_domain_shared::settings::SettingsProvider>,
            stream_capacity:    64,
            identity_resolver:  None,
            session_resolver:   None,
            event_bus:          None,
            guard:              None,
            audit_log:          None,
            approval:           None,
            event_queue_config: None,
            kv_operator:        None,
            tool_call_recorder: None,
        }
    }
}

// ---------------------------------------------------------------------------
// boot()
// ---------------------------------------------------------------------------

/// Assemble a fully-configured [`Kernel`] with I/O subsystem.
///
/// This is the single entry point for creating a production-ready kernel.
/// The returned `Kernel` owns its EventQueue, stream hub, endpoint registry,
/// and ingress pipeline. Call [`Kernel::register_adapter`] to add egress
/// adapters, then [`Kernel::start`] to spawn the unified event loop.
pub fn boot(config: BootConfig) -> Kernel {
    let stream_hub: Arc<StreamHub> = crate::stream::default_stream_hub(config.stream_capacity);

    // Resolvers
    let identity_resolver: Arc<dyn IdentityResolver> =
        config.identity_resolver.unwrap_or_else(|| {
            Arc::new(DefaultIdentityResolver::new(
                rara_kernel::process::principal::UserId("root".to_string()),
            ))
        });
    let session_resolver: Arc<dyn SessionResolver> = config
        .session_resolver
        .unwrap_or_else(|| Arc::new(DefaultSessionResolver::new(config.session_repo.clone())));

    // Components (use overrides or boot defaults)
    let memory: Arc<dyn Memory> = Arc::new(NoopMemory);
    let event_bus = config
        .event_bus
        .unwrap_or_else(crate::components::default_event_bus);
    let guard = config
        .guard
        .unwrap_or_else(crate::components::default_guard);
    let audit_log: Arc<dyn AuditLog> = config
        .audit_log
        .unwrap_or_else(|| Arc::new(InMemoryAuditLog::default()));
    let tool_call_recorder: Arc<dyn rara_kernel::audit::ToolCallRecorder> = config
        .tool_call_recorder
        .unwrap_or_else(|| Arc::new(rara_kernel::audit::NoopToolCallRecorder));
    let audit = Arc::new(rara_kernel::audit::AuditSubsystem::new(
        audit_log,
        tool_call_recorder,
    ));
    let approval = config
        .approval
        .unwrap_or_else(|| Arc::new(ApprovalManager::new(ApprovalPolicy::default())));

    // Assemble the unified security subsystem.
    let security = Arc::new(rara_kernel::security::SecuritySubsystem::new(
        config.user_store,
        guard,
        approval,
    ));

    // Eagerly register all Prometheus metrics so /metrics shows them immediately.
    rara_kernel::metrics::init();

    tracing::info!(
        stream_capacity = config.stream_capacity,
        "booting kernel via boot::kernel::boot()"
    );

    let mut kernel_config = config.kernel_config;
    if let Some(eq_config) = config.event_queue_config {
        kernel_config.event_queue = eq_config;
    }

    let session_index: Arc<dyn SessionIndex> = config
        .session_index
        .unwrap_or_else(|| Arc::new(rara_kernel::session::NoopSessionIndex));

    Kernel::new(
        kernel_config,
        config.driver_registry,
        config.tool_registry,
        memory,
        event_bus,
        security,
        config.agent_registry,
        config.session_repo,
        session_index,
        config.settings,
        stream_hub,
        identity_resolver,
        session_resolver,
        audit,
        config.kv_operator.unwrap_or_else(|| {
            opendal::Operator::new(opendal::services::Memory::default())
                .expect("memory operator")
                .finish()
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boot_default_config() {
        let kernel = boot(BootConfig::default());
        assert_eq!(kernel.config().max_concurrency, 16);
        assert_eq!(kernel.config().default_child_limit, 8);
        assert!(!kernel.event_queue().is_sharded());
    }

    #[test]
    fn test_boot_custom_config() {
        let config = BootConfig {
            kernel_config: KernelConfig {
                max_concurrency: 4,
                default_child_limit: 2,
                default_max_iterations: 10,
                memory_quota_per_agent: 1000,
                ..Default::default()
            },
            ..Default::default()
        };
        let kernel = boot(config);
        assert_eq!(kernel.config().max_concurrency, 4);
        assert_eq!(kernel.config().default_child_limit, 2);
        assert_eq!(kernel.config().default_max_iterations, 10);
    }

    #[test]
    fn test_boot_exposes_io_subsystem() {
        let kernel = boot(BootConfig::default());
        // These accessors should not panic
        let _ = kernel.ingress_pipeline();
        let _ = kernel.stream_hub();
        let _ = kernel.endpoint_registry();
        let _ = kernel.event_queue();
    }
}
