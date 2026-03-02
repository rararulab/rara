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

//! Egress — outbound message delivery to channel adapters.
//!
//! The egress layer receives
//! [`OutboundEnvelope`](crate::io::types::OutboundEnvelope) messages from the
//! kernel event loop (via `KernelEvent::Deliver`) and delivers them to the
//! appropriate channel adapters based on the user's connected [`Endpoint`]s.
//!
//! Key components:
//! - [`Endpoint`] / [`EndpointAddress`] — concrete delivery targets
//! - [`EndpointRegistry`] — tracks per-user active endpoints (DashMap-based)
//! - [`EgressAdapter`] — send-only adapter interface for egress
//! - [`Egress`] — main loop consuming the outbound bus and fanning out

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use async_trait::async_trait;
use dashmap::DashMap;
use snafu::Snafu;

use crate::{
    channel::types::ChannelType,
    io::types::{Attachment, OutboundEnvelope, OutboundPayload, OutboundRouting, ReplyContext},
    process::{principal::UserId, user::UserStore},
};

// ---------------------------------------------------------------------------
// Endpoint / EndpointAddress
// ---------------------------------------------------------------------------

/// A concrete deliverable target (not the coarse [`ChannelType`]).
///
/// An endpoint pairs a channel type with a specific address, enabling
/// precise delivery to individual connections (e.g. a specific Telegram
/// chat, a specific WebSocket connection).
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Endpoint {
    /// The channel type of this endpoint.
    pub channel_type: ChannelType,
    /// Platform-specific address details.
    pub address:      EndpointAddress,
}

/// Platform-specific addressing for an [`Endpoint`].
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum EndpointAddress {
    /// Telegram chat endpoint.
    Telegram {
        /// Telegram chat ID.
        chat_id:   i64,
        /// Optional thread ID within the chat.
        thread_id: Option<i64>,
    },
    /// Web (SSE / WebSocket) endpoint.
    Web {
        /// Unique connection identifier.
        connection_id: String,
    },
    /// CLI session endpoint.
    Cli {
        /// CLI session identifier.
        session_id: String,
    },
}

// ---------------------------------------------------------------------------
// EndpointRegistry
// ---------------------------------------------------------------------------

/// Tracks per-user active endpoints.
///
/// Thread-safe via `DashMap`. Adapters register endpoints when a user
/// connects and unregister when they disconnect.
pub struct EndpointRegistry {
    connections: DashMap<UserId, HashSet<Endpoint>>,
}

impl EndpointRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
        }
    }

    /// Register an endpoint for a user.
    pub fn register(&self, user: &UserId, endpoint: Endpoint) {
        self.connections
            .entry(user.clone())
            .or_default()
            .insert(endpoint);
    }

    /// Unregister an endpoint for a user.
    pub fn unregister(&self, user: &UserId, endpoint: &Endpoint) {
        if let Some(mut endpoints) = self.connections.get_mut(user) {
            endpoints.remove(endpoint);
            if endpoints.is_empty() {
                drop(endpoints);
                self.connections.remove(user);
            }
        }
    }

    /// Get all active endpoints for a user.
    pub fn get_endpoints(&self, user: &UserId) -> Vec<Endpoint> {
        self.connections
            .get(user)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Check whether a user has any active endpoints.
    pub fn is_online(&self, user: &UserId) -> bool {
        self.connections
            .get(user)
            .map(|set| !set.is_empty())
            .unwrap_or(false)
    }
}

impl Default for EndpointRegistry {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// PlatformOutbound
// ---------------------------------------------------------------------------

/// What an [`EgressAdapter::send`] receives for delivery.
///
/// This is the adapter-facing message type — already formatted and ready
/// for the specific platform.
#[derive(Debug, Clone)]
pub enum PlatformOutbound {
    /// A complete reply message.
    Reply {
        /// Session key for routing (e.g. "telegram:chat-123").
        session_key:   String,
        /// Text content to deliver.
        content:       String,
        /// Binary attachments.
        attachments:   Vec<Attachment>,
        /// Optional reply context for threading.
        reply_context: Option<ReplyContext>,
    },
    /// An incremental streaming chunk.
    StreamChunk {
        /// Session key for routing.
        session_key: String,
        /// Incremental text delta.
        delta:       String,
        /// Platform message ID to edit (for progressive updates).
        edit_target: Option<String>,
    },
    /// A progress/status update.
    Progress {
        /// Session key for routing.
        session_key: String,
        /// Progress text.
        text:        String,
    },
}

// ---------------------------------------------------------------------------
// EgressAdapter
// ---------------------------------------------------------------------------

/// Simplified adapter interface for egress (send-only).
///
/// Each platform channel implements this trait. The egress loop calls
/// [`send`](Self::send) for each target endpoint.
#[async_trait]
pub trait EgressAdapter: Send + Sync + 'static {
    /// Which channel type this adapter handles.
    fn channel_type(&self) -> ChannelType;

