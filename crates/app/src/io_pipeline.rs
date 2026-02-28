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

//! I/O Bus pipeline wiring for `rara-app`.
//!
//! This module initializes the I/O Bus pipeline components (IngressPipeline,
//! TickLoop, Egress) and provides the [`IoBusPipeline`] struct that holds
//! all components needed to run the new message pipeline.
//!
//! The pipeline is now the sole message path — the legacy ChatService bridge
//! has been removed (#366).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Semaphore;
use tracing::info;

use rara_kernel::channel::types::ChannelType;
use rara_kernel::io::bus::{InboundBus, OutboundBus};
use rara_kernel::io::egress::{EgressAdapter, Egress, EndpointRegistry};
use rara_kernel::defaults::noop::{NoopOutboxStore, NoopSessionRepository};
use rara_kernel::executor::AgentExecutor;
use rara_kernel::io::ingress::{IdentityResolver, IngressPipeline, SessionResolver};
use rara_kernel::io::stream::StreamHub;
use rara_kernel::tick::TickLoop;
use rara_kernel::process::ProcessTable;
use rara_kernel::provider::{EnvLlmProviderLoader, LlmProviderLoaderRef};
use rara_kernel::tool::ToolRegistry;

use crate::resolvers::{AppIdentityResolver, AppSessionResolver};

// ---------------------------------------------------------------------------
// IoBusPipeline
// ---------------------------------------------------------------------------

/// Holds all I/O Bus pipeline components.
///
/// Created by [`init_io_pipeline`] and used by the app startup to spawn
/// background tasks (TickLoop, Egress).
pub struct IoBusPipeline {
    /// The inbound message bus (shared with IngressPipeline and TickLoop).
    pub inbound_bus: Arc<dyn InboundBus>,
    /// The outbound message bus (shared with AgentExecutor and Egress).
    pub outbound_bus: Arc<dyn OutboundBus>,
    /// Ephemeral stream hub for real-time token deltas.
    pub stream_hub: Arc<StreamHub>,
    /// The ingress pipeline (implements InboundSink for adapters).
    pub ingress_pipeline: Arc<IngressPipeline>,
    /// The kernel tick loop (drains InboundBus, dispatches to executor).
    pub tick_loop: TickLoop,
    /// The egress engine (delivers outbound envelopes to adapters).
    pub egress: Egress,
    /// Per-user endpoint registry (tracks connected channels).
    pub endpoint_registry: Arc<EndpointRegistry>,
}

/// Initialize the full I/O Bus pipeline.
///
/// This creates all components needed for the new message pipeline:
/// 1. InboundBus + OutboundBus (via rara-boot factories)
/// 2. StreamHub + SessionScheduler
/// 3. Identity + Session resolvers
/// 4. IngressPipeline (implements InboundSink)
/// 5. AgentExecutor (processes messages through LLM)
/// 6. TickLoop (drains bus, dispatches to executor)
/// 7. Egress (delivers responses to adapters)
///
/// The `telegram_adapter` is optional — if provided, it is registered as
/// an [`EgressAdapter`] for outbound delivery.
pub fn init_io_pipeline(
    telegram_adapter: Option<Arc<rara_channels::telegram::TelegramAdapter>>,
) -> IoBusPipeline {
    // 1. Create buses
    let inbound_bus = rara_boot::bus::default_inbound_bus(1024);
    let outbound_bus = rara_boot::bus::default_outbound_bus(256);
    let stream_hub = rara_boot::stream::default_stream_hub(64);
    let session_scheduler = rara_boot::scheduler::default_session_scheduler(16);

    // 2. Create resolvers
    let identity_resolver: Arc<dyn IdentityResolver> = Arc::new(AppIdentityResolver::new());
    let session_resolver: Arc<dyn SessionResolver> = Arc::new(AppSessionResolver::new());

    // 3. Create IngressPipeline
    let ingress_pipeline = Arc::new(IngressPipeline::new(
        identity_resolver,
        session_resolver,
        inbound_bus.clone(),
    ));

    // 4. Create SessionManager with noop repo for now
    let session_manager = rara_boot::session::default_session_manager(
        Arc::new(NoopSessionRepository),
    );

    // 5. Create AgentExecutor
    let executor = Arc::new(AgentExecutor::new(
        ProcessTable::new(),
        Arc::new(Semaphore::new(16)), // global concurrency limit
        session_scheduler.clone(),
        inbound_bus.clone(),
        outbound_bus.clone(),
        Arc::new(NoopOutboxStore),
        stream_hub.clone(),
        session_manager,
        Arc::new(EnvLlmProviderLoader::default()) as LlmProviderLoaderRef,
        Arc::new(ToolRegistry::new()),
    ));

    // 6. Create TickLoop
    let tick_loop = TickLoop::new(
        inbound_bus.clone(),
        session_scheduler,
        executor,
    );

    // 7. Create Egress
    let endpoint_registry = Arc::new(EndpointRegistry::new());
    let outbound_sub = outbound_bus.subscribe();

    let mut adapters: HashMap<ChannelType, Arc<dyn EgressAdapter>> = HashMap::new();
    if let Some(ref tg) = telegram_adapter {
        adapters.insert(ChannelType::Telegram, tg.clone() as Arc<dyn EgressAdapter>);
    }

    let egress = Egress::new(adapters, endpoint_registry.clone(), outbound_sub);

    info!(
        inbound_capacity = 1024,
        outbound_capacity = 256,
        stream_capacity = 64,
        max_pending_per_session = 16,
        adapters = if telegram_adapter.is_some() { "telegram" } else { "none" },
        "I/O Bus pipeline initialized"
    );

    IoBusPipeline {
        inbound_bus,
        outbound_bus,
        stream_hub,
        ingress_pipeline,
        tick_loop,
        egress,
        endpoint_registry,
    }
}
