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

//! Ingress Pipeline — resolves identity and session for raw platform messages.
//!
//! The pipeline orchestrates two resolution steps:
//! 1. **Identity resolution** — map `(channel_type, platform_user_id)` to a
//!    unified [`UserId`](crate::process::principal::UserId).
//! 2. **Session resolution** — resolve or create a
//!    [`SessionId`](crate::process::SessionId) for this user + channel context.
//!
//! Channel adapters call
//! [`KernelHandle::ingest`](crate::handle::KernelHandle::ingest)
//! which delegates to [`IngressPipeline::resolve`] for resolution, then
//! pushes the resulting [`InboundMessage`] through the event queue.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde_json::Value;

/// Shared reference to an [`IdentityResolver`] implementation.
pub type IdentityResolverRef = Arc<dyn IdentityResolver>;

/// Shared reference to a [`SessionResolver`] implementation.
pub type SessionResolverRef = Arc<dyn SessionResolver>;

/// Shared reference to the [`IngressPipeline`].
pub type IngressPipelineRef = Arc<IngressPipeline>;

use crate::{
    channel::types::{ChannelType, MessageContent},
    io::types::{ChannelSource, InboundMessage, IngestError, MessageId, ReplyContext},
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

/// Resolves identity and session for raw platform messages.
///
/// This is a pure resolution layer — it does not push events or interact
/// with the event queue.
/// [`KernelHandle::ingest`](crate::handle::KernelHandle::ingest)
/// calls [`resolve`](Self::resolve) and then pushes the resulting
/// [`InboundMessage`] through the event queue.
pub struct IngressPipeline {
    identity_resolver: Arc<dyn IdentityResolver>,
    session_resolver:  Arc<dyn SessionResolver>,
}

impl IngressPipeline {
    /// Create a new ingress pipeline.
    pub fn new(
        identity_resolver: Arc<dyn IdentityResolver>,
        session_resolver: Arc<dyn SessionResolver>,
    ) -> Self {
        Self {
            identity_resolver,
            session_resolver,
        }
    }

    /// Resolve identity and session for a raw platform message.
    ///
    /// Returns a fully-formed [`InboundMessage`] ready for the event queue.
    pub async fn resolve(&self, raw: RawPlatformMessage) -> Result<InboundMessage, IngestError> {
        let span = tracing::info_span!(
            "ingress",
            channel = ?raw.channel_type,
            platform_user = %raw.platform_user_id,
            session_id = tracing::field::Empty,
            user_id = tracing::field::Empty,
        );
        let _guard = span.enter();

        // 1. Resolve identity
        let user_id = self
            .identity_resolver
            .resolve(
                raw.channel_type,
                &raw.platform_user_id,
                raw.platform_chat_id.as_deref(),
            )
            .await?;
        span.record("user_id", tracing::field::display(&user_id.0));

        // 2. Resolve session
        let session_id = self
            .session_resolver
            .resolve(&user_id, raw.channel_type, raw.platform_chat_id.as_deref())
            .await?;
        span.record("session_id", tracing::field::display(&session_id));

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
            target_agent_id: None,
            target_agent: None,
            content: raw.content,
            reply_context: raw.reply_context,
            timestamp: jiff::Timestamp::now(),
            metadata: raw.metadata,
        };

        tracing::info!(
            channel = ?msg.source.channel_type,
            user_id = %msg.user.0,
            session_id = %msg.session_id,
            content = %msg.content.as_text(),
            "resolved inbound message",
        );

        Ok(msg)
    }
}

// ---------------------------------------------------------------------------
// Test-only Noop resolvers
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "testing"))]
mod noop {
    use async_trait::async_trait;

    use super::{IdentityResolver, SessionResolver};
    use crate::{
        channel::types::ChannelType,
        io::types::IngestError,
        process::{SessionId, principal::UserId},
    };

    /// A no-op identity resolver for testing — maps to
    /// `"{channel_type}:{platform_user_id}"`.
    pub struct NoopIdentityResolver;

    #[async_trait]
    impl IdentityResolver for NoopIdentityResolver {
        async fn resolve(
            &self,
            channel_type: ChannelType,
            platform_user_id: &str,
            _platform_chat_id: Option<&str>,
        ) -> Result<UserId, IngestError> {
            Ok(UserId(format!("{}:{}", channel_type, platform_user_id)))
        }
    }

    /// A no-op session resolver for testing — maps to
    /// `"{channel_type}:{platform_chat_id}"`.
    pub struct NoopSessionResolver;

    #[async_trait]
    impl SessionResolver for NoopSessionResolver {
        async fn resolve(
            &self,
            _user: &UserId,
            channel_type: ChannelType,
            platform_chat_id: Option<&str>,
        ) -> Result<SessionId, IngestError> {
            let _ = (channel_type, platform_chat_id);
            Ok(SessionId::new())
        }
    }
}

#[cfg(any(test, feature = "testing"))]
pub use noop::{NoopIdentityResolver, NoopSessionResolver};

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

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
        session_id: SessionId,
    }

    impl MockSessionResolver {
        fn new(session_id: SessionId) -> Self { Self { session_id } }
    }

    #[async_trait]
    impl SessionResolver for MockSessionResolver {
        async fn resolve(
            &self,
            _user: &UserId,
            _channel_type: ChannelType,
            _platform_chat_id: Option<&str>,
        ) -> Result<SessionId, IngestError> {
            Ok(self.session_id.clone())
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
    async fn test_pipeline_resolve_success() {
        let identity = Arc::new(MockIdentityResolver::succeeding("user-1"));
        let expected_session = SessionId::new();
        let session = Arc::new(MockSessionResolver::new(expected_session));

        let pipeline = IngressPipeline::new(
            identity.clone() as Arc<dyn IdentityResolver>,
            session as Arc<dyn SessionResolver>,
        );

        let msg = pipeline.resolve(raw_message("hello")).await.unwrap();

        assert_eq!(msg.content.as_text(), "hello");
        assert_eq!(msg.user, UserId("user-1".to_string()));
        assert_eq!(msg.session_id, expected_session);
        assert_eq!(msg.source.channel_type, ChannelType::Telegram);
        assert_eq!(msg.source.platform_user_id, "tg-user-1");

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
        let session = Arc::new(MockSessionResolver::new(SessionId::new()));

        let pipeline = IngressPipeline::new(
            identity as Arc<dyn IdentityResolver>,
            session as Arc<dyn SessionResolver>,
        );

        let result = pipeline.resolve(raw_message("hello")).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            IngestError::IdentityResolutionFailed { .. }
        ));
    }
}