    /// Deliver a message to a specific endpoint.
    async fn send(&self, endpoint: &Endpoint, msg: PlatformOutbound) -> Result<(), EgressError>;
}

// ---------------------------------------------------------------------------
// EgressError
// ---------------------------------------------------------------------------

/// Errors from egress delivery.
#[derive(Debug, Snafu)]
pub enum EgressError {
    /// Delivery to the target endpoint failed.
    #[snafu(display("delivery failed: {message}"))]
    DeliveryFailed { message: String },

    /// Delivery timed out.
    #[snafu(display("delivery timeout"))]
    Timeout,
}

// ---------------------------------------------------------------------------
// Egress
// ---------------------------------------------------------------------------

/// Outbound delivery engine.
///
/// Provides static delivery methods for routing [`OutboundEnvelope`]s to
/// the appropriate [`EgressAdapter`]s based on the user's connected
/// endpoints and routing rules.
///
/// Called directly by `Kernel::handle_deliver()` in the unified event loop.
pub struct Egress;

use std::sync::Arc;

/// Shared reference to an [`EgressAdapter`] implementation.
pub type EgressAdapterRef = Arc<dyn EgressAdapter>;

/// Shared reference to the [`EndpointRegistry`].
pub type EndpointRegistryRef = Arc<EndpointRegistry>;

