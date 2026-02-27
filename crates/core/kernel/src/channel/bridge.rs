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

//! Channel bridge — the kernel-side handle that adapters call.
//!
//! When a [`ChannelAdapter`](super::adapter::ChannelAdapter) receives an
//! inbound message, it converts it to a [`ChannelMessage`] and calls
//! [`ChannelBridge::dispatch`]. The bridge is responsible for routing,
//! policy enforcement, and agent invocation.

use async_trait::async_trait;

use super::types::ChannelMessage;
use crate::error::KernelError;

/// Kernel-side bridge that channel adapters interact with.
///
/// Implementations handle routing, rate limiting, RBAC, and agent dispatch.
/// The bridge is the single entry point for all inbound messages regardless
/// of channel.
#[async_trait]
pub trait ChannelBridge: Send + Sync {
    /// Dispatch an inbound message for routing and agent processing.
    ///
    /// The bridge will:
    /// 1. Apply channel policies (rate limiting, authorization)
    /// 2. Route to the appropriate agent via [`ChannelRouter`](super::router::ChannelRouter)
    /// 3. Execute the agent and return the response text
    ///
    /// Returns the agent's response text on success.
    async fn dispatch(&self, message: ChannelMessage) -> Result<String, KernelError>;
}
