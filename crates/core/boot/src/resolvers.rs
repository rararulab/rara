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

//! Default identity and session resolvers for the I/O Bus pipeline.
//!
//! The [`DefaultIdentityResolver`] looks up platform identities in the
//! [`UserStore`] so that linked Telegram (and other) accounts resolve to
//! their real kernel user instead of a synthetic `"telegram:<chat_id>"` ID
//! that would fail `validate_principal`.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    channel::types::ChannelType,
    io::{
        ingress::{IdentityResolver, SessionResolver},
        types::IngestError,
    },
    process::{SessionId, principal::UserId, user::UserStore},
};
use tracing::debug;

// ---------------------------------------------------------------------------
// DefaultIdentityResolver
// ---------------------------------------------------------------------------

/// Identity resolver backed by a [`UserStore`].
///
/// Resolution strategy per channel:
///
/// 1. Look up `user_store.get_by_platform(channel, platform_user_id)`.
/// 2. If a linked user is found, return `UserId(user.name)` — the real
///    kernel username (e.g. `"root"`).
/// 3. If **not** found:
///    - **Web** channel: fall through to the old synthetic format
///      `"web:<platform_user_id>"` for backward compatibility (the Web
///      adapter already sends the real username from JWT).
///    - All other channels (Telegram, CLI, …): return
///      `IdentityResolutionFailed` — the user must link their platform
///      account first.
pub struct DefaultIdentityResolver {
    user_store: Arc<dyn UserStore>,
}

impl DefaultIdentityResolver {
    /// Create a new resolver backed by the given user store.
    pub fn new(user_store: Arc<dyn UserStore>) -> Self {
        Self { user_store }
    }
}

