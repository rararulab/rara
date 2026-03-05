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

//! Channel adapter trait.
//!
//! Each communication platform (Telegram, Web, CLI, ...) implements
//! [`ChannelAdapter`] to bridge platform-specific message handling into
//! the kernel's unified [`ChannelMessage`](super::types::ChannelMessage)
//! model.

use async_trait::async_trait;

use super::types::{AgentPhase, ChannelType};
use crate::{
    error::KernelError,
    handle::KernelHandle,
    io::{EgressError, Endpoint, PlatformOutbound},
};

/// A pluggable adapter for a single communication channel.
///
/// Unifies both ingress (listening for inbound messages) and egress
/// (delivering outbound messages) for a platform.
///
/// # Lifecycle
///
/// 1. **start** — The adapter begins listening for inbound messages (long
///    polling, WebSocket, etc.) and pushes them to the kernel via
///    [`KernelHandle::ingest`].
/// 2. **stop** — Graceful shutdown.
///
/// # Optional hooks
///
/// [`start`](Self::start), [`stop`](Self::stop),
/// [`typing_indicator`](Self::typing_indicator), and
/// [`set_phase`](Self::set_phase) have default no-op implementations.
/// Egress-only adapters (e.g. CLI) only need to implement
/// [`channel_type`](Self::channel_type) and [`send`](Self::send).
#[async_trait]
pub trait ChannelAdapter: Send + Sync + 'static {
    /// Which channel type this adapter serves.
    fn channel_type(&self) -> ChannelType;

    /// Deliver an outbound message to a specific endpoint.
    async fn send(&self, endpoint: &Endpoint, msg: PlatformOutbound) -> Result<(), EgressError>;

    /// Start the adapter with a [`KernelHandle`] for dispatching inbound
    /// messages into the kernel.
    ///
    /// No-op by default; egress-only adapters don't need to override this.
    async fn start(&self, _handle: KernelHandle) -> Result<(), KernelError> { Ok(()) }

    /// Gracefully stop the adapter.
    ///
    /// No-op by default.
    async fn stop(&self) -> Result<(), KernelError> { Ok(()) }

    /// Show a typing indicator in the given session.
    ///
    /// No-op by default; override for platforms that support it.
    async fn typing_indicator(&self, _session_key: &str) -> Result<(), KernelError> { Ok(()) }

    /// Signal an agent phase change for UX feedback (e.g. emoji reactions).
    ///
    /// No-op by default; override for platforms that support reactions.
    async fn set_phase(&self, _session_key: &str, _phase: AgentPhase) -> Result<(), KernelError> {
        Ok(())
    }
}

/// Shared reference to a [`ChannelAdapter`] implementation.
pub type ChannelAdapterRef = std::sync::Arc<dyn ChannelAdapter>;
