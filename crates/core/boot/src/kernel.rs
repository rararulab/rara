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
    io::{
        bus::{InboundBus, OutboundBus},
        ingress::{IdentityResolver, SessionResolver},
        stream::StreamHub,
    },
    kernel::{Kernel, KernelConfig},
    model_repo::ModelRepo,
    process::{manifest_loader::ManifestLoader, user::UserStore},
    provider::LlmProviderLoaderRef,
    session::SessionRepository,
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
/// supply the truly external deps (llm_provider, tool_registry, etc.).
pub struct BootConfig {
    // -- core kernel config --------------------------------------------------
    /// Kernel concurrency / iteration limits.
    pub kernel_config: KernelConfig,
    /// LLM provider loader.
    pub llm_provider:  LlmProviderLoaderRef,
    /// Global tool registry.
    pub tool_registry: Arc<ToolRegistry>,
    /// Agent manifest loader.
    pub manifest_loader: ManifestLoader,
    /// User store for permission checks.
    pub user_store: Arc<dyn UserStore>,
    /// Session repository for conversation history.
    pub session_repo: Arc<dyn SessionRepository>,
    /// Model repository for runtime model resolution.
    pub model_repo: Arc<dyn ModelRepo>,

    // -- I/O capacities (optional, have sensible defaults) -------------------
    /// Inbound bus capacity.
    pub inbound_capacity:  usize,
    /// Outbound bus capacity.
    pub outbound_capacity: usize,
    /// Per-stream broadcast capacity.
    pub stream_capacity:   usize,

    // -- optional overrides for resolvers / components -----------------------
    /// Identity resolver (optional — defaults to `DefaultIdentityResolver`).
    pub identity_resolver: Option<Arc<dyn IdentityResolver>>,
    /// Session resolver (optional — defaults to `DefaultSessionResolver`).
    pub session_resolver:  Option<Arc<dyn SessionResolver>>,
    /// Memory implementation (optional — defaults to NoopMemory).
    pub memory:  Option<Arc<dyn rara_kernel::memory::Memory>>,
    /// Event bus (optional — defaults to BroadcastEventBus).
    pub event_bus: Option<Arc<dyn rara_kernel::event::EventBus>>,
    /// Guard (optional — defaults to NoopGuard).
    pub guard: Option<Arc<dyn rara_kernel::guard::Guard>>,
}

impl Default for BootConfig {
    fn default() -> Self {
        use rara_kernel::defaults::noop::{NoopModelRepo, NoopSessionRepository};
        use rara_kernel::defaults::noop_user_store::NoopUserStore;
        use rara_kernel::provider::EnvLlmProviderLoader;

        Self {
            kernel_config:     KernelConfig::default(),
            llm_provider:      Arc::new(EnvLlmProviderLoader::default()) as LlmProviderLoaderRef,
            tool_registry:     Arc::new(ToolRegistry::new()),
            manifest_loader:   ManifestLoader::new(),
            user_store:        Arc::new(NoopUserStore) as Arc<dyn UserStore>,
            session_repo:      Arc::new(NoopSessionRepository) as Arc<dyn SessionRepository>,
            model_repo:        Arc::new(NoopModelRepo) as Arc<dyn ModelRepo>,
            inbound_capacity:  1024,
            outbound_capacity: 256,
            stream_capacity:   64,
            identity_resolver: None,
            session_resolver:  None,
            memory:            None,
            event_bus:         None,
            guard:             None,
        }
    }
}

// ---------------------------------------------------------------------------
// boot()
// ---------------------------------------------------------------------------

/// Assemble a fully-configured [`Kernel`] with I/O subsystem.
///
/// This is the single entry point for creating a production-ready kernel.
/// The returned `Kernel` owns its buses, stream hub, endpoint registry, and
/// ingress pipeline. Call [`Kernel::register_adapter`] to add egress
/// adapters, then [`Kernel::start`] to spawn background tasks.
pub fn boot(config: BootConfig) -> Kernel {
    // I/O buses
    let inbound_bus: Arc<dyn InboundBus> =
        crate::bus::default_inbound_bus(config.inbound_capacity);
    let outbound_bus: Arc<dyn OutboundBus> =
        crate::bus::default_outbound_bus(config.outbound_capacity);
    let stream_hub: Arc<StreamHub> =
        crate::stream::default_stream_hub(config.stream_capacity);

    // Resolvers
    let identity_resolver: Arc<dyn IdentityResolver> = config
        .identity_resolver
        .unwrap_or_else(|| Arc::new(DefaultIdentityResolver::new()));
    let session_resolver: Arc<dyn SessionResolver> = config
        .session_resolver
        .unwrap_or_else(|| Arc::new(DefaultSessionResolver::new()));

    // Components (use overrides or boot defaults)
    let memory = config
        .memory
        .unwrap_or_else(crate::components::default_memory);
    let event_bus = config
        .event_bus
        .unwrap_or_else(crate::components::default_event_bus);
    let guard = config
        .guard
        .unwrap_or_else(crate::components::default_guard);

    tracing::info!(
        inbound_capacity = config.inbound_capacity,
        outbound_capacity = config.outbound_capacity,
        stream_capacity = config.stream_capacity,
        "booting kernel via boot::kernel::boot()"
    );

    Kernel::new(
        config.kernel_config,
        config.llm_provider,
        config.tool_registry,
        memory,
        event_bus,
        guard,
        config.manifest_loader,
        config.user_store,
        config.session_repo,
        config.model_repo,
        inbound_bus,
        outbound_bus,
        stream_hub,
        identity_resolver,
        session_resolver,
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
    }

    #[test]
    fn test_boot_custom_config() {
        let config = BootConfig {
            kernel_config: KernelConfig {
                max_concurrency:        4,
                default_child_limit:    2,
                default_max_iterations: 10,
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
        let _ = kernel.inbound_bus();
    }
}
