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
use crate::{error::KernelError, handle::KernelHandle};

/// A pluggable adapter for a single communication channel.
///
/// # Lifecycle
///
/// 1. **start** — The adapter begins listening for inbound messages (long
///    polling, WebSocket, etc.) and pushes them to the kernel via
///    [`KernelHandle::ingest`].
/// 2. **stop** — Graceful shutdown.
///
/// Outbound delivery is handled by
/// [`EgressAdapter`](crate::io::egress::EgressAdapter), not by this trait.
///
/// # Optional UX hooks
///
/// [`typing_indicator`](Self::typing_indicator) and
/// [`set_phase`](Self::set_phase) have default no-op implementations. Adapters
/// that support typing indicators or emoji reactions can override them.
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Which channel type this adapter serves.
    fn channel_type(&self) -> ChannelType;

    /// Start the adapter with a [`KernelHandle`] for dispatching inbound
    /// messages into the kernel.
    async fn start(&self, handle: KernelHandle) -> Result<(), KernelError>;

    /// Gracefully stop the adapter.
    async fn stop(&self) -> Result<(), KernelError>;

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