#[async_trait]
impl IdentityResolver for DefaultIdentityResolver {
    async fn resolve(
        &self,
        channel_type: ChannelType,
        platform_user_id: &str,
        _platform_chat_id: Option<&str>,
    ) -> Result<UserId, IngestError> {
        // 1. Try platform identity lookup.
        let platform_label = channel_type.to_string();
        match self
            .user_store
            .get_by_platform(&platform_label, platform_user_id)
            .await
        {
            Ok(Some(user)) => {
                debug!(
                    channel = %channel_type,
                    platform_user_id,
                    resolved_user = %user.name,
                    "identity resolved via platform link"
                );
                return Ok(UserId(user.name));
            }
            Ok(None) => {
                // Not linked — fall through to channel-specific handling.
            }
            Err(e) => {
                // DB error — log and treat as "not found" for Web, error for
                // others.
                tracing::warn!(
                    error = %e,
                    channel = %channel_type,
                    platform_user_id,
                    "platform identity lookup failed"
                );
            }
        }

        // 2. Channel-specific fallback.
        match channel_type {
            // Web channel: the platform_user_id comes from JWT and is
            // already the real kernel username (e.g. "root").
            ChannelType::Web => Ok(UserId(platform_user_id.to_string())),
            // Non-web channels require a linked platform identity.
            _ => Err(IngestError::IdentityResolutionFailed {
                message: format!(
                    "no linked user for platform {}:{}  — link your account first",
                    channel_type, platform_user_id
                ),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// DefaultSessionResolver
// ---------------------------------------------------------------------------

/// Simple session resolver that maps each platform chat to its own session
/// using the format `"{channel_type}:{platform_chat_id}"`.
///
/// This mirrors the `tg:<chat_id>` convention used by the Telegram
/// adapter.
pub struct DefaultSessionResolver;

impl DefaultSessionResolver {
    /// Create a new resolver.
    pub fn new() -> Self { Self }
}

impl Default for DefaultSessionResolver {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl SessionResolver for DefaultSessionResolver {
    async fn resolve(
        &self,
        _user: &UserId,
        _channel_type: ChannelType,
        platform_chat_id: Option<&str>,
    ) -> Result<SessionId, IngestError> {
        // Use the raw platform chat ID as the session key so that it
        // matches the key the frontend uses for REST API lookups.
        let chat_id = platform_chat_id.unwrap_or("default");
        let session_id = SessionId::new(chat_id.to_string());
        Ok(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rara_kernel::{
        error::Result as KResult,
        process::user::{KernelUser, PlatformIdentity},
    };

    // -- In-memory UserStore for unit tests ---------------------------------

    struct FakeUserStore {
        users:     Vec<KernelUser>,
        platforms: Vec<(String, String, String)>, // (platform, platform_user_id, user_name)
    }

    impl FakeUserStore {
        fn empty() -> Self {
            Self {
                users:     vec![],
                platforms: vec![],
            }
        }

        fn with_linked(platform: &str, platform_uid: &str, user_name: &str) -> Self {
            let user = KernelUser {
                name: user_name.to_string(),
                ..KernelUser::root()
            };
            Self {
                users:     vec![user],
                platforms: vec![(
                    platform.to_string(),
                    platform_uid.to_string(),
                    user_name.to_string(),
                )],
            }
        }
    }

    #[async_trait]
    impl UserStore for FakeUserStore {
        async fn get_by_id(&self, _id: uuid::Uuid) -> KResult<Option<KernelUser>> {
            Ok(None)
        }

        async fn get_by_name(&self, name: &str) -> KResult<Option<KernelUser>> {
            Ok(self.users.iter().find(|u| u.name == name).cloned())
        }

        async fn get_by_platform(
            &self,
            platform: &str,
            platform_user_id: &str,
        ) -> KResult<Option<KernelUser>> {
            for (p, puid, uname) in &self.platforms {
                if p == platform && puid == platform_user_id {
                    return Ok(self.users.iter().find(|u| u.name == *uname).cloned());
                }
            }
            Ok(None)
        }

        async fn create(&self, _user: &KernelUser) -> KResult<()> { Ok(()) }
        async fn update(&self, _user: &KernelUser) -> KResult<()> { Ok(()) }
        async fn delete(&self, _id: uuid::Uuid) -> KResult<()> { Ok(()) }
        async fn list(&self) -> KResult<Vec<KernelUser>> { Ok(vec![]) }
        async fn link_platform(&self, _identity: &PlatformIdentity) -> KResult<()> { Ok(()) }
        async fn unlink_platform(&self, _id: uuid::Uuid) -> KResult<()> { Ok(()) }
        async fn list_platforms(&self, _user_id: uuid::Uuid) -> KResult<Vec<PlatformIdentity>> {
            Ok(vec![])
        }
    }

    // -- Tests --------------------------------------------------------------

    #[tokio::test]
    async fn test_telegram_linked_resolves_to_real_user() {
        let store = Arc::new(FakeUserStore::with_linked("telegram", "12345", "root"));
        let resolver = DefaultIdentityResolver::new(store);
        let uid = resolver
            .resolve(ChannelType::Telegram, "12345", Some("chat-1"))
            .await
            .unwrap();
        assert_eq!(uid.0, "root");
    }

    #[tokio::test]
    async fn test_telegram_unlinked_returns_error() {
        let store = Arc::new(FakeUserStore::empty());
        let resolver = DefaultIdentityResolver::new(store);
        let result = resolver
            .resolve(ChannelType::Telegram, "99999", Some("chat-1"))
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            IngestError::IdentityResolutionFailed { .. }
        ));
    }

    #[tokio::test]
    async fn test_web_unlinked_falls_through() {
        let store = Arc::new(FakeUserStore::empty());
        let resolver = DefaultIdentityResolver::new(store);
        let uid = resolver
            .resolve(ChannelType::Web, "root", None)
            .await
            .unwrap();
        // Web channel uses the JWT username directly (no prefix).
        assert_eq!(uid.0, "root");
    }

    #[tokio::test]
    async fn test_web_linked_resolves_to_real_user() {
        let store = Arc::new(FakeUserStore::with_linked("web", "user-abc", "alice"));
        let resolver = DefaultIdentityResolver::new(store);
        let uid = resolver
            .resolve(ChannelType::Web, "user-abc", None)
            .await
            .unwrap();
        assert_eq!(uid.0, "alice");
    }

    #[tokio::test]
    async fn test_session_resolver_with_chat_id() {
        let resolver = DefaultSessionResolver::new();
        let user = UserId("telegram:12345".to_string());
        let session_id = resolver
            .resolve(&user, ChannelType::Telegram, Some("chat-1"))
            .await
            .unwrap();
        assert_eq!(session_id.to_string(), "chat-1");
    }

    #[tokio::test]
    async fn test_session_resolver_default_chat_id() {
        let resolver = DefaultSessionResolver::new();
        let user = UserId("web:user-abc".to_string());
        let session_id = resolver
            .resolve(&user, ChannelType::Web, None)
            .await
            .unwrap();
        assert_eq!(session_id.to_string(), "default");
    }
}
