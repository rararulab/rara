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

//! Update pipeline — wires [`UpdateDetector`] state changes to
//! [`UpdateExecutor`] and [`SupervisorHandle`] for automatic updates.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde::Serialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{
    detector::UpdateState,
    executor::{UpdateExecutor, UpdateResult},
    notifier::UpdateNotifier,
    supervisor::SupervisorHandle,
};
use crate::GatewayConfig;

/// Guard that prevents concurrent update executions.
static UPDATING: AtomicBool = AtomicBool::new(false);

struct UpdateExecutionGuard;

impl UpdateExecutionGuard {
    fn acquire() -> Option<Self> {
        UPDATING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        Some(Self)
    }
}

impl Drop for UpdateExecutionGuard {
    fn drop(&mut self) {
        UPDATING.store(false, Ordering::Relaxed);
    }
}

/// Structured outcome for a manual or automatic gateway update attempt.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateExecutionSummary {
    pub ok: bool,
    pub status: String,
    pub detail: String,
    pub target_rev: Option<String>,
    pub active_rev: Option<String>,
    pub rolled_back: Option<bool>,
}

/// Run the update pipeline loop.
///
/// Watches for [`UpdateState`] changes from the detector. When an update is
/// available and `config.auto_update` is enabled, it creates an
/// [`UpdateExecutor`], builds the new revision, and restarts the agent via
/// the [`SupervisorHandle`].
pub async fn run_update_pipeline(
    config: GatewayConfig,
    mut update_rx: watch::Receiver<UpdateState>,
    supervisor_handle: SupervisorHandle,
    cancel: CancellationToken,
    notifier: Arc<UpdateNotifier>,
) {
    info!(
        "Update pipeline started (auto_update={})",
        config.auto_update
    );

    loop {
        tokio::select! {
            result = update_rx.changed() => {
                if result.is_err() {
                    // Sender dropped — detector is gone.
                    info!("Update pipeline: detector channel closed, exiting");
                    return;
                }
            }
            () = cancel.cancelled() => {
                info!("Update pipeline shutting down");
                return;
            }
        }

        let state = update_rx.borrow_and_update().clone();

        if !state.update_available || !config.auto_update {
            continue;
        }

        let upstream_rev = match state.upstream_rev {
            Some(ref rev) => rev.clone(),
            None => continue,
        };

        let Some(_guard) = UpdateExecutionGuard::acquire() else {
            info!("Update pipeline: another update is already in progress, skipping");
            continue;
        };

        info!(rev = %upstream_rev, "Auto-update: starting update to {}", upstream_rev);
        notifier.update_started(&upstream_rev).await;

        let result = execute_and_handle(&upstream_rev, &supervisor_handle, &notifier).await;

        if let Err(ref e) = result {
            warn!(error = %e, "Auto-update: executor creation failed");
            notifier.executor_creation_failed(e).await;
        }
    }
}

/// Execute an update immediately if no other update is already in flight.
pub async fn trigger_update(
    upstream_rev: &str,
    supervisor_handle: &SupervisorHandle,
    notifier: &UpdateNotifier,
) -> Result<UpdateExecutionSummary, String> {
    let Some(_guard) = UpdateExecutionGuard::acquire() else {
        return Ok(UpdateExecutionSummary {
            ok: false,
            status: "busy".to_owned(),
            detail: "another update is already in progress".to_owned(),
            target_rev: Some(upstream_rev.to_owned()),
            active_rev: None,
            rolled_back: None,
        });
    };

    info!(rev = %upstream_rev, "Manual update: starting update to {}", upstream_rev);
    notifier.update_started(upstream_rev).await;
    execute_and_handle(upstream_rev, supervisor_handle, notifier).await
}

/// Create an executor, run the update, and handle the result.
async fn execute_and_handle(
    upstream_rev: &str,
    supervisor_handle: &SupervisorHandle,
    notifier: &UpdateNotifier,
) -> Result<UpdateExecutionSummary, String> {
    let mut executor = UpdateExecutor::new()
        .await
        .map_err(|e| format!("failed to create UpdateExecutor: {e}"))?;

    info!("Auto-update: building new version...");
    notifier.build_in_progress().await;

    let result = executor
        .execute_update(upstream_rev)
        .await
        .map_err(|e| format!("execute_update error: {e}"))?;

    match result {
        UpdateResult::Success { new_rev } => {
            info!(rev = %new_rev, "Auto-update: successfully updated to {}, restarting agent", new_rev);
            notifier.update_success(&new_rev).await;
            if let Err(e) = supervisor_handle.restart().await {
                warn!(error = %e, "Auto-update: failed to send restart command");
                notifier.restart_failed(&e.to_string()).await;
            }
            if let Err(e) = executor.cleanup().await {
                warn!(error = %e, "Auto-update: cleanup failed (non-fatal)");
            }
            Ok(UpdateExecutionSummary {
                ok: true,
                status: "updated".to_owned(),
                detail: "update built successfully and restart requested".to_owned(),
                target_rev: Some(upstream_rev.to_owned()),
                active_rev: Some(new_rev),
                rolled_back: None,
            })
        }
        UpdateResult::BuildFailed { reason } => {
            warn!(reason = %reason, "Auto-update: build failed for {}: {}", upstream_rev, reason);
            notifier.build_failed(upstream_rev, &reason).await;
            Ok(UpdateExecutionSummary {
                ok: false,
                status: "build_failed".to_owned(),
                detail: reason,
                target_rev: Some(upstream_rev.to_owned()),
                active_rev: None,
                rolled_back: None,
            })
        }
        UpdateResult::ActivationFailed {
            reason,
            rolled_back,
        } => {
            warn!(
                reason = %reason,
                rolled_back,
                "Auto-update: activation failed, rolled back to previous version"
            );
            notifier.activation_failed(&reason, rolled_back).await;
            if !rolled_back {
                if let Err(e) = executor.rollback().await {
                    warn!(error = %e, "Auto-update: manual rollback also failed");
                }
            }
            if let Err(e) = supervisor_handle.restart().await {
                warn!(error = %e, "Auto-update: failed to send restart command after rollback");
                notifier.restart_failed(&e.to_string()).await;
            }
            Ok(UpdateExecutionSummary {
                ok: false,
                status: "activation_failed".to_owned(),
                detail: reason,
                target_rev: Some(upstream_rev.to_owned()),
                active_rev: None,
                rolled_back: Some(rolled_back),
            })
        }
    }
}
