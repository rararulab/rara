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

//! Ingress Pipeline — converts raw platform messages into unified
//! [`InboundMessage`](crate::io::types::InboundMessage) and publishes
//! to the [`InboundBus`](crate::io::bus::InboundBus).
//!
//! The pipeline orchestrates three steps:
//! 1. **Identity resolution** — map `(channel_type, platform_user_id)` to a
//!    unified [`UserId`](crate::process::principal::UserId).
//! 2. **Session resolution** — resolve or create a
//!    [`SessionId`](crate::process::SessionId) for this user + channel context.
//! 3. **Bus publish** — build an [`InboundMessage`] and publish it to the bus.
//!
//! Channel adapters only need to call [`IngressPipeline::ingest`] — all
//! coordination lives here.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    channel::types::{ChannelType, MessageContent},
    io::{
        bus::InboundBus,
        types::{BusError, ChannelSource, InboundMessage, IngestError, MessageId, ReplyContext},
    },
    process::{SessionId, principal::UserId},
};

// ---------------------------------------------------------------------------
// RawPlatformMessage
// ---------------------------------------------------------------------------

/// Raw message from a channel adapter before identity/session resolution.
///
/// Adapters construct this from platform-specific events and hand it to
/// [`IngressPipeline::ingest`]. The ingress pipeline then resolves identity
/// and session before publishing to the bus.
#[derive(Debug)]
pub struct RawPlatformMessage {
    /// Which channel this message arrived from.
    pub channel_type:        ChannelType,
    /// Platform-specific message ID (for dedup / reply mapping).
    pub platform_message_id: Option<String>,
    /// Platform-specific user identifier.
    pub platform_user_id:    String,
    /// Platform-specific chat/thread identifier.
    pub platform_chat_id:    Option<String>,
    /// Message content (text or multimodal).
    pub content:             MessageContent,
    /// Optional reply/thread context for egress routing.
    pub reply_context:       Option<ReplyContext>,
    /// Arbitrary adapter-specific metadata.
    pub metadata:            HashMap<String, Value>,
}

// ---------------------------------------------------------------------------
// IdentityResolver
// ---------------------------------------------------------------------------

/// Resolves a platform identity to a unified [`UserId`].
///
/// Implementations may look up a database mapping, create auto-provisioned
/// users, or apply group-chat policies.
#[async_trait]
pub trait IdentityResolver: Send + Sync + 'static {
    /// Map platform coordinates to a kernel-level user identity.
    async fn resolve(
        &self,
        channel_type: ChannelType,
        platform_user_id: &str,
        platform_chat_id: Option<&str>,
    ) -> Result<UserId, IngestError>;
}

// ---------------------------------------------------------------------------
// SessionResolver
// ---------------------------------------------------------------------------

/// Resolves or creates a session for a given user + channel context.
///
/// Implementations may support cross-channel session sharing (e.g. the same
/// user on Telegram and Web shares a session) or per-chat isolation.
#[async_trait]
pub trait SessionResolver: Send + Sync + 'static {
    /// Resolve (or create) a session for the given user and channel context.
    async fn resolve(
        &self,
        user: &UserId,
        channel_type: ChannelType,
        platform_chat_id: Option<&str>,
    ) -> Result<SessionId, IngestError>;
}

// ---------------------------------------------------------------------------
// IngressPipeline
// ---------------------------------------------------------------------------

/// Orchestrates identity resolution, session resolution, and bus publishing.
///
/// Channel adapters call [`ingest`](Self::ingest) with a
/// [`RawPlatformMessage`]; the pipeline handles identity resolution,
/// session resolution, and bus publishing. It composes an
/// [`IdentityResolver`], a [`SessionResolver`], and an [`InboundBus`]
/// publisher.
pub struct IngressPipeline {
    identity_resolver: Arc<dyn IdentityResolver>,
    session_resolver:  Arc<dyn SessionResolver>,
    publisher:         Arc<dyn InboundBus>,
}

impl IngressPipeline {
    /// Create a new ingress pipeline.
    pub fn new(
        identity_resolver: Arc<dyn IdentityResolver>,
        session_resolver: Arc<dyn SessionResolver>,
        publisher: Arc<dyn InboundBus>,
    ) -> Self {
        Self {
            identity_resolver,
            session_resolver,
            publisher,
        }
    }

