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

//! [`UpdateDetector`] — periodically checks upstream `origin/main` for new commits.

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
    pub current_rev:     String,
    /// Latest upstream `origin/main` revision (after last successful fetch).
    pub upstream_rev:    Option<String>,
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
    state:  UpdateState,
    tx:     watch::Sender<UpdateState>,
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
            upstream_rev:     None,
            last_check_time:  None,
            update_available: false,
        };

        let (tx, rx) = watch::channel(state.clone());

        (Self { config, state, tx }, rx)
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
        // 1. git fetch origin main
        if let Err(e) = Self::git_fetch().await {
            warn!(error = %e, "git fetch failed — will retry next cycle");
            return;
        }

        // 2. Refresh local HEAD (may have changed if an update was applied).
        match Self::git_rev_parse("HEAD").await {
            Ok(rev) => self.state.current_rev = rev,
            Err(e) => {
                warn!(error = %e, "git rev-parse HEAD failed");
                return;
            }
        }

        // 3. Get upstream rev.
        match Self::git_rev_parse("origin/main").await {
            Ok(rev) => self.state.upstream_rev = Some(rev),
            Err(e) => {
                warn!(error = %e, "git rev-parse origin/main failed");
                return;
            }
        }

        self.state.last_check_time = Some(chrono::Utc::now());

        let upstream = self.state.upstream_rev.as_deref().unwrap_or_default();

        // Only trigger when remote is strictly ahead of local.
        // A simple != would also fire when local is ahead (e.g. local commits
        // not yet pushed), which is not an updateable situation.
        self.state.update_available = if upstream == self.state.current_rev {
            false
        } else {
            Self::is_ancestor(&self.state.current_rev, upstream).await
        };

        if self.state.update_available {
            info!(
                current = %self.state.current_rev,
                upstream = %upstream,
                "Update available (remote ahead)"
            );
        } else if upstream != self.state.current_rev {
            info!(
                current = %self.state.current_rev,
                upstream = %upstream,
                "Revisions differ but remote is not ahead — skipping"
            );
        } else {
            info!(rev = %self.state.current_rev, "Already up to date");
        }

        // Publish to watchers (ignore error — means no active receivers).
        let _ = self.tx.send(self.state.clone());
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
