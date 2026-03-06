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
//! On each heartbeat tick, creates (or reuses) Mita's dedicated session
//! and submits a synthetic message to trigger the Mita agent loop.

use rara_kernel::{
    handle::KernelHandle,
    identity::Principal,
    memory::TapeService,
};
use tracing::{error, info};

/// Fixed tape name for Mita's own session.
const MITA_TAPE: &str = "mita";

/// Worker that runs the Mita heartbeat.
///
/// Each heartbeat:
/// 1. Ensures Mita's tape has a bootstrap anchor.
/// 2. Spawns a Mita agent session via `KernelHandle::spawn_with_input`.
/// 3. The agent loop runs with Mita's tools (list_sessions, read_tape,
///    dispatch_rara).
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
        info!(worker = ctx.name(), "Mita heartbeat worker started");
    }

    async fn work<S: Clone + Send + Sync>(&mut self, ctx: common_worker::WorkerContext<S>) {
        info!(worker = ctx.name(), "Mita heartbeat triggered");

        // Ensure Mita's tape exists with a bootstrap anchor.
        if let Err(e) = self.tape_service.ensure_bootstrap_anchor(MITA_TAPE).await {
            error!(error = %e, "failed to bootstrap Mita tape");
            return;
        }

        // Resolve agent manifest for Mita.
        let manifest = match self.kernel_handle.agent_registry().get("mita") {
            Some(m) => m,
            None => {
                error!("Mita agent manifest not found in registry");
                return;
            }
        };

        // Provide a lookup principal — `spawn_with_input` will resolve it
        // through `SecuritySubsystem::resolve_principal()` before storing
        // it in the session, so this is just a query key.
        let principal = Principal::lookup("system");

        // Spawn a new agent session for this heartbeat cycle.
        let input = "Heartbeat triggered. Analyze active sessions and determine if any proactive \
                      actions are needed. Review your previous tape entries to avoid repeating \
                      recent actions."
            .to_string();

        match self
            .kernel_handle
            .spawn_with_input(manifest, input, principal, None)
            .await
        {
            Ok(session_key) => {
                info!(
                    session_key = %session_key,
                    "Mita heartbeat session spawned"
                );
            }
            Err(e) => {
                error!(error = %e, "failed to spawn Mita heartbeat session");
            }
        }
    }

}
