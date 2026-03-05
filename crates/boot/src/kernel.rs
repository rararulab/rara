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
    io::{IdentityResolver, SessionResolver, StreamHub},
    kernel::{Kernel, KernelConfig},
    llm::DriverRegistry,
    process::{agent_registry::AgentRegistry, user::UserStore},
    security::{ApprovalManager, ApprovalPolicy},
    session::SessionIndex,
    tool::ToolRegistry,
};

use crate::resolvers::DefaultIdentityResolver;

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
    pub identity_resolver: Option<Arc<dyn IdentityResolver>>,
    /// Session resolver (optional — defaults to `DefaultSessionResolver`).
    pub session_resolver:  Option<Arc<dyn SessionResolver>>,
    /// Notification bus (optional — defaults to BroadcastNotificationBus).
    pub event_bus:         Option<Arc<dyn rara_kernel::notification::NotificationBus>>,

    /// Approval manager (optional — defaults to ApprovalManager with default
    /// policy).
    pub approval:           Option<Arc<ApprovalManager>>,
    /// Event queue sharding configuration (optional — defaults to
    /// single-queue mode via `KernelConfig`).
    pub event_queue_config: Option<rara_kernel::queue::ShardedEventQueueConfig>,
    /// OpenDAL operator for the kernel shared KV store (optional — defaults
    /// to in-memory).
    pub kv_operator:        Option<opendal::Operator>,
}

// ---------------------------------------------------------------------------
// boot()
// ---------------------------------------------------------------------------

// FIXME: boot 本来就是为了去setup一堆kernel的config
// 以及kernel需要的组件，结果又创建了一个boot config，何意味？
/// Assemble a fully-configured [`Kernel`] with I/O subsystem.
///
/// This is the single entry point for creating a production-ready kernel.
/// The returned `Kernel` owns its EventQueue, stream hub, endpoint registry,
/// and ingress pipeline. Call [`Kernel::register_adapter`] to add egress
/// adapters, then [`Kernel::start`] to spawn the unified event loop.
pub fn boot(config: BootConfig) -> Kernel {
    let stream_hub = Arc::new(StreamHub::new(config.stream_capacity));

    // Resolvers
    let identity_resolver: Arc<dyn IdentityResolver> =
        config.identity_resolver.unwrap_or_else(|| {
            Arc::new(DefaultIdentityResolver::new(
                rara_kernel::process::principal::UserId("root".to_string()),
            ))
        });
    let _session_index_for_resolver: Arc<dyn SessionIndex> = config.session_index.clone().unwrap();
    let session_resolver: Arc<dyn SessionResolver> = config.session_resolver.unwrap();

    // Tape store — falls back to a temporary in-memory-like store if not
    // provided by the caller (production code always provides one).
    let tape_store: Arc<rara_memory::tape::FileTapeStore> = config
        .tape_store
        .clone()
        .expect("tape_store must be provided in BootConfig");

    // Components (use overrides or boot defaults)
    let event_bus = config
        .event_bus
        .unwrap_or_else(crate::components::default_event_bus);
    let approval = config
        .approval
        .unwrap_or_else(|| Arc::new(ApprovalManager::new(ApprovalPolicy::default())));

    // Assemble the unified security subsystem.
    let security = Arc::new(rara_kernel::security::SecuritySubsystem::new(
        config.user_store,
        approval,
    ));

    tracing::info!(
        stream_capacity = config.stream_capacity,
        "booting kernel via boot::kernel::boot()"
    );

    let mut kernel_config = config.kernel_config;
    if let Some(eq_config) = config.event_queue_config {
        kernel_config.event_queue = eq_config;
    }

    let session_index: Arc<dyn SessionIndex> = config.session_index.unwrap();

    Kernel::new(
        kernel_config,
        config.driver_registry,
        config.tool_registry,
        tape_store,
        event_bus,
        security,
        config.agent_registry,
        session_index,
        config.settings,
        stream_hub,
        identity_resolver,
        session_resolver,
        config.kv_operator.unwrap_or_else(|| {
            opendal::Operator::new(opendal::services::Memory::default())
                .expect("memory operator")
                .finish()
        }),
    )
}
