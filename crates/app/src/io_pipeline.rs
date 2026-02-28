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
//! The pipeline uses the Kernel's long-lived process model — the TickLoop
//! routes messages to existing processes via mailbox or spawns new ones.

use std::{collections::HashMap, sync::Arc};

use rara_kernel::{
    channel::types::ChannelType,
    io::{
        bus::{InboundBus, OutboundBus},
        egress::{Egress, EgressAdapter, EndpointRegistry},
        ingress::{IdentityResolver, IngressPipeline, SessionResolver},
        stream::StreamHub,
    },
    kernel::Kernel,
    tick::TickLoop,
};
use tracing::info;

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
    pub inbound_bus:       Arc<dyn InboundBus>,
    /// The outbound message bus (shared with process_loop and Egress).
    pub outbound_bus:      Arc<dyn OutboundBus>,
    /// Ephemeral stream hub for real-time token deltas.
    pub stream_hub:        Arc<StreamHub>,
    /// The ingress pipeline (implements InboundSink for adapters).
    pub ingress_pipeline:  Arc<IngressPipeline>,
    /// The kernel tick loop (drains InboundBus, routes to processes).
    pub tick_loop:         TickLoop,
    /// The egress engine (delivers outbound envelopes to adapters).
    pub egress:            Egress,
    /// Per-user endpoint registry (tracks connected channels).
    pub endpoint_registry: Arc<EndpointRegistry>,
    /// The kernel (owns the process table).
    pub kernel:            Arc<Kernel>,
    /// The web adapter (if created).
    pub web_adapter:       Option<Arc<rara_channels::web::WebAdapter>>,
}

/// Initialize the full I/O Bus pipeline.
///
/// This creates all components needed for the new message pipeline:
/// 1. InboundBus + OutboundBus (via rara-boot factories)
/// 2. StreamHub
/// 3. Identity + Session resolvers
/// 4. IngressPipeline (implements InboundSink)
/// 5. Kernel with IO context (session_repo, stream_hub, outbound_bus)
/// 6. TickLoop (drains bus, routes to Kernel processes)
/// 7. Egress (delivers responses to adapters)
///
/// The `telegram_adapter` is optional — if provided, it is registered as
/// an [`EgressAdapter`] for outbound delivery.
pub fn init_io_pipeline(
    telegram_adapter: Option<Arc<rara_channels::telegram::TelegramAdapter>>,
    web_adapter: Option<Arc<rara_channels::web::WebAdapter>>,
    session_repo: Arc<dyn rara_kernel::session::SessionRepository>,
    mut kernel: Kernel,
) -> IoBusPipeline {
    // 1. Create buses
    let inbound_bus = rara_boot::bus::default_inbound_bus(1024);
    let outbound_bus = rara_boot::bus::default_outbound_bus(256);
    let stream_hub = rara_boot::stream::default_stream_hub(64);

    // 2. Create resolvers
    let identity_resolver: Arc<dyn IdentityResolver> = Arc::new(AppIdentityResolver::new());
    let session_resolver: Arc<dyn SessionResolver> = Arc::new(AppSessionResolver::new());

    // 3. Create IngressPipeline
    let ingress_pipeline = Arc::new(IngressPipeline::new(
        identity_resolver,
        session_resolver,
        inbound_bus.clone(),
    ));

    // 4. Set IO context on kernel (session_repo used directly, no SessionManager wrapper)
    kernel.set_io_context(session_repo, stream_hub.clone(), outbound_bus.clone());
    let kernel = Arc::new(kernel);

    // 6. Create TickLoop
    let tick_loop = TickLoop::new(inbound_bus.clone(), kernel.clone());

    // 7. Create Egress
    let endpoint_registry = Arc::new(EndpointRegistry::new());
    let outbound_sub = outbound_bus.subscribe();

    let mut adapters: HashMap<ChannelType, Arc<dyn EgressAdapter>> = HashMap::new();
    if let Some(ref tg) = telegram_adapter {
        adapters.insert(ChannelType::Telegram, tg.clone() as Arc<dyn EgressAdapter>);
    }
    if let Some(ref web) = web_adapter {
        adapters.insert(ChannelType::Web, web.clone() as Arc<dyn EgressAdapter>);
    }

    let egress = Egress::new(adapters, endpoint_registry.clone(), outbound_sub);

    {
        let mut adapter_names = Vec::new();
        if telegram_adapter.is_some() {
            adapter_names.push("telegram");
        }
        if web_adapter.is_some() {
            adapter_names.push("web");
        }
        let adapters_str = if adapter_names.is_empty() {
            "none".to_owned()
        } else {
            adapter_names.join(", ")
        };
        info!(
            inbound_capacity = 1024,
            outbound_capacity = 256,
            stream_capacity = 64,
            adapters = %adapters_str,
            "I/O Bus pipeline initialized"
        );
    }

    IoBusPipeline {
        inbound_bus,
        outbound_bus,
        stream_hub,
        ingress_pipeline,
        tick_loop,
        egress,
        endpoint_registry,
        kernel,
        web_adapter,
    }
}
