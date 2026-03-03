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

//! Delivery subsystem — egress adapter management and outbound message
//! delivery.
//!
//! Extracted from `Kernel` to encapsulate the egress adapter map and
//! endpoint registry behind a single sub-component.

use std::{collections::HashMap, sync::Arc};

use tracing::Instrument;

use crate::{
    channel::types::ChannelType,
    io::{
        egress::{EgressAdapterRef, EndpointRegistryRef},
        types::{InboundMessage, OutboundEnvelope, OutboundPayload},
    },
    security::SecurityRef,
};

/// Manages egress adapters and the endpoint registry for outbound message
/// delivery.
///
/// Owns the `egress_adapters` map (previously on `Kernel`) and the
/// `endpoint_registry`. Provides `deliver()` for fire-and-forget outbound
/// delivery and `register_endpoint()` for stateless channel registration.
pub(crate) struct DeliverySubsystem {
    /// Registered egress adapters keyed by channel type.
    egress_adapters: HashMap<ChannelType, EgressAdapterRef>,
    /// Per-user endpoint registry (tracks connected channels).
    endpoint_registry: EndpointRegistryRef,
}

impl DeliverySubsystem {
    /// Create a new delivery subsystem.
    pub fn new(endpoint_registry: EndpointRegistryRef) -> Self {
        Self {
            egress_adapters: HashMap::new(),
            endpoint_registry,
        }
    }

    /// Register an egress adapter for a channel type.
    ///
    /// Must be called **before** the kernel event loop starts.
    pub fn register_adapter(&mut self, channel_type: ChannelType, adapter: EgressAdapterRef) {
        self.egress_adapters.insert(channel_type, adapter);
    }

    /// Access the egress adapters map.
    pub fn egress_adapters(&self) -> &HashMap<ChannelType, EgressAdapterRef> {
        &self.egress_adapters
    }

    /// Access the endpoint registry.
    pub fn endpoint_registry(&self) -> &EndpointRegistryRef {
        &self.endpoint_registry
    }

    /// Spawn a Deliver event as an independent task so that egress I/O
    /// (Telegram API, WebSocket send, etc.) does not block the event loop.
    pub fn deliver(&self, envelope: OutboundEnvelope, security: &SecurityRef) {
        let adapters = self.egress_adapters.clone();
        let endpoints = Arc::clone(&self.endpoint_registry);
        let user_store = Arc::clone(security.user_store());

        let payload_type = match &envelope.payload {
            OutboundPayload::Reply { .. } => "reply",
            OutboundPayload::Progress { .. } => "progress",
            OutboundPayload::StateChange { .. } => "state_change",
            OutboundPayload::Error { .. } => "error",
        };
        let span = tracing::info_span!(
            "deliver",
            session_id = %envelope.session_id,
            payload_type,
        );

        tokio::spawn(
            async move {
                crate::io::egress::Egress::deliver(
                    &adapters,
                    &endpoints,
                    Some(user_store.as_ref()),
                    envelope,
                )
                .await;
            }
            .instrument(span),
        );
    }

    /// Register egress endpoint for stateless channels (e.g. Telegram).
    ///
    /// Connection-oriented channels (Web) register on WS/SSE connect.
    /// Stateless channels have no persistent connection, so we register on
    /// every inbound message (idempotent — EndpointRegistry uses a HashSet).
    pub fn register_stateless_endpoint(&self, msg: &InboundMessage) {
        if msg.source.channel_type != ChannelType::Telegram {
            return;
        }
        let Some(ref chat_id_str) = msg.source.platform_chat_id else {
            return;
        };
        let Ok(chat_id) = chat_id_str.parse::<i64>() else {
            return;
        };
        self.endpoint_registry.register(
            &msg.user,
            crate::io::egress::Endpoint {
                channel_type: ChannelType::Telegram,
                address:      crate::io::egress::EndpointAddress::Telegram {
                    chat_id,
                    thread_id: None,
                },
            },
        );
    }
}
