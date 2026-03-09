use std::time::Duration;

use tokio::process::{Child, Command};
use tracing::{info, warn};

use crate::client::RalphClient;
use crate::error::{RalphSnafu, Result};

/// Default port for the ralph RPC API server.
const RALPH_API_PORT: u16 = 13781;

/// Default command for starting ralph.
const RALPH_COMMAND: &str = "ralph";

/// How long to wait between health check retries during startup.
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Maximum time to wait for ralph API to become healthy on startup.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Delay before restarting a crashed ralph-api process.
const RESTART_DELAY: Duration = Duration::from_secs(3);

/// Manages the ralph RPC API child process lifecycle.
///
/// Spawns `ralph web` as a subprocess, monitors health via HTTP,
/// and restarts automatically on crash.
pub struct RalphSupervisor {
    child: Option<Child>,
    port: u16,
    workspace_root: String,
    client: RalphClient,
}

impl RalphSupervisor {
    /// Create a new supervisor. Does not start the process yet.
    ///
    /// `workspace_root` is the directory ralph uses for task/loop storage.
    #[must_use]
    pub fn new(workspace_root: &str) -> Self {
        let port = RALPH_API_PORT;
        let client = RalphClient::new(&format!("http://127.0.0.1:{port}"));
        Self {
            child: None,
            port,
            workspace_root: workspace_root.to_owned(),
            client,
        }
    }

    /// Return a clone of the ralph client for use by other components.
    #[must_use]
    pub fn client(&self) -> RalphClient {
        self.client.clone()
    }

    /// Start the ralph-api process and wait until it's healthy.
    pub async fn start(&mut self) -> Result<()> {
        info!(port = self.port, "starting ralph-api");

        let child = Command::new(RALPH_COMMAND)
            .arg("web")
            .arg("--no-open")
            .arg("--backend-port")
            .arg(self.port.to_string())
            .arg("--workspace")
            .arg(&self.workspace_root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                RalphSnafu {
                    message: format!("failed to spawn ralph-api: {e}"),
                }
                .build()
            })?;

        self.child = Some(child);

        // Wait for health check to pass.
        let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
        loop {
            if tokio::time::Instant::now() > deadline {
                // Kill the process if it never became healthy.
                self.stop().await.ok();
                return Err(RalphSnafu {
                    message: format!(
                        "ralph-api did not become healthy within {}s",
                        STARTUP_TIMEOUT.as_secs()
                    ),
                }
                .build());
            }

            if self.client.health().await {
                info!(port = self.port, "ralph-api is healthy");
                return Ok(());
            }

            // Check if process exited early.
            if let Some(ref mut child) = self.child {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(RalphSnafu {
                        message: format!("ralph-api exited during startup with {status}"),
                    }
                    .build());
                }
            }

            tokio::time::sleep(STARTUP_POLL_INTERVAL).await;
        }
    }

    /// Check health and restart if the process has died.
    pub async fn ensure_alive(&mut self) -> Result<()> {
        // Fast path: health check passes.
        if self.client.health().await {
            return Ok(());
        }

        // Process might have crashed — check.
        let needs_restart = match &mut self.child {
            Some(child) => matches!(child.try_wait(), Ok(Some(_))),
            None => true,
        };

        if needs_restart {
            warn!("ralph-api is not healthy, restarting after delay");
            tokio::time::sleep(RESTART_DELAY).await;
            self.start().await?;
        }

        Ok(())
    }

    /// Gracefully stop the ralph-api process.
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            info!("stopping ralph-api");
            // Try graceful kill first.
            if let Err(e) = child.kill().await {
                warn!(error = %e, "failed to kill ralph-api (may have already exited)");
            }
            child.wait().await.ok();
        }
        Ok(())
    }
}

impl Drop for RalphSupervisor {
    fn drop(&mut self) {
        // Best-effort sync kill on drop.
        if let Some(ref mut child) = self.child {
            child.start_kill().ok();
        }
    }
}
