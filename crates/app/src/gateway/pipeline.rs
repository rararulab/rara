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

//! Update pipeline — wires [`super::detector::UpdateDetector`] state changes to
//! [`UpdateExecutor`] and [`SupervisorHandle`] for automatic updates.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

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

    // Track the last successfully applied revision so we skip stale
    // detector states that were published *during* a build.
    let mut last_applied_rev: Option<String> = None;

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

        // Skip if we already applied this revision. The detector may have
        // published a stale `update_available` state mid-build (before the
        // local HEAD was updated), causing a duplicate trigger.
        if last_applied_rev.as_deref() == Some(upstream_rev.as_str()) {
            info!(rev = %upstream_rev, "Update pipeline: already applied this revision, skipping");
            continue;
        }

        // Prevent concurrent updates.
        if UPDATING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            info!("Update pipeline: another update is already in progress, skipping");
            continue;
        }

        info!(rev = %upstream_rev, "Auto-update: starting update to {}", upstream_rev);
        notifier.update_started(&upstream_rev).await;

        let result = execute_and_handle(&upstream_rev, &supervisor_handle, &notifier).await;

        match result {
            Ok(()) => {
                last_applied_rev = Some(upstream_rev);
            }
            Err(ref e) => {
                warn!(error = %e, "Auto-update: executor creation failed");
                notifier.executor_creation_failed(e).await;
            }
        }

        UPDATING.store(false, Ordering::Relaxed);
    }
}

/// Create an executor, run the update, and handle the result.
async fn execute_and_handle(
    upstream_rev: &str,
    supervisor_handle: &SupervisorHandle,
    notifier: &UpdateNotifier,
) -> Result<(), String> {
    let mut executor = UpdateExecutor::new()
        .await
        .map_err(|e| format!("failed to create UpdateExecutor: {e}"))?;

    info!("Auto-update: building new version...");
    notifier.build_in_progress().await;

    let build_start = std::time::Instant::now();
    let result = executor
        .execute_update(upstream_rev)
        .await
        .map_err(|e| format!("execute_update error: {e}"))?;
    let build_duration = build_start.elapsed();

    match result {
        UpdateResult::Success { new_rev } => {
            info!(rev = %new_rev, ?build_duration, "Auto-update: successfully updated to {}, restarting agent", new_rev);
            notifier.update_success(&new_rev, build_duration).await;
            if let Err(e) = supervisor_handle.restart().await {
                warn!(error = %e, "Auto-update: failed to send restart command");
                notifier.restart_failed(&e.to_string()).await;
            }
            if let Err(e) = executor.cleanup().await {
                warn!(error = %e, "Auto-update: cleanup failed (non-fatal)");
            }
        }
        UpdateResult::BuildFailed { reason } => {
            warn!(reason = %reason, ?build_duration, "Auto-update: build failed for {}: {}", upstream_rev, reason);
            notifier
                .build_failed(upstream_rev, &reason, build_duration)
                .await;
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
        }
    }

    Ok(())
}
