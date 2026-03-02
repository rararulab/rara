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
    session::SessionRepository,
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
/// 2. If a linked user is found, return `UserId(user.name)` — the real kernel
///    username (e.g. `"root"`).
/// 3. If **not** found:
///    - **Web** channel: fall through to the old synthetic format
///      `"web:<platform_user_id>"` for backward compatibility (the Web adapter
///      already sends the real username from JWT).
///    - All other channels (Telegram, CLI, …): return
///      `IdentityResolutionFailed` — the user must link their platform account
///      first.
pub struct DefaultIdentityResolver {
    user_store: Arc<dyn UserStore>,
}

impl DefaultIdentityResolver {
    /// Create a new resolver backed by the given user store.
    pub fn new(user_store: Arc<dyn UserStore>) -> Self { Self { user_store } }
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

/// Session resolver that first consults [`ChannelBinding`]s in the database
/// and falls back to the raw `platform_chat_id` when no binding exists.
///
/// This allows the Telegram `/new` and `/sessions` commands to redirect
/// subsequent messages to the correct session after re-binding.
pub struct DefaultSessionResolver {
    session_repo: Arc<dyn SessionRepository>,
}

impl DefaultSessionResolver {
    /// Create a new resolver backed by the given session repository.
    pub fn new(session_repo: Arc<dyn SessionRepository>) -> Self { Self { session_repo } }
}

#[async_trait]
impl SessionResolver for DefaultSessionResolver {
    async fn resolve(
        &self,
        _user: &UserId,
        channel_type: ChannelType,
        platform_chat_id: Option<&str>,
    ) -> Result<SessionId, IngestError> {
        let chat_id = platform_chat_id.unwrap_or("default");

        // Try channel binding lookup first (honours /new and /sessions switch).
        if let Some(chat_id_str) = platform_chat_id {
            let ch_label = channel_type.to_string();
            match self
                .session_repo
                .get_binding_by_chat(&ch_label, chat_id_str)
                .await
            {
                Ok(Some(binding)) => {
                    debug!(
                        channel = %channel_type,
                        chat_id = chat_id_str,
                        bound_session = %binding.session_key,
                        "session resolved via channel binding"
                    );
                    return Ok(SessionId::new(binding.session_key.as_str()));
                }
                Ok(None) => {
                    // No binding — fall through to raw chat_id.
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        channel = %channel_type,
                        chat_id = chat_id_str,
                        "channel binding lookup failed, falling back to raw chat_id"
                    );
                }
            }
        }

        Ok(SessionId::new(chat_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use rara_kernel::{
        channel::types::ChatMessage,
        error::Result as KResult,
        process::user::{KernelUser, PlatformIdentity},
        session::{ChannelBinding, SessionEntry, SessionError, SessionKey},
    };

    use super::*;

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
        async fn get_by_id(&self, _id: uuid::Uuid) -> KResult<Option<KernelUser>> { Ok(None) }

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

    // -- Fake SessionRepository for unit tests ------------------------------

    struct FakeSessionRepo {
        bindings: Vec<ChannelBinding>,
    }

    impl FakeSessionRepo {
        fn empty() -> Self { Self { bindings: vec![] } }

        fn with_binding(channel_type: &str, chat_id: &str, session_key: &str) -> Self {
            let now = chrono::Utc::now();
            Self {
                bindings: vec![ChannelBinding {
                    channel_type: channel_type.to_string(),
                    account:      "bot".to_string(),
                    chat_id:      chat_id.to_string(),
                    session_key:  SessionKey::new(session_key),
                    created_at:   now,
                    updated_at:   now,
                }],
            }
        }
    }

    #[async_trait]
    impl SessionRepository for FakeSessionRepo {
        async fn create_session(&self, _: &SessionEntry) -> Result<SessionEntry, SessionError> {
            unimplemented!()
        }

        async fn get_session(&self, _: &SessionKey) -> Result<Option<SessionEntry>, SessionError> {
            Ok(None)
        }

        async fn list_sessions(&self, _: i64, _: i64) -> Result<Vec<SessionEntry>, SessionError> {
            Ok(vec![])
        }

        async fn update_session(&self, _: &SessionEntry) -> Result<SessionEntry, SessionError> {
            unimplemented!()
        }

        async fn delete_session(&self, _: &SessionKey) -> Result<(), SessionError> { Ok(()) }

        async fn append_message(
            &self,
            _: &SessionKey,
            _: &ChatMessage,
        ) -> Result<ChatMessage, SessionError> {
            unimplemented!()
        }

        async fn read_messages(
            &self,
            _: &SessionKey,
            _: Option<i64>,
            _: Option<i64>,
        ) -> Result<Vec<ChatMessage>, SessionError> {
            Ok(vec![])
        }

        async fn clear_messages(&self, _: &SessionKey) -> Result<(), SessionError> { Ok(()) }

        async fn fork_session(
            &self,
            _: &SessionKey,
            _: &SessionKey,
            _: i64,
        ) -> Result<SessionEntry, SessionError> {
            unimplemented!()
        }

        async fn bind_channel(&self, _: &ChannelBinding) -> Result<ChannelBinding, SessionError> {
            unimplemented!()
        }

        async fn get_channel_binding(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Option<ChannelBinding>, SessionError> {
            Ok(None)
        }

        async fn get_binding_by_chat(
            &self,
            channel_type: &str,
            chat_id: &str,
        ) -> Result<Option<ChannelBinding>, SessionError> {
            Ok(self
                .bindings
                .iter()
                .find(|b| b.channel_type == channel_type && b.chat_id == chat_id)
                .cloned())
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
    async fn test_session_resolver_no_binding_falls_back() {
        let repo = Arc::new(FakeSessionRepo::empty());
        let resolver = DefaultSessionResolver::new(repo);
        let user = UserId("telegram:12345".to_string());
        let session_id = resolver
            .resolve(&user, ChannelType::Telegram, Some("chat-1"))
            .await
            .unwrap();
        // No binding → raw chat_id used
        assert_eq!(session_id.to_string(), "chat-1");
    }

    #[tokio::test]
    async fn test_session_resolver_with_binding() {
        let repo = Arc::new(FakeSessionRepo::with_binding(
            "telegram",
            "12345",
            "tg-12345-1700000000",
        ));
        let resolver = DefaultSessionResolver::new(repo);
        let user = UserId("root".to_string());
        let session_id = resolver
            .resolve(&user, ChannelType::Telegram, Some("12345"))
            .await
            .unwrap();
        // Binding found → bound session key used
        assert_eq!(session_id.to_string(), "tg-12345-1700000000");
    }

    #[tokio::test]
    async fn test_session_resolver_default_chat_id() {
        let repo = Arc::new(FakeSessionRepo::empty());
        let resolver = DefaultSessionResolver::new(repo);
        let user = UserId("web:user-abc".to_string());
        let session_id = resolver
            .resolve(&user, ChannelType::Web, None)
            .await
            .unwrap();
        assert_eq!(session_id.to_string(), "default");
    }
}
