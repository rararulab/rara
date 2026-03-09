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

//! [`UpdateDetector`] — periodically checks upstream `origin/main` for new
//! commits.

use tokio::sync::watch;
use tracing::{info, warn};

use crate::GatewayConfig;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Snapshot of the current update-detection state.
#[derive(Debug, Clone)]
pub struct UpdateState {
    /// Local HEAD revision.
    pub current_rev: String,
    /// Latest upstream `origin/main` revision (after last successful fetch).
    pub upstream_rev: Option<String>,
    /// Timestamp of the last successful check.
    pub last_check_time: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether the upstream has commits not yet applied locally.
    pub update_available: bool,
}

// ---------------------------------------------------------------------------
// UpdateDetector
// ---------------------------------------------------------------------------

/// Periodically fetches `origin/main` and compares revisions.
///
/// Publishes [`UpdateState`] changes via a `tokio::sync::watch` channel so
/// that downstream consumers (e.g. `UpdateExecutor` in #94) can react.
pub struct UpdateDetector {
    config: GatewayConfig,
    state: UpdateState,
    tx: watch::Sender<UpdateState>,
}

impl UpdateDetector {
    /// Create a new detector.
    ///
    /// Returns the detector **and** a watch receiver that will receive
    /// [`UpdateState`] updates every check cycle.
    pub async fn new(config: GatewayConfig) -> (Self, watch::Receiver<UpdateState>) {
        let current_rev = Self::git_rev_parse("HEAD").await.unwrap_or_default();

        let state = UpdateState {
            current_rev,
            upstream_rev: None,
            last_check_time: None,
            update_available: false,
        };

        let (tx, rx) = watch::channel(state.clone());

        (Self { config, state, tx }, rx)
    }

    /// Clone the sender used to publish fresh update state snapshots.
    pub fn sender(&self) -> watch::Sender<UpdateState> {
        self.tx.clone()
    }

    /// Run the detection loop until the provided cancellation token is
    /// cancelled.
    pub async fn run(mut self, cancel: tokio_util::sync::CancellationToken) {
        let interval = self.config.check_interval;
        info!(
            ?interval,
            current_rev = %self.state.current_rev,
            "Update detector started"
        );

        loop {
            tokio::select! {
                () = tokio::time::sleep(interval) => {}
                () = cancel.cancelled() => {
                    info!("Update detector shutting down");
                    return;
                }
            }

            self.check_once().await;
        }
    }

    /// Execute a single fetch-and-compare cycle.
    async fn check_once(&mut self) {
        match Self::probe().await {
            Ok(state) => {
                self.state = state;
                let _ = self.tx.send(self.state.clone());
            }
            Err(e) => warn!(error = %e, "git update probe failed — will retry next cycle"),
        }
    }

    /// Execute a single fetch-and-compare cycle and return the fresh state.
    pub async fn probe() -> Result<UpdateState, String> {
        // 1. git fetch origin main
        Self::git_fetch().await?;

        let current_rev = Self::git_rev_parse("HEAD").await?;
        let upstream_rev = Self::git_rev_parse("origin/main").await?;
        let last_check_time = Some(chrono::Utc::now());

        let update_available = if upstream_rev == current_rev {
            false
        } else {
            Self::is_ancestor(&current_rev, &upstream_rev).await
        };

        // Only trigger when remote is strictly ahead of local.
        // A simple != would also fire when local is ahead (e.g. local commits
        // not yet pushed), which is not an updateable situation.
        if update_available {
            info!(
                current = %current_rev,
                upstream = %upstream_rev,
                "Update available (remote ahead)"
            );
        } else if upstream_rev != current_rev {
            info!(
                current = %current_rev,
                upstream = %upstream_rev,
                "Revisions differ but remote is not ahead — skipping"
            );
        } else {
            info!(rev = %current_rev, "Already up to date");
        }

        Ok(UpdateState {
            current_rev,
            upstream_rev: Some(upstream_rev),
            last_check_time,
            update_available,
        })
    }

    // -- git helpers --------------------------------------------------------

    async fn git_fetch() -> Result<(), String> {
        let output = tokio::process::Command::new("git")
            .args(["fetch", "origin", "main"])
            .output()
            .await
            .map_err(|e| format!("spawn git fetch: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git fetch exited {}: {stderr}", output.status));
        }
        Ok(())
    }

    /// Returns `true` if `ancestor` is an ancestor of `descendant`, i.e.
    /// `descendant` is strictly ahead of `ancestor`.
    async fn is_ancestor(ancestor: &str, descendant: &str) -> bool {
        let output = tokio::process::Command::new("git")
            .args(["merge-base", "--is-ancestor", ancestor, descendant])
            .output()
            .await;

        match output {
            Ok(o) => o.status.success(),
            Err(e) => {
                warn!(error = %e, "git merge-base --is-ancestor failed, assuming not ancestor");
                false
            }
        }
    }

    async fn git_rev_parse(refspec: &str) -> Result<String, String> {
        let output = tokio::process::Command::new("git")
            .args(["rev-parse", refspec])
            .output()
            .await
            .map_err(|e| format!("spawn git rev-parse: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "git rev-parse {refspec} exited {}: {stderr}",
                output.status
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}
