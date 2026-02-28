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

//! Identity and session resolvers for the I/O Bus pipeline.
//!
//! These are simple implementations used during the transition period.
//! Once a real user store is wired into the I/O Bus model, the identity
//! resolver can look up registered users via the database.

use async_trait::async_trait;

use rara_kernel::channel::types::ChannelType;
use rara_kernel::io::ingress::{IdentityResolver, SessionResolver};
use rara_kernel::io::types::IngestError;
use rara_kernel::process::principal::UserId;
use rara_kernel::process::SessionId;

// ---------------------------------------------------------------------------
// AppIdentityResolver
// ---------------------------------------------------------------------------

/// Simple identity resolver that maps platform user IDs to a unified
/// [`UserId`] using the format `"{channel_type}:{platform_user_id}"`.
///
/// This is a pass-through implementation for the initial I/O Bus wiring.
/// A future version will look up the user store / auto-provision users.
pub struct AppIdentityResolver;

impl AppIdentityResolver {
    /// Create a new resolver.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl IdentityResolver for AppIdentityResolver {
    async fn resolve(
        &self,
        channel_type: ChannelType,
        platform_user_id: &str,
        _platform_chat_id: Option<&str>,
    ) -> Result<UserId, IngestError> {
        // Simple: use "channel_type:platform_user_id" as the UserId.
        // This can be improved later with a real user store lookup.
        let user_id = UserId(format!("{}:{}", channel_type, platform_user_id));
        Ok(user_id)
    }
}

// ---------------------------------------------------------------------------
// AppSessionResolver
// ---------------------------------------------------------------------------

/// Simple session resolver that maps each platform chat to its own session
/// using the format `"{channel_type}:{platform_chat_id}"`.
///
/// This mirrors the existing `tg:<chat_id>` convention used by the legacy
/// ChatService path.
pub struct AppSessionResolver;

impl AppSessionResolver {
    /// Create a new resolver.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SessionResolver for AppSessionResolver {
    async fn resolve(
        &self,
        _user: &UserId,
        channel_type: ChannelType,
        platform_chat_id: Option<&str>,
    ) -> Result<SessionId, IngestError> {
        // Simple: use "channel_type:chat_id" as the session key.
        let chat_id = platform_chat_id.unwrap_or("default");
        let session_id = SessionId::new(format!("{}:{}", channel_type, chat_id));
        Ok(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_identity_resolver_format() {
        let resolver = AppIdentityResolver::new();
        let user_id = resolver
            .resolve(ChannelType::Telegram, "12345", Some("chat-1"))
            .await
            .unwrap();
        assert_eq!(user_id.0, "telegram:12345");
    }

    #[tokio::test]
    async fn test_identity_resolver_no_chat_id() {
        let resolver = AppIdentityResolver::new();
        let user_id = resolver
            .resolve(ChannelType::Web, "user-abc", None)
            .await
            .unwrap();
        assert_eq!(user_id.0, "web:user-abc");
    }

    #[tokio::test]
    async fn test_session_resolver_with_chat_id() {
        let resolver = AppSessionResolver::new();
        let user = UserId("telegram:12345".to_string());
        let session_id = resolver
            .resolve(&user, ChannelType::Telegram, Some("chat-1"))
            .await
            .unwrap();
        assert_eq!(session_id.to_string(), "telegram:chat-1");
    }

    #[tokio::test]
    async fn test_session_resolver_default_chat_id() {
        let resolver = AppSessionResolver::new();
        let user = UserId("web:user-abc".to_string());
        let session_id = resolver
            .resolve(&user, ChannelType::Web, None)
            .await
            .unwrap();
        assert_eq!(session_id.to_string(), "web:default");
    }
}