    /// Ingest a raw platform message into the kernel pipeline.
    pub async fn ingest(&self, raw: RawPlatformMessage) -> Result<(), IngestError> {
        // 1. Resolve identity
        let user_id = self
            .identity_resolver
            .resolve(
                raw.channel_type,
                &raw.platform_user_id,
                raw.platform_chat_id.as_deref(),
            )
            .await?;

        // 2. Resolve session
        let session_id = self
            .session_resolver
            .resolve(&user_id, raw.channel_type, raw.platform_chat_id.as_deref())
            .await?;

        // 3. Build InboundMessage
        let msg = InboundMessage {
            id: MessageId::new(),
            source: ChannelSource {
                channel_type:        raw.channel_type,
                platform_message_id: raw.platform_message_id,
                platform_user_id:    raw.platform_user_id,
                platform_chat_id:    raw.platform_chat_id,
            },
            user: user_id,
            session_id,
            content: raw.content,
            reply_context: raw.reply_context,
            timestamp: jiff::Timestamp::now(),
            metadata: raw.metadata,
        };

        // 4. Publish to bus (translate BusError → IngestError)
        self.publisher.publish(msg).await.map_err(|e| match e {
            BusError::Full => IngestError::SystemBusy,
            other => IngestError::Internal {
                message: other.to_string(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::io::memory_bus::InMemoryInboundBus;

    // -----------------------------------------------------------------------
    // Mock IdentityResolver
    // -----------------------------------------------------------------------

    struct MockIdentityResolver {
        result: Mutex<Result<UserId, IngestError>>,
        calls:  Mutex<Vec<(ChannelType, String, Option<String>)>>,
    }

    impl MockIdentityResolver {
        fn succeeding(user_id: &str) -> Self {
            Self {
                result: Mutex::new(Ok(UserId(user_id.to_string()))),
                calls:  Mutex::new(Vec::new()),
            }
        }

        fn failing(err: IngestError) -> Self {
            Self {
                result: Mutex::new(Err(err)),
                calls:  Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl IdentityResolver for MockIdentityResolver {
        async fn resolve(
            &self,
            channel_type: ChannelType,
            platform_user_id: &str,
            platform_chat_id: Option<&str>,
        ) -> Result<UserId, IngestError> {
            self.calls.lock().unwrap().push((
                channel_type,
                platform_user_id.to_string(),
                platform_chat_id.map(|s| s.to_string()),
            ));
            let guard = self.result.lock().unwrap();
            match &*guard {
                Ok(uid) => Ok(uid.clone()),
                Err(IngestError::IdentityResolutionFailed { message }) => {
                    Err(IngestError::IdentityResolutionFailed {
                        message: message.clone(),
                    })
                }
                Err(IngestError::SystemBusy) => Err(IngestError::SystemBusy),
                Err(IngestError::Internal { message }) => Err(IngestError::Internal {
                    message: message.clone(),
                }),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Mock SessionResolver
    // -----------------------------------------------------------------------

    struct MockSessionResolver {
        session_id: String,
    }

    impl MockSessionResolver {
        fn new(session_id: &str) -> Self {
            Self {
                session_id: session_id.to_string(),
            }
        }
    }

    #[async_trait]
    impl SessionResolver for MockSessionResolver {
        async fn resolve(
            &self,
            _user: &UserId,
            _channel_type: ChannelType,
            _platform_chat_id: Option<&str>,
        ) -> Result<SessionId, IngestError> {
            Ok(SessionId::new(&self.session_id))
        }
    }

    // -----------------------------------------------------------------------
    // Helper
    // -----------------------------------------------------------------------

    fn raw_message(text: &str) -> RawPlatformMessage {
        RawPlatformMessage {
            channel_type:        ChannelType::Telegram,
            platform_message_id: Some("msg-42".to_string()),
            platform_user_id:    "tg-user-1".to_string(),
            platform_chat_id:    Some("tg-chat-1".to_string()),
            content:             MessageContent::Text(text.to_string()),
            reply_context:       None,
            metadata:            HashMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_pipeline_ingest_success() {
        let identity = Arc::new(MockIdentityResolver::succeeding("user-1"));
        let session = Arc::new(MockSessionResolver::new("session-1"));
        let bus = Arc::new(InMemoryInboundBus::new(100));

        let pipeline = IngressPipeline::new(
            identity.clone() as Arc<dyn IdentityResolver>,
            session as Arc<dyn SessionResolver>,
            bus.clone() as Arc<dyn InboundBus>,
        );

        pipeline.ingest(raw_message("hello")).await.unwrap();

        // Verify the message landed in the bus
        let messages = bus.drain(10).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_text(), "hello");
        assert_eq!(messages[0].user, UserId("user-1".to_string()));
        assert_eq!(messages[0].session_id, SessionId::new("session-1"));
        assert_eq!(messages[0].source.channel_type, ChannelType::Telegram);
        assert_eq!(messages[0].source.platform_user_id, "tg-user-1".to_string());

        // Verify identity resolver was called correctly
        let calls = identity.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, ChannelType::Telegram);
        assert_eq!(calls[0].1, "tg-user-1");
        assert_eq!(calls[0].2, Some("tg-chat-1".to_string()));
    }

    #[tokio::test]
    async fn test_pipeline_identity_failure() {
        let identity = Arc::new(MockIdentityResolver::failing(
            IngestError::IdentityResolutionFailed {
                message: "unknown user".to_string(),
            },
        ));
        let session = Arc::new(MockSessionResolver::new("session-1"));
        let bus = Arc::new(InMemoryInboundBus::new(100));

        let pipeline = IngressPipeline::new(
            identity as Arc<dyn IdentityResolver>,
            session as Arc<dyn SessionResolver>,
            bus.clone() as Arc<dyn InboundBus>,
        );

        let result = pipeline.ingest(raw_message("hello")).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            IngestError::IdentityResolutionFailed { .. }
        ));

        // Bus should be empty — nothing was published
        let messages = bus.drain(10).await;
        assert_eq!(messages.len(), 0);
    }

    #[tokio::test]
    async fn test_pipeline_bus_full() {
        let identity = Arc::new(MockIdentityResolver::succeeding("user-1"));
        let session = Arc::new(MockSessionResolver::new("session-1"));
        // Capacity of 1 — second message should fail
        let bus = Arc::new(InMemoryInboundBus::new(1));

        let pipeline = IngressPipeline::new(
            identity as Arc<dyn IdentityResolver>,
            session as Arc<dyn SessionResolver>,
            bus as Arc<dyn InboundBus>,
        );

        // First message should succeed
        pipeline.ingest(raw_message("first")).await.unwrap();

        // Second message should fail with SystemBusy
        let result = pipeline.ingest(raw_message("second")).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), IngestError::SystemBusy));
    }
}
