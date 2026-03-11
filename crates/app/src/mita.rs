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

//! Mita heartbeat worker — drives the background proactive agent.
//!
//! Mita spawns once at startup with a deterministic [`SessionKey`] and
//! receives heartbeat messages on the existing session via `submit_message`.
//! The session stays in Ready state between heartbeats, and its tape
//! accumulates naturally since tape name = session_key.

use rara_kernel::{
    handle::KernelHandle,
    identity::{Principal, UserId},
    io::InboundMessage,
    memory::TapeService,
    session::SessionKey,
};
use tracing::{error, info, warn};

/// Deterministic session key for Mita — derived from the fixed name "mita".
fn mita_session_key() -> SessionKey {
    SessionKey::deterministic("mita")
}

/// Worker that runs the Mita heartbeat.
///
/// On startup, spawns a long-lived Mita session with a deterministic key.
/// Each heartbeat delivers a synthetic message to that existing session
/// via `submit_message`, so the session and its tape persist across ticks.
pub struct MitaHeartbeatWorker {
    kernel_handle: KernelHandle,
    tape_service:  TapeService,
}

impl MitaHeartbeatWorker {
    pub fn new(kernel_handle: KernelHandle, tape_service: TapeService) -> Self {
        Self {
            kernel_handle,
            tape_service,
        }
    }
}

#[async_trait::async_trait]
impl common_worker::Worker for MitaHeartbeatWorker {
    async fn on_start<S: Clone + Send + Sync>(&mut self, ctx: common_worker::WorkerContext<S>) {
        info!(worker = ctx.name(), "Mita heartbeat worker starting");

        // Ensure Mita's tape exists with a bootstrap anchor.
        if let Err(e) = self.tape_service.ensure_bootstrap_anchor("mita").await {
            error!(error = %e, "failed to bootstrap Mita tape");
            return;
        }

        // Spawn Mita as a long-lived session with a fixed session key.
        let manifest = match self.kernel_handle.agent_registry().get("mita") {
            Some(m) => m,
            None => {
                error!("Mita agent manifest not found in registry");
                return;
            }
        };

        let principal = Principal::lookup("system");
        let session_key = mita_session_key();

        match self
            .kernel_handle
            .spawn_with_input(
                manifest,
                "Mita session initialized. Awaiting heartbeat instructions.".to_string(),
                principal,
                None,
                Some(session_key),
            )
            .await
        {
            Ok(key) => {
                info!(session_key = %key, "Mita long-lived session spawned");
            }
            Err(e) => {
                error!(error = %e, "failed to spawn Mita session");
            }
        }
    }

    async fn work<S: Clone + Send + Sync>(&mut self, ctx: common_worker::WorkerContext<S>) {
        info!(worker = ctx.name(), "Mita heartbeat triggered");

        let session_key = mita_session_key();

        // Check that Mita's session is still alive in the process table.
        if !self.kernel_handle.process_table().contains(&session_key) {
            warn!("Mita session not found in process table, skipping heartbeat");
            return;
        }

        // Deliver heartbeat message to the existing Mita session.
        let msg = InboundMessage::synthetic(
            "Heartbeat triggered. Analyze active sessions and determine if any proactive \
             actions are needed. Review your previous tape entries to avoid repeating \
             recent actions."
                .to_string(),
            UserId("system".to_string()),
            session_key,
        );

        if let Err(e) = self.kernel_handle.submit_message(msg) {
            error!(error = %e, "failed to deliver heartbeat to Mita session");
        }
    }
}