impl Egress {
    /// Deliver a single outbound envelope to all resolved targets.
    ///
    /// This is a free function over the needed fields so that the
    /// `outbound_sub` is never borrowed immutably across an `.await`.
    ///
    /// Also used directly by `Kernel::handle_deliver()` in the unified
    /// event loop, bypassing the outbound bus subscribe loop.
    #[tracing::instrument(
        skip(adapters, endpoints, user_store, envelope),
        fields(
            user_id = %envelope.user.0,
            session_id = %envelope.session_id,
        )
    )]
    pub async fn deliver(
        adapters: &HashMap<ChannelType, Arc<dyn EgressAdapter>>,
        endpoints: &Arc<EndpointRegistry>,
        user_store: Option<&dyn UserStore>,
        envelope: OutboundEnvelope,
    ) {
        let targets = Self::resolve_targets(endpoints, user_store, &envelope).await;

        // Parallel delivery with per-endpoint timeout
        let futs = targets.into_iter().map(|endpoint| {
            let adapter = adapters.get(&endpoint.channel_type).cloned();
            let outbound = Self::format_for_endpoint(&endpoint, &envelope);
            async move {
                if let Some(adapter) = adapter {
                    match tokio::time::timeout(
                        Duration::from_secs(10),
                        adapter.send(&endpoint, outbound),
                    )
                    .await
                    {
                        Ok(Ok(())) => {
                            crate::metrics::MESSAGE_OUTBOUND
                                .with_label_values(&[&format!("{:?}", endpoint.channel_type)])
                                .inc();
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(?endpoint, %e, "delivery failed");
                        }
                        Err(_) => {
                            tracing::warn!(?endpoint, "delivery timeout");
                        }
                    }
                }
            }
        });
        futures::future::join_all(futs).await;
    }

    /// Resolve which endpoints should receive this envelope.
    ///
    /// For stateless channels (Telegram), falls back to querying
    /// `UserStore` for persisted platform identities when no ephemeral
    /// endpoint is found in the `EndpointRegistry`. Connection-oriented
    /// channels (Web, CLI) are never synthesised from the database.
    async fn resolve_targets(
        endpoints: &EndpointRegistry,
        user_store: Option<&dyn UserStore>,
        envelope: &OutboundEnvelope,
    ) -> Vec<Endpoint> {
        let mut connected = endpoints.get_endpoints(&envelope.user);

        // Check whether routing permits Telegram delivery.
        let telegram_wanted = match &envelope.routing {
            OutboundRouting::BroadcastAll => true,
            OutboundRouting::BroadcastExcept { exclude } => *exclude != ChannelType::Telegram,
            OutboundRouting::Targeted { channels } => channels.contains(&ChannelType::Telegram),
        };

        // Fallback: if no Telegram endpoint is registered but Telegram
        // delivery is desired, query persistent platform identities.
        let has_telegram = connected
            .iter()
            .any(|e| e.channel_type == ChannelType::Telegram);

        if telegram_wanted && !has_telegram {
            if let Some(store) = user_store {
                match Self::fallback_telegram_endpoints(store, &envelope.user).await {
                    Ok(eps) => connected.extend(eps),
                    Err(e) => {
                        tracing::warn!(
                            user_id = %envelope.user.0,
                            error = %e,
                            "egress fallback: failed to query platform identities"
                        );
                    }
                }
            }
        }

        // Apply routing filter on the (possibly augmented) endpoint list.
        match &envelope.routing {
            OutboundRouting::BroadcastAll => connected,
            OutboundRouting::BroadcastExcept { exclude } => connected
                .into_iter()
                .filter(|e| &e.channel_type != exclude)
                .collect(),
            OutboundRouting::Targeted { channels } => connected
                .into_iter()
                .filter(|e| channels.contains(&e.channel_type))
                .collect(),
        }
    }

    /// Query `UserStore` for Telegram platform identities and convert them
    /// into [`Endpoint`]s.
    async fn fallback_telegram_endpoints(
        store: &dyn UserStore,
        user_id: &UserId,
    ) -> std::result::Result<Vec<Endpoint>, Box<dyn std::error::Error + Send + Sync>> {
        let user = store.get_by_name(&user_id.0).await?;
        let user = match user {
            Some(u) => u,
            None => return Ok(vec![]),
        };

        let platforms = store.list_platforms(user.id).await?;
        let endpoints: Vec<Endpoint> = platforms
            .into_iter()
            .filter(|p| p.platform == "telegram")
            .filter_map(|p| {
                p.platform_user_id.parse::<i64>().ok().map(|chat_id| {
                    Endpoint {
                        channel_type: ChannelType::Telegram,
                        address:      EndpointAddress::Telegram {
                            chat_id,
                            thread_id: None,
                        },
                    }
                })
            })
            .collect();

        if !endpoints.is_empty() {
            tracing::debug!(
                user_id = %user_id.0,
                count = endpoints.len(),
                "egress fallback: resolved Telegram endpoints from platform identities"
            );
        }

        Ok(endpoints)
    }

    /// Convert an [`OutboundPayload`] into a [`PlatformOutbound`] for
    /// a specific endpoint.
    fn format_for_endpoint(endpoint: &Endpoint, envelope: &OutboundEnvelope) -> PlatformOutbound {
        let session_key = format!("{}:{}", endpoint.channel_type, envelope.session_id);

        match &envelope.payload {
            OutboundPayload::Reply {
                content,
                attachments,
            } => PlatformOutbound::Reply {
                session_key,
                content: content.as_text(),
                attachments: attachments.clone(),
                reply_context: None,
            },
            OutboundPayload::Progress { stage, detail } => PlatformOutbound::Progress {
                session_key,
                text: detail.as_deref().unwrap_or(stage).to_string(),
            },
            OutboundPayload::Error { code, message } => PlatformOutbound::Reply {
                session_key,
                content: format!("Error [{}]: {}", code, message),
                attachments: vec![],
                reply_context: None,
            },
            OutboundPayload::StateChange { .. } => {
                // State changes are not directly sent to platforms.
                // They could be used for Web UI updates via SSE.
                PlatformOutbound::Progress {
                    session_key,
                    text: String::new(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::{
        channel::types::MessageContent,
        io::types::{MessageId, OutboundRouting},
        process::SessionId,
    };

    // -----------------------------------------------------------------------
    // EndpointRegistry tests
    // -----------------------------------------------------------------------

    fn tg_endpoint(chat_id: i64) -> Endpoint {
        Endpoint {
            channel_type: ChannelType::Telegram,
            address:      EndpointAddress::Telegram {
                chat_id,
                thread_id: None,
            },
        }
    }

    fn web_endpoint(conn_id: &str) -> Endpoint {
        Endpoint {
            channel_type: ChannelType::Web,
            address:      EndpointAddress::Web {
                connection_id: conn_id.to_string(),
            },
        }
    }

    #[test]
    fn test_endpoint_registry_register_unregister() {
        let registry = EndpointRegistry::new();
        let user = UserId("user-1".to_string());

        let ep1 = tg_endpoint(123);
        let ep2 = web_endpoint("conn-1");

        // Register two endpoints
        registry.register(&user, ep1.clone());
        registry.register(&user, ep2.clone());

        let endpoints = registry.get_endpoints(&user);
        assert_eq!(endpoints.len(), 2);
        assert!(registry.is_online(&user));

        // Unregister one
        registry.unregister(&user, &ep1);
        let endpoints = registry.get_endpoints(&user);
        assert_eq!(endpoints.len(), 1);
        assert!(endpoints.contains(&ep2));

        // Unregister the last one — user entry should be cleaned up
        registry.unregister(&user, &ep2);
        let endpoints = registry.get_endpoints(&user);
        assert_eq!(endpoints.len(), 0);
        assert!(!registry.is_online(&user));
    }

    #[test]
    fn test_endpoint_registry_multiple_users() {
        let registry = EndpointRegistry::new();
        let user1 = UserId("user-1".to_string());
        let user2 = UserId("user-2".to_string());

        registry.register(&user1, tg_endpoint(100));
        registry.register(&user1, web_endpoint("conn-a"));
        registry.register(&user2, tg_endpoint(200));

        assert_eq!(registry.get_endpoints(&user1).len(), 2);
        assert_eq!(registry.get_endpoints(&user2).len(), 1);

        // Unknown user
        let user3 = UserId("user-3".to_string());
        assert_eq!(registry.get_endpoints(&user3).len(), 0);
        assert!(!registry.is_online(&user3));
    }

    // -----------------------------------------------------------------------
    // Resolve targets tests
    // -----------------------------------------------------------------------

    fn test_envelope(routing: OutboundRouting) -> OutboundEnvelope {
        OutboundEnvelope {
            id: MessageId::new(),
            in_reply_to: MessageId::new(),
            user: UserId("user-1".to_string()),
            session_id: SessionId::new("session-1"),
            routing,
            payload: OutboundPayload::Reply {
                content:     MessageContent::Text("hello".to_string()),
                attachments: vec![],
            },
            timestamp: jiff::Timestamp::now(),
        }
    }

    #[tokio::test]
    async fn test_resolve_targets_broadcast_all() {
        let registry = Arc::new(EndpointRegistry::new());
        let user = UserId("user-1".to_string());
        registry.register(&user, tg_endpoint(100));
        registry.register(&user, web_endpoint("conn-1"));

        let envelope = test_envelope(OutboundRouting::BroadcastAll);

        let targets = Egress::resolve_targets(&registry, None, &envelope).await;
        assert_eq!(targets.len(), 2);
    }

    #[tokio::test]
    async fn test_resolve_targets_broadcast_except() {
        let registry = Arc::new(EndpointRegistry::new());
        let user = UserId("user-1".to_string());
        registry.register(&user, tg_endpoint(100));
        registry.register(&user, web_endpoint("conn-1"));

        let envelope = test_envelope(OutboundRouting::BroadcastExcept {
            exclude: ChannelType::Telegram,
        });

        let targets = Egress::resolve_targets(&registry, None, &envelope).await;
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].channel_type, ChannelType::Web);
    }

    #[tokio::test]
    async fn test_resolve_targets_targeted() {
        let registry = Arc::new(EndpointRegistry::new());
        let user = UserId("user-1".to_string());
        registry.register(&user, tg_endpoint(100));
        registry.register(&user, web_endpoint("conn-1"));
        registry.register(
            &user,
            Endpoint {
                channel_type: ChannelType::Cli,
                address:      EndpointAddress::Cli {
                    session_id: "cli-1".to_string(),
                },
            },
        );

        let envelope = test_envelope(OutboundRouting::Targeted {
            channels: vec![ChannelType::Telegram, ChannelType::Cli],
        });

        let targets = Egress::resolve_targets(&registry, None, &envelope).await;
        assert_eq!(targets.len(), 2);
        let types: Vec<ChannelType> = targets.iter().map(|e| e.channel_type).collect();
        assert!(types.contains(&ChannelType::Telegram));
        assert!(types.contains(&ChannelType::Cli));
        assert!(!types.contains(&ChannelType::Web));
    }

    // -----------------------------------------------------------------------
    // EgressAdapter mock + delivery test
    // -----------------------------------------------------------------------

    struct MockEgressAdapter {
        channel: ChannelType,
        sent:    Mutex<Vec<(Endpoint, PlatformOutbound)>>,
    }

    impl MockEgressAdapter {
        fn new(channel: ChannelType) -> Self {
            Self {
                channel,
                sent: Mutex::new(Vec::new()),
            }
        }

        fn sent_count(&self) -> usize { self.sent.lock().unwrap().len() }
    }

    #[async_trait]
    impl EgressAdapter for MockEgressAdapter {
        fn channel_type(&self) -> ChannelType { self.channel }

        async fn send(
            &self,
            endpoint: &Endpoint,
            msg: PlatformOutbound,
        ) -> Result<(), EgressError> {
            self.sent.lock().unwrap().push((endpoint.clone(), msg));
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_egress_deliver_to_adapters() {
        let registry = Arc::new(EndpointRegistry::new());
        let user = UserId("user-1".to_string());
        registry.register(&user, tg_endpoint(100));

        let tg_adapter = Arc::new(MockEgressAdapter::new(ChannelType::Telegram));

        let mut adapters: HashMap<ChannelType, Arc<dyn EgressAdapter>> = HashMap::new();
        adapters.insert(ChannelType::Telegram, tg_adapter.clone());

        let envelope = test_envelope(OutboundRouting::BroadcastAll);
        Egress::deliver(&adapters, &registry, None, envelope).await;

        assert_eq!(tg_adapter.sent_count(), 1);
    }

    #[tokio::test]
    async fn test_egress_format_reply() {
        let registry = Arc::new(EndpointRegistry::new());
        let user = UserId("user-1".to_string());
        registry.register(&user, tg_endpoint(100));

        let tg_adapter = Arc::new(MockEgressAdapter::new(ChannelType::Telegram));
        let mut adapters: HashMap<ChannelType, Arc<dyn EgressAdapter>> = HashMap::new();
        adapters.insert(ChannelType::Telegram, tg_adapter.clone());

        let envelope = test_envelope(OutboundRouting::BroadcastAll);
        Egress::deliver(&adapters, &registry, None, envelope).await;

        let sent = tg_adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        match &sent[0].1 {
            PlatformOutbound::Reply {
                session_key,
                content,
                ..
            } => {
                assert!(session_key.starts_with("telegram:"));
                assert_eq!(content, "hello");
            }
            other => panic!("expected Reply, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_egress_format_error_payload() {
        let registry = Arc::new(EndpointRegistry::new());
        let user = UserId("user-1".to_string());
        registry.register(&user, tg_endpoint(100));

        let tg_adapter = Arc::new(MockEgressAdapter::new(ChannelType::Telegram));
        let mut adapters: HashMap<ChannelType, Arc<dyn EgressAdapter>> = HashMap::new();
        adapters.insert(ChannelType::Telegram, tg_adapter.clone());

        let envelope = OutboundEnvelope {
            id:          MessageId::new(),
            in_reply_to: MessageId::new(),
            user:        UserId("user-1".to_string()),
            session_id:  SessionId::new("session-1"),
            routing:     OutboundRouting::BroadcastAll,
            payload:     OutboundPayload::Error {
                code:    "E001".to_string(),
                message: "something broke".to_string(),
            },
            timestamp:   jiff::Timestamp::now(),
        };
        Egress::deliver(&adapters, &registry, None, envelope).await;

        let sent = tg_adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        match &sent[0].1 {
            PlatformOutbound::Reply { content, .. } => {
                assert!(content.contains("E001"));
                assert!(content.contains("something broke"));
            }
            other => panic!("expected Reply for error, got {:?}", other),
        }
    }
}
