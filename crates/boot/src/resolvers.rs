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

//! Identity resolvers for the I/O Bus pipeline.
//!
//! - [`PlatformIdentityResolver`] — config-driven identity resolver (platform
//!   identity → kernel user via in-memory mapping).

use std::collections::HashMap;

use async_trait::async_trait;
use rara_kernel::{
    channel::types::ChannelType,
    identity::UserId,
    io::{IOError, IdentityResolver},
};
use tracing::debug;

use crate::user_store::UserConfig;

// ---------------------------------------------------------------------------
// PlatformIdentityResolver
// ---------------------------------------------------------------------------

/// Config-driven identity resolver that maps platform identities to kernel
/// users via an in-memory lookup table built from YAML configuration.
///
/// All channels must have their platform mappings explicitly configured.
/// Unknown platform users are rejected with
/// [`IOError::IdentityResolutionFailed`].
pub struct PlatformIdentityResolver {
    /// `(channel_type, platform_uid)` → kernel user name.
    mappings: HashMap<(String, String), String>,
}

impl PlatformIdentityResolver {
    /// Build the resolver from the configured user list.
    pub fn new(users: &[UserConfig]) -> Self {
        let mut mappings = HashMap::new();
        for user in users {
            for platform in &user.platforms {
                mappings.insert(
                    (platform.channel_type.clone(), platform.user_id.clone()),
                    user.name.clone(),
                );
            }
        }
        Self { mappings }
    }
}

#[async_trait]
impl IdentityResolver for PlatformIdentityResolver {
    async fn resolve(
        &self,
        channel_type: ChannelType,
        platform_user_id: &str,
        _platform_chat_id: Option<&str>,
    ) -> Result<UserId, IOError> {
        let key = (channel_type.to_string(), platform_user_id.to_string());

        match self.mappings.get(&key) {
            Some(user_name) => {
                debug!(
                    channel = %channel_type,
                    platform_user_id,
                    resolved = %user_name,
                    "identity resolved via platform mapping"
                );
                Ok(UserId(user_name.clone()))
            }
            None => Err(IOError::IdentityResolutionFailed {
                message: format!("unknown platform identity: {channel_type}:{platform_user_id}"),
            }),
        }
    }
}

