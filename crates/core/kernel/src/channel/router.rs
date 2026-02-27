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

//! Channel router — maps inbound messages to agent names.
//!
//! The router decides which agent should handle a given
//! [`ChannelMessage`](super::types::ChannelMessage) based on channel
//! bindings, user preferences, and a system-level default.

use async_trait::async_trait;

use super::types::ChannelMessage;
use crate::error::KernelError;

/// Outcome of a routing decision.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    /// Name of the agent to handle this message.
    pub agent_name: String,
}

/// Routes inbound messages to the appropriate agent.
///
/// Implementations can use any strategy: static bindings, user preferences,
/// ML-based classification, or a simple default.
///
/// # Resolution priority (recommended)
///
/// 1. **Explicit binding** — channel + session → agent name
/// 2. **User preference** — user-configured default agent
/// 3. **System default** — fallback when nothing else matches
#[async_trait]
pub trait ChannelRouter: Send + Sync {
    /// Determine which agent should handle the given message.
    async fn route(&self, message: &ChannelMessage) -> Result<RouteDecision, KernelError>;
}

/// A trivial router that always routes to a single named agent.
///
/// Useful for simple deployments or as a fallback.
pub struct DefaultRouter {
    agent_name: String,
}

impl DefaultRouter {
    /// Create a router that always returns the given agent name.
    pub fn new(agent_name: impl Into<String>) -> Self {
        Self {
            agent_name: agent_name.into(),
        }
    }
}

#[async_trait]
impl ChannelRouter for DefaultRouter {
    async fn route(&self, _message: &ChannelMessage) -> Result<RouteDecision, KernelError> {
        Ok(RouteDecision {
            agent_name: self.agent_name.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_router_returns_configured_name() {
        let router = DefaultRouter::new("chat");
        let msg = crate::channel::types::ChannelMessage {
            id:           "test-1".to_owned(),
            channel_type: crate::channel::types::ChannelType::Web,
            user:         crate::channel::types::ChannelUser {
                platform_id:  "u1".to_owned(),
                display_name: None,
            },
            session_key:  "user:alice".to_owned(),
            role:         crate::channel::types::MessageRole::User,
            content:      crate::channel::types::MessageContent::Text("hello".to_owned()),
            tool_call_id: None,
            tool_name:    None,
            timestamp:    jiff::Timestamp::now(),
            metadata:     Default::default(),
        };
        let decision = router.route(&msg).await.unwrap();
        assert_eq!(decision.agent_name, "chat");
    }
}
